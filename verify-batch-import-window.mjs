import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, lstatSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { dirname, join, relative, resolve } from 'node:path'
import { setTimeout as wait } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

assert.ok(process.argv[2], 'Pass the isolated evidence directory')
const evidence = resolve(process.argv[2])
const fixture = JSON.parse(readFileSync(join(evidence, 'fixture.json'), 'utf8'))
const workspace = join(evidence, 'workspace')
const managementFolders = fixture.owner === 'windows-isolated-management-folders'
assert.ok(managementFolders || fixture.owner === 'windows-isolated-batch-import')
assert.equal(fixture.workspace.toLowerCase(), workspace.toLowerCase())
assert.equal(fixture.source.toLowerCase(), join(workspace, 'instances').toLowerCase())
const checkedBinaries = [
  [fixture.cli, fixture.cliSha256], [fixture.gui, fixture.guiSha256],
]
for (const [path, expected] of checkedBinaries) {
  assert.equal(sha(readFileSync(path)), expected.toLowerCase(), 'Candidate binary changed')
}
const checks = []
const results = {}
const children = []
const env = {
  ...process.env,
  IH_WORKSPACE: workspace,
  LOCALAPPDATA: fixture.appData,
  WEBVIEW2_USER_DATA_FOLDER: join(evidence, 'webview'),
}
const descriptor = join(workspace, 'resident-manager.json')

function sha(content) {
  return createHash('sha256').update(content).digest('hex')
}

function captureTree(root) {
  const entries = []
  function walk(directory) {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name)
      const stats = lstatSync(path, { bigint: true })
      assert.ok(!stats.isSymbolicLink(), 'An isolated fixture unexpectedly contains a link')
      const localPath = relative(root, path)
      if (stats.isDirectory()) {
        entries.push({ path: localPath, directory: true })
        walk(path)
      } else {
        assert.ok(stats.isFile(), 'An isolated fixture unexpectedly contains a special file')
        // Windows lock files can deny a read beyond EOF even when their contents are empty.
        let digest
        try { digest = sha(stats.size === 0n ? Buffer.alloc(0) : readFileSync(path)) }
        catch (error) { throw new Error('Could not snapshot ' + localPath + ': ' + error.message) }
        entries.push({ path: localPath, sha256: digest, modifiedNs: stats.mtimeNs.toString() })
      }
    }
  }
  walk(root)
  return entries
}

function stableTree(root) {
  return JSON.stringify(captureTree(root))
}

function verifySameTree(before, after, section) {
  results[section] = { beforeSha256: sha(before), afterSha256: sha(after) }
  if (before !== after) {
    writeFileSync(join(evidence, section + '-before.json'), before)
    writeFileSync(join(evidence, section + '-after.json'), after)
  }
  assert.equal(results[section].beforeSha256, results[section].afterSha256, section + ' did not preserve files')
  checks.push(section)
}

function ih(...args) {
  return JSON.parse(execFileSync(fixture.cli, [workspace, ...args], { windowsHide: true, encoding: 'utf8', env }))
}

async function until(check, message, milliseconds = 18000) {
  const end = Date.now() + milliseconds
  do {
    const hit = await check()
    if (hit) return hit
    await wait(120)
  } while (Date.now() < end)
  throw new Error(message)
}

function managerIdentity() {
  if (!existsSync(descriptor)) return undefined
  const raw = readFileSync(descriptor, 'utf8')
  const info = JSON.parse(raw)
  const created = /"created"\s*:\s*(\d+)/.exec(raw)?.[1]
  assert.ok(Number.isSafeInteger(info.pid) && info.pid > 0 && created, 'Invalid isolated Manager identity')
  return { pid: info.pid, created }
}

function managerProcess(owned, waitForExit = false) {
  if (!owned) return false
  const command = [
    '$p = Get-Process -Id ' + owned.pid + ' -ErrorAction SilentlyContinue',
    'if ($p -and $p.StartTime.ToFileTimeUtc() -eq ' + owned.created + ') { ' +
      (waitForExit ? '$p.WaitForExit(8000)' : '$true') + ' } else { ' + (waitForExit ? '$true' : '$false') + ' }',
  ].join('\n')
  const reply = execFileSync('pwsh', ['-NoProfile', '-Command', command], { windowsHide: true, encoding: 'utf8' })
  return reply.trim().toLowerCase() === 'true'
}

async function stopManager(owned) {
  if (!owned) return
  const current = managerIdentity()
  if (!current) return
  assert.deepEqual(current, owned, 'Isolated Manager changed before exit')
  if (!managerProcess(owned)) return
  assert.equal(ih('manager-exit').exiting, true)
  assert.ok(managerProcess(owned, true), 'The isolated Manager process did not exit')
}

async function openWindow() {
  const probe = createServer()
  await new Promise((accept, reject) => {
    probe.on('error', reject)
    probe.listen(0, '127.0.0.1', accept)
  })
  const { port } = probe.address()
  await new Promise((accept) => probe.close(accept))
  const child = spawn(fixture.gui, [], {
    windowsHide: true,
    stdio: 'ignore',
    env: { ...env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=' + port + ' --remote-debugging-address=127.0.0.1' },
  })
  const owned = { child }
  children.push(owned)
  const target = await until(async () => {
    if (child.exitCode !== null) throw new Error('The isolated native GUI exited before opening')
    try {
      const endpoint = 'http://127.0.0.1:' + port + '/json/list'
      const pages = await (await fetch(endpoint, { signal: AbortSignal.timeout(600) })).json()
      return pages.find((page) => page.type === 'page' && page.url.includes('tauri.localhost'))
    } catch { return undefined }
  }, 'Native WebView did not open')
  const socket = new WebSocket(target.webSocketDebuggerUrl)
  owned.socket = socket
  await new Promise((accept, reject) => {
    socket.addEventListener('open', accept, { once: true })
    socket.addEventListener('error', reject, { once: true })
  })
  let sequence = 0
  function send(method, params = {}) {
    const id = ++sequence
    return new Promise((accept, reject) => {
      const timer = setTimeout(() => {
        socket.removeEventListener('message', onMessage)
        reject(new Error('Native WebView response timed out for ' + method))
      }, 5000)
      function onMessage(event) {
        const response = JSON.parse(event.data)
        if (response.id !== id) return
        clearTimeout(timer)
        socket.removeEventListener('message', onMessage)
        if (response.error) reject(new Error(response.error.message))
        else accept(response.result)
      }
      socket.addEventListener('message', onMessage)
      socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async function evaluate(expression, ...args) {
    const message = '(' + expression.toString() + ')(...' + JSON.stringify(args) + ')'
    const value = await send('Runtime.evaluate', { expression: message, awaitPromise: true, returnByValue: true })
    if (value.exceptionDetails) throw new Error(value.exceptionDetails.text)
    return value.result.value
  }
  async function screenshot(name) {
    const frame = await send('Page.captureScreenshot')
    writeFileSync(join(evidence, name + '.png'), Buffer.from(frame.data, 'base64'))
  }
  return { child, evaluate, screenshot, close: () => closeWindow(owned) }
}

async function closeWindow({ child, socket }) {
  socket?.close()
  if (child.exitCode !== null) return
  const command = 'param($pidValue,$path)\n$p=Get-Process -Id $pidValue -ErrorAction SilentlyContinue\n'
    + 'if($p -and $p.Path -ieq $path){$null=$p.CloseMainWindow()}'
  for (let attempt = 0; attempt < 3 && child.exitCode === null; attempt++) {
    execFileSync('pwsh', ['-NoProfile', '-CommandWithArgs', command, String(child.pid), fixture.gui], { encoding: 'utf8', windowsHide: true })
    await wait(350)
  }
  await until(() => child.exitCode !== null, 'The isolated native GUI did not close')
}

async function click(gui, selector) {
  await until(() => gui.evaluate((selector) => {
    const control = document.querySelector(selector)
    if (!control || control.disabled || control.getClientRects().length === 0) return false
    control.click()
    return true
  }, selector), 'Control unavailable: ' + selector)
}

function assertStoppedInstances(expected) {
  const actual = ih('list')
  assert.deepEqual(actual.map(({name, port, state, run}) => ({name, port, state, run})).sort((a, b) => a.name.localeCompare(b.name)), expected)
}

function verifyReceipt(workId, receipt, expected) {
  assert.equal(receipt.workId, workId)
  assert.equal(receipt.state, 'Complete')
  assert.deepEqual(receipt.rows.map(({name, state}) => ({name, state})).sort((a, b) => a.name.localeCompare(b.name)),
    expected.map(({name}) => ({name, state: 'Imported'})))
}

const dialog = 'dialog.batch-import-dialog[open]'
const rows = dialog + ' .batch-import-preview .batch-import-row'
const expected = fixture.expected.map(({name, port}) => ({ name, port, state: 'Stopped', run: null }))
  .sort((a, b) => a.name.localeCompare(b.name))
const sourceBefore = stableTree(fixture.source)
const originalPolicy = readFileSync(join(workspace, 'scan-policy.json'))
assert.equal(sha(originalPolicy), fixture.policySha256.toLowerCase())

async function setField(gui, selector, value) {
  const applied = await gui.evaluate((selector, value) => {
    const field = document.querySelector(selector)
    if (!field || field.readOnly || field.disabled) return false
    field.value = value
    field.dispatchEvent(new Event('input', { bubbles: true }))
    return true
  }, selector, value)
  assert.ok(applied, 'Field unavailable: ' + selector)
}

async function scan(gui) {
  await click(gui, '.batch-import-trigger')
  await setField(gui, dialog + ' [name=batch-import-directory]', fixture.scanDirectory)
  await setField(gui, dialog + ' [name=batch-import-java]', fixture.java)
  await click(gui, dialog + ' button[form=batch-import-scan]')
  const candidates = await until(async () => {
    const items = await gui.evaluate((rows) => [...document.querySelectorAll(rows)].map((row) => {
      const checkbox = row.querySelector('input[type=checkbox]')
      return {
        name: row.querySelector('strong')?.textContent,
        directory: row.dataset.directory,
        core: row.querySelector('.batch-import-core')?.textContent,
        port: row.querySelector('.batch-import-port')?.textContent,
        checked: checkbox?.checked, disabled: checkbox?.disabled,
      }
    }), rows)
    return items.length >= expected.length ? items : undefined
  }, 'Native preview did not show the isolated candidates')
  assert.deepEqual(candidates.map((item) => item.name).sort(), expected.map((item) => item.name))
  for (const row of candidates) {
    const registered = fixture.expected.find((entry) => entry.name === row.name)
    assert.match(row.directory.replaceAll('\\', '/'), new RegExp('/' + row.name + '$'))
    assert.match(row.core, new RegExp(registered.kind, 'i'))
    assert.ok(row.port.includes(String(registered.port)))
    assert.equal(row.checked, false)
    assert.equal(row.disabled, false)
  }
  assert.ok(await gui.evaluate((selector) => document.querySelector(selector)?.disabled, dialog + ' .batch-import-confirm'))
  return candidates
}

async function displayedReceipt(gui) {
  return until(async () => {
    const result = await gui.evaluate((dialog) => {
      const node = document.querySelector(dialog + ' .batch-import-result')
      return {
        id: node?.querySelector('bdi')?.textContent?.trim() ?? '',
        rows: [...(node?.querySelectorAll('.batch-import-receipt-row strong') ?? [])].map((strong) => strong.textContent),
      }
    }, dialog)
    return result.rows.length >= expected.length ? result : undefined
  }, 'Native batch import receipt did not show both rows')
}

function nativePicker(gui, initial, selection) {
  const helper = fileURLToPath(new URL('./verify-management-folder-picker.ps1', import.meta.url))
  const args = ['-NoProfile', '-File', helper, '-GuiPid', String(gui.child.pid), '-Executable', fixture.gui, '-InitialFolder', initial]
  if (selection) args.push('-SelectFolder', selection)
  return JSON.parse(execFileSync('pwsh', args, { windowsHide: true, encoding: 'utf8', timeout: 22000 }))
}

async function verifyManagementFolders(gui) {
  const trigger = '.management-folder-trigger'
  const modal = 'dialog.management-folder-dialog[open]'
  const rowSelector = modal + ' .management-folder-row'
  const allNames = [...expected.map(({ name }) => name), 'batch-ignored'].sort()
  const before = stableTree(workspace)
  const bin = dirname(fixture.gui)
  await click(gui, trigger)
  results.emptyPicker = nativePicker(gui, bin)
  await until(() => gui.evaluate((selector) => !document.querySelector(selector), modal), 'Cancel unexpectedly opened a preview')
  verifySameTree(before, stableTree(workspace), 'picker-cancel-zero-writes')

  async function openPreview() {
    await click(gui, trigger)
    const picker = nativePicker(gui, bin, fixture.scanPath)
    const candidateRows = await until(() => gui.evaluate((selector, size) => {
      const rows = [...document.querySelectorAll(selector)]
      return rows.length === size && rows.map((row) => ({
        name: row.querySelector('strong')?.textContent?.trim(),
        checked: row.querySelector('input')?.checked,
      }))
    }, rowSelector, allNames.length), 'Management folder preview did not list the expected servers')
    assert.deepEqual(candidateRows.map(({ name }) => name).sort(), allNames)
    assert.ok(candidateRows.every(({ checked }) => checked))
    assert.equal(await gui.evaluate((selector) => !!document.querySelector(selector + ' input:not([type=checkbox])'), modal), false)
    return picker
  }

  results.initialPicker = await openPreview()
  await gui.screenshot('management-folder-preview')
  verifySameTree(before, stableTree(workspace), 'native-preview-zero-writes')
  await click(gui, modal + ' .dialog-actions button:first-child')
  await until(() => gui.evaluate((selector) => !document.querySelector(selector), modal), 'Could not cancel management preview')
  verifySameTree(before, stableTree(workspace), 'preview-cancel-zero-writes')

  await openPreview()
  await click(gui, rowSelector + '[data-directory$="batch-ignored"] input')
  await click(gui, modal + ' .management-folder-save')
  const registered = await until(() => gui.evaluate((selector, size) => {
    const rows = [...document.querySelectorAll(selector)]
    return rows.length === size && rows.map((row) => row.querySelector('strong')?.textContent?.trim()).sort()
  }, modal + ' .management-folder-receipt-row', expected.length), 'Management folder receipt did not show both selected servers')
  assert.deepEqual(registered, expected.map(({ name }) => name))
  const policy = ih('scan-policy')
  assert.equal(policy.policy.allowPaths.length, 1)
  assert.equal(policy.policy.excludePaths.length, 1)
  assert.ok(policy.policy.excludePaths[0].endsWith('/batch-ignored'))
  assertStoppedInstances(expected)
  verifySameTree(sourceBefore, stableTree(fixture.source), 'management-save-source-zero-writes')
  await gui.screenshot('management-folder-receipt')
  await click(gui, modal + ' .dialog-actions button:first-child')

  async function clickText(text) {
    await until(() => gui.evaluate((text) => {
      const button = [...document.querySelectorAll('button')].find((item) => item.textContent.trim() === text && !item.disabled && item.getClientRects().length)
      if (!button) return false
      button.click()
      return true
    }, text), 'Button is unavailable: ' + text)
  }
  await clickText('應用程式設定')
  await clickText('管理資料夾')
  const allow = '[name="management-allow-path"]'
  const exclude = '[name="management-exclude-path"]'
  await until(() => gui.evaluate((selector) => !!document.querySelector(selector), exclude), 'The saved blacklist row did not appear')
  await click(gui, '.path-rule:has(' + allow + ') button:first-of-type')
  results.allowedPicker = nativePicker(gui, dirname(fixture.scanPath))
  await click(gui, '.path-rule:has(' + exclude + ') button:first-of-type')
  results.excludedPicker = nativePicker(gui, dirname(fixture.excluded))
  await click(gui, '#management-allow-add')
  await click(gui, '.path-rules:has(' + allow + ') .path-rule:last-of-type button:first-of-type')
  results.emptySettingsPicker = nativePicker(gui, bin)
  await click(gui, '.path-rule:has(' + allow + '):last-of-type .remove-path')
  const values = await gui.evaluate((allow, exclude) => ({
    allow: [...document.querySelectorAll(allow)].map((input) => input.value),
    exclude: [...document.querySelectorAll(exclude)].map((input) => input.value),
  }), allow, exclude)
  assert.equal(values.allow.length, 1)
  assert.equal(values.exclude.length, 1)
  assert.ok(values.allow[0].replaceAll('\\', '/').endsWith('/BatchFixture'))
  assert.ok(values.exclude[0].replaceAll('\\', '/').endsWith('/batch-ignored'))
  checks.push('per-row native picker opened each parent or the executable directory and cancel preserved each draft')
  results.registration = { rows: registered.map((name) => ({ name, state: 'Imported' })) }
  await gui.screenshot('management-folder-settings')
}

let manager
let gui
let error = null
let cleanupError = null
let result = 'FAIL'
try {
  assert.equal(managerProcess(managerIdentity()), false, 'Isolated Manager was already present before the test')
  if (managementFolders) {
    const existing = execFileSync('pwsh', ['-NoProfile', '-Command', '(Get-Process -Name InstanceHarbor,instanceharbor-v2,instanceharbor-acceptance -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -NE 0 | Measure-Object).Count'], { windowsHide: true, encoding: 'utf8' })
    assert.equal(existing.trim(), '0', 'Another InstanceHarbor GUI is open. It was left untouched')
  }
  ih('manager-connect')
  manager = await until(managerIdentity, 'Isolated Manager did not start')
  const firstManager = manager
  gui = await openWindow()
  if (managementFolders) {
    await verifyManagementFolders(gui)
    result = 'PASS'
  } else {
  const firstGui = gui.child.pid
  await until(() => gui.evaluate(() => !document.querySelector('.batch-import-trigger')?.disabled
    && !!document.querySelector('.batch-import-trigger')), 'Native instance page did not become interactive')
  assertStoppedInstances([])
  const initial = stableTree(workspace)
  const cliPreview = ih('import-batch-preview', JSON.stringify({directory: fixture.scanDirectory, java: fixture.java}))
  assert.equal(cliPreview.complete, true)
  assert.deepEqual(cliPreview.rows.map(({name, state}) => ({name, state})).sort((a, b) => a.name.localeCompare(b.name)),
    expected.map(({name}) => ({name, state: 'Ready'})))
  verifySameTree(initial, stableTree(workspace), 'cli-preview-zero-writes')

  const candidates = await scan(gui)
  await gui.screenshot('batch-import-preview')
  verifySameTree(initial, stableTree(workspace), 'gui-preview-zero-writes')
  await click(gui, rows + ':first-child input[type=checkbox]')
  await click(gui, dialog + ' .dialog-actions button:first-child')
  await until(() => gui.evaluate((dialog) => !document.querySelector(dialog), dialog), 'Native preview did not close')
  verifySameTree(initial, stableTree(workspace), 'gui-cancel-zero-writes')
  assertStoppedInstances([])

  await scan(gui)
  for (let index = 0; index < candidates.length; index++) {
    await click(gui, rows + ':nth-child(' + (index + 1) + ') input[type=checkbox]')
  }
  await click(gui, dialog + ' .batch-import-confirm')
  const visible = await displayedReceipt(gui)
  const {id: workId} = visible
  assert.match(workId, /^[A-Za-z0-9_-]{1,64}$/)
  assert.deepEqual(visible.rows.sort(), expected.map(({name}) => name))
  const receipt = ih('import-batch-status', workId)
  verifyReceipt(workId, receipt, expected)
  assertStoppedInstances(expected)
  verifySameTree(sourceBefore, stableTree(fixture.source), 'gui-import-source-zero-writes')
  assert.ok(readFileSync(join(workspace, 'scan-policy.json')).equals(originalPolicy))
  await gui.screenshot('batch-import-receipt')
  await gui.close()
  await stopManager(manager)
  manager = undefined

  ih('manager-connect')
  manager = await until(managerIdentity, 'Replacement isolated Manager did not start')
  assert.notDeepEqual(manager, firstManager)
  assertStoppedInstances(expected)
  assert.deepEqual(ih('import-batch-status', workId), receipt)
  gui = await openWindow()
  await until(() => gui.evaluate(() => document.querySelectorAll('tr[data-instance]').length === 2), 'Reopened native instance list did not show both rows')
  await click(gui, '.batch-import-trigger')
  await click(gui, dialog + ' .batch-import-history summary')
  await setField(gui, dialog + ' [name=batch-import-work-id]', workId)
  await click(gui, dialog + ' .batch-import-lookup button')
  const recovered = await displayedReceipt(gui)
  assert.equal(recovered.id, workId)
  assert.deepEqual(recovered.rows.sort(), expected.map(({name}) => name))
  assertStoppedInstances(expected)
  verifySameTree(sourceBefore, stableTree(fixture.source), 'reopened-source-zero-writes')
  assert.ok(readFileSync(join(workspace, 'scan-policy.json')).equals(originalPolicy))
  await gui.screenshot('batch-import-reopened-receipt')
  Object.assign(results, { workId, receipt, firstGui, reopenedGui: gui.child.pid, firstManager, reopenedManager: manager })
  checks.push('both instances stayed stopped', 'same work ID and rows across GUI and Manager restart')
  result = 'PASS'
  }
} catch (failure) {
  error = failure instanceof Error ? failure.message : String(failure)
  if (gui) {
    try { await gui.screenshot('native-failure') } catch { /* Preserve the primary failure. */ }
  }
} finally {
  try {
    for (const owned of children) await closeWindow(owned)
  } catch (failure) {
    result = 'FAIL'
    cleanupError = failure instanceof Error ? failure.message : String(failure)
  }
  try {
    await stopManager(manager)
  } catch (failure) {
    result = 'FAIL'
    cleanupError = [cleanupError, failure instanceof Error ? failure.message : String(failure)].filter(Boolean).join('. ')
  }
  writeFileSync(join(evidence, 'native-result.json'), JSON.stringify({ result, checks, ...results, error, cleanupError }, null, 2))
}
if (result !== 'PASS') throw new Error([error, cleanupError].filter(Boolean).join('. '))
console.log(managementFolders ? 'Native management folder picker, preview, receipt and per-row navigation passed.'
  : 'Native GUI and CLI returned the same persisted batch import receipt.')
