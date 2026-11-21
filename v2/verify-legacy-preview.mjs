import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { dirname, join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const [raw, cliArg, hostArg, paperArg, guiArg, mode, theme = 'system'] = process.argv.slice(2)
assert.ok(raw && cliArg && hostArg && paperArg, 'Usage: node verify-legacy-preview.mjs .evidence/legacy-preview-ID ihctl.exe Host.exe Paper.jar [GUI.exe]')
assert.ok(['system','light','dark'].includes(theme), 'Invalid isolated GUI theme')
const sandbox = resolve(raw)
assert.equal(dirname(sandbox), resolve(import.meta.dirname, '.evidence'))
assert.match(sandbox, /[\\/]legacy-preview-[a-z0-9-]+$/i)
assert.ok(!existsSync(sandbox), 'Choose a new evidence directory')
const cli = resolve(cliArg), hostExe = resolve(hostArg)
const source = join(sandbox, 'source'), workspace = join(sandbox, 'workspace')
const policy = join(source, 'config', 'managed-scan.json')
const collection = join(source, 'instances', 'Human', 'Example')
const env = { ...process.env, IH_V2_WORKSPACE: workspace, LOCALAPPDATA: join(sandbox, 'appdata'),
  WEBVIEW2_USER_DATA_FOLDER: join(sandbox, 'webview') }
const file = (p, content) => { mkdirSync(dirname(p), {recursive:true}); writeFileSync(p, content) }
const save = (p, content) => file(p, JSON.stringify(content, null, 2))
const hash = b => createHash('sha256').update(b).digest('hex')
const names = ['stopped', 'owned', 'external', 'uncertain', 'malformed', 'denied', 'outside']
const namesForRows = names.slice(0, 5)
const before = {}, after = {}
function files(root) {
  const listing = {}
  function walk(dir, relative) {
    for (const child of readdirSync(dir, {withFileTypes:true})) {
      const path = join(dir, child.name), name = relative + child.name
      if (child.isSymbolicLink()) listing[name] = 'link'
      else if (child.isDirectory()) { listing[name + '/'] = 'dir'; walk(path, name + '/') }
      else if (child.name === '.testlab-manager.lock') {
        try { listing[name] = {sha256:hash(readFileSync(path)), writeTime:statSync(path).mtimeMs} }
        catch (e) {
          if (e.code !== 'EBUSY') throw e
          const stat = statSync(path)
          listing[name] = {locked:true, size:stat.size, writeTime:stat.mtimeMs}
        }
      } else listing[name] = { sha256:hash(readFileSync(path)), writeTime:statSync(path).mtimeMs }
    }
  }
  walk(root, '')
  return listing
}
function ih(...args) {
  return JSON.parse(execFileSync(cli, [workspace, ...args], {env, encoding:'utf8', windowsHide:true, timeout:16000}))
}
function inspect(host = hostExe, p = '') { return ih('legacy-preview', source, p, host) }
const byName = (result, name) => result.rows.find(row => row.name === name)
async function until(fn, message, ms = 12000) {
  const limit = Date.now() + ms
  do {
    const result = await fn()
    if (result) return result
    await delay(120)
  } while (Date.now() < limit)
  throw new Error(message)
}
async function listening() {
  const server = createServer()
  await new Promise((ok, no) => { server.once('error', no); server.listen(0, '127.0.0.1', ok) })
  return server
}
const close = server => new Promise(ok => server.close(ok))
const ports = Object.fromEntries(await Promise.all(names.map(async name => {
  const server = await listening()
  const port = server.address().port
  if (name !== 'external') await close(server)
  return [name, {port, server}]
})))
assert.equal(new Set(Object.values(ports).map(x => x.port)).size, names.length)
const paper = readFileSync(paperArg)
assert.equal(hash(paper), 'a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968')
let host, gui, socket
let preferenceLock
const pwsh = join(process.env.ProgramFiles, 'PowerShell','7','pwsh.exe')
const pathFor = name => join(collection, name)
const run = randomUUID().replaceAll('-', ''), launch = randomUUID().replaceAll('-', '')
const ticket = join(pathFor('owned'), 'testlab-console', 'launch-' + launch + '.json')
const snapshots = async () => {
  const owned = byName(inspect(), 'owned')
  return owned.observation.state === 'managed' ? owned : false
}

try {
  save(policy, {schemaVersion:1, allow:['Human', 'Human/Example/stopped'], deny:['Human/Example/denied']})
  const java = join(sandbox, 'fake', 'java.exe')
  file(java, 'test fixture: this Java path must never run')
  save(join(source, 'settings', 'java-runtimes.json'), {schemaVersion:1, paths:{jdk25:java}})
  for (const name of names.slice(0, -1)) {
    const target = pathFor(name), {port} = ports[name]
    save(join(target, 'workspace.json'), {schemaVersion:2, audience:'Human', project:'Example', instance:name, port, javaAlias:'jdk25', eulaAccepted:true})
    file(join(target,'server.properties'), 'server-ip=127.0.0.1\nonline-mode=false\nserver-port=' + port + '\nrcon.password=TEST_SECRET_SHOULD_NOT_APPEAR\n')
    file(join(target,'eula.txt'),'eula=true\n')
    file(join(target,'server.jar'),name === 'stopped' ? paper : 'Test placeholder. Do not start this fixture.')
  }
  file(join(pathFor('malformed'),'workspace.json'), '{invalid')
  const outside = join(sandbox,'outside-target')
  file(join(outside,'workspace.json'),'OUTSIDE_SECRET_SHOULD_NOT_APPEAR')
  symlinkSync(outside, pathFor('outside'), 'junction')
  save(join(source,'out-of-bounds.json'),{schemaVersion:1,allow:[outside],deny:[]})
  save(join(source,'unreadable.json'),{schemaVersion:1,allow:['Human/Example/outside'],deny:[]})
  save(join(source,'unsupported.json'),{schemaVersion:2,allow:['Human'],deny:[]})
  const pending = join(pathFor('uncertain'), 'testlab-console', 'launch-' + 'f'.repeat(32) + '.json')
  save(pending,{schemaVersion:1,state:'Preparing'})
  ih('init')
  const child = join(sandbox, 'child.cjs')
  file(child, "const fs=require('node:fs'); const s=require('node:net').createServer(); s.listen(+process.argv[2],'127.0.0.1',()=>console.log('Done (0.1s)!')); const end=Date.now()+Number(process.argv[3]); const timer=setInterval(()=>{ if(Date.now()>=end || fs.existsSync('fixture-exit')) { clearInterval(timer); s.close(); } },100);")
  host = spawn(hostExe, ['detached-host', pathFor('owned'), String(ports.owned.port), run, process.execPath,
    JSON.stringify([child, String(ports.owned.port), mode === '--human' ? '1200000' : '120000'])], {cwd:sandbox,windowsHide:true,stdio:['ignore','ignore','pipe']})
  assert.ok(host.pid)
  const hostErrors = []
  host.stderr.on('data', b => hostErrors.push(String(b)))
  const started = execFileSync(pwsh, ['-NoProfile','-Command',
    '(Get-Process -Id '+host.pid+').StartTime.ToUniversalTime().ToString("O")'], {encoding:'utf8',windowsHide:true}).trim()
  const ticketData = {schemaVersion:1,id:pathFor('owned'),launchId:launch,state:'Accepted',hostPid:host.pid,hostStartUtc:started,runId:run,mode:'Detached'}
  save(ticket,ticketData)
  await until(() => existsSync(join(pathFor('owned'),'testlab-console','host-' + run + '.log')), 'Legacy Host did not write startup log')
  const original = await until(snapshots, 'Could not verify actual legacy Host: ' + hostErrors.join(''))
  assert.equal(original.observation.hostPid,host.pid)
  assert.equal(original.observation.runId,run)
  assert.equal(original.observation.listening,true)
  before.source = files(source); before.workspace = files(workspace); before.outside = files(outside)
  const verified = inspect()
  assert.deepEqual(verified.rows.map(r => r.name).sort(), namesForRows.toSorted())
  assert.equal(verified.skippedLinks,1)
  const check = (name, state, reason) => {
    const row = byName(verified,name)
    assert.equal(row.observation.state,state,JSON.stringify(row))
    if(reason) assert.equal(row.observation.reason,reason)
    return row
  }
  const stopped = check('stopped','stopped','NoHostOrListener')
  assert.equal(stopped.core.identity.family,'Paper')
  assert.equal(stopped.java.fileVerified,true)
  check('owned','managed','VerifiedLegacyHost')
  assert.equal(check('external','external','IndependentListener').observation.pid,process.pid)
  check('uncertain','unknown','HostUnconfirmed')
  check('malformed','unknown','ConfigUnreadable')
  assert.equal(byName(inspect(''),'owned').observation.state,'unknown')
  assert.equal(byName(inspect(process.execPath),'owned').observation.state,'unknown')
  assert.throws(()=>inspect(hostExe,join(source,'out-of-bounds.json')), /LegacyPolicyOutsideSource/)
  assert.throws(()=>inspect(hostExe,join(source,'unreadable.json')), /LegacyPolicyUnreadable/)
  assert.throws(()=>inspect(hostExe,join(source,'unsupported.json')), /LegacyPolicyUnsupported/)
  assert.ok(!JSON.stringify(verified).includes('SECRET_SHOULD_NOT_APPEAR'))
  assert.deepEqual(inspect(),verified)
  after.source = files(source); after.workspace = files(workspace); after.outside = files(outside)
  assert.deepEqual(after,before,'Source, destination and link target must remain byte for byte unchanged')
  const stale = { ...ticketData, runId:'e'.repeat(32) }
  save(ticket,stale)
  assert.equal(byName(inspect(),'owned').observation.state,'unknown')
  save(ticket,{ ...ticketData, hostStartUtc:'2024-01-01T00:00:00.0000000Z' })
  assert.equal(byName(inspect(),'owned').observation.state,'unknown')
  save(ticket,ticketData)
  const collision = join(pathFor('uncertain'),'workspace.json'), originalMetadata = readFileSync(collision)
  save(collision,{...JSON.parse(originalMetadata),port:ports.stopped.port})
  file(join(pathFor('uncertain'),'server.properties'),
    'server-ip=127.0.0.1\nonline-mode=false\nserver-port='+ports.stopped.port+'\n')
  assert.equal(byName(inspect(),'stopped').observation.reason,'DuplicatePort')
  file(collision,originalMetadata)
  file(join(pathFor('uncertain'),'server.properties'),
    'server-ip=127.0.0.1\nonline-mode=false\nserver-port='+ports.uncertain.port+'\nrcon.password=TEST_SECRET_SHOULD_NOT_APPEAR\n')
  if (process.platform === 'win32') {
    // This exact, test-created junction is deliberately removed without traversal.
    const { rmdirSync } = await import('node:fs')
    rmdirSync(pathFor('outside'))
  } else unlinkSync(pathFor('outside'))
  const result = { hostPid:host.pid, childPid:original.observation.pid, run, verified,
    matched:['source','workspace','outside-target'], cases:['wrong executable','missing executable input','stale run','stale PID timestamp','policy out of bounds','unsupported policy','denied by policy','junction','malformed config','duplicate port'] }
  save(join(sandbox,'sha256-before.json'),before)
  save(join(sandbox,'sha256-after.json'),after)
  save(join(sandbox,'result.json'),result)
  console.log(JSON.stringify({sandbox, rows:verified.rows.map(r=>[r.name,r.observation.state]), compared:result.matched}))
  if (guiArg) {
    mkdirSync(env.LOCALAPPDATA)
    if (theme !== 'system') save(join(workspace,'preferences.json'),{theme,pins:[],pathSeparator:'forward'})
    // The standard home-page refresh creates its coordination lock on first
    // access. Establish that baseline before checking the preview itself.
    ih('list')
    const guiBefore = {source:files(source), workspace:files(workspace)}
    if (mode === '--human') {
      const liveBefore = {source:files(source), workspace:files(workspace)}
      console.log('\n在 InstanceHarbor 2.0 選擇「辨識 1.0 資料」。')
      console.log('1.0 資料根目錄：'+source)
      console.log('1.0 Host 執行檔路徑：'+hostExe)
      console.log('掃描 policy 留空。驗收完關閉這個 2.0 視窗，隔離子程序會隨即結束。\n')
      gui = spawn(resolve(guiArg), [], {env,cwd:sandbox,stdio:['ignore','ignore','inherit']})
      await new Promise((ok,no)=>{gui.once('exit',ok);gui.once('error',no)})
      assert.equal(gui.exitCode,0)
      assert.deepEqual(files(source),liveBefore.source)
      assert.deepEqual(files(workspace),liveBefore.workspace)
      save(join(sandbox,'human-session-data-check.json'),{sourceUnchanged:true,workspaceUnchanged:true})
    } else {
    const debug = await listening()
    const cdp = debug.address().port
    await close(debug)
    const failLog = ['--log-failure','--both-failures'].includes(mode)
    const failPreferences = ['--preferences-failure','--both-failures'].includes(mode)
    async function lockPreferences(label) {
      const target = join(workspace,'preferences.json')
      const ready = join(sandbox,label+'.ready')
      preferenceLock = spawn(pwsh,['-NoProfile','-Command',
        "$stream=[IO.File]::Open($env:IH_TEST_PREF,'Open','ReadWrite','None'); try {[IO.File]::WriteAllText($env:IH_TEST_READY,'ready'); [Console]::ReadLine() | Out-Null} finally {$stream.Dispose()}"],
        {env:{...env,IH_TEST_PREF:target,IH_TEST_READY:ready},windowsHide:true,stdio:['pipe','ignore','inherit']})
      await until(()=>existsSync(ready),'Could not obtain isolated file lock')
    }
    async function releasePreferences() {
      if (!preferenceLock) return
      preferenceLock.stdin.end('\n')
      await until(()=>preferenceLock.exitCode!==null,'Could not release isolated file lock')
      assert.equal(preferenceLock.exitCode,0)
      preferenceLock=undefined
    }
    if (failLog) file(join(env.LOCALAPPDATA,'InstanceHarbor','v2','logs'),'isolated blocker')
    if (failPreferences) await lockPreferences('startup')
    let diagnosticStderr = ''
    gui = spawn(resolve(guiArg), [], {env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+cdp},cwd:sandbox,stdio:['ignore','ignore','pipe']})
    gui.stderr.setEncoding('utf8').on('data',chunk=>{ diagnosticStderr += chunk })
    const page = await until(async () => {
      try { return (await (await fetch('http://127.0.0.1:'+cdp+'/json/list',{signal:AbortSignal.timeout(700)})).json()).find(p=>p.type==='page') }
      catch { return null }
    }, 'No isolated WebView2 target. Confirm a different 2.0 GUI does not already hold the singleton.', 25000)
    socket = new WebSocket(page.webSocketDebuggerUrl)
    await new Promise((ok,no)=>{socket.onopen=ok; socket.onerror=no})
    let id=0
    function cdpCall(method,params={}) {
      const seq=++id
      return new Promise((ok,no)=>{
        const listener=e=>{const msg=JSON.parse(e.data); if(msg.id!==seq)return
          clearTimeout(timer);socket.removeEventListener('message',listener);msg.error ? no(new Error(JSON.stringify(msg.error))) : ok(msg.result)}
        const timer=setTimeout(()=>{socket.removeEventListener('message',listener);no(new Error('WebView2: '+method))},5000)
        socket.addEventListener('message',listener);socket.send(JSON.stringify({id:seq,method,params}))
      })
    }
    const evalJs = async expression => {
      const response = await cdpCall('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true})
      assert.ok(!response.exceptionDetails,JSON.stringify(response.exceptionDetails))
      return response.result.value
    }
    await until(() => evalJs('[...document.querySelectorAll("button")].some(b=>b.textContent.trim()==="辨識 1.0 資料")'),'Vue app not ready')
    if (failPreferences) {
      await until(()=>evalJs('document.querySelector(".statusbar").textContent.includes("外觀設定尚未讀取")'),'Startup preference failure was not shown')
      await evalJs('[...document.querySelectorAll("nav button")].find(b=>b.textContent.trim()==="應用程式設定").click()')
      assert.equal(await evalJs('document.querySelector("#app-theme") === null'),true)
      assert.equal(await evalJs('document.querySelector(".settings-footer button.primary").disabled'),true)
      await evalJs('[...document.querySelectorAll("nav button")].find(b=>b.textContent.trim()==="實例").click()')
      if (failLog) {
        await until(()=>diagnosticStderr.includes('"osCode":32') && diagnosticStderr.includes('Diagnostic'),'Terminal fallback omitted both failure stages')
      }
      await releasePreferences()
    }
    if (theme !== 'system') await until(() => evalJs('document.documentElement.dataset.theme === '+JSON.stringify(theme)),'Saved theme was not applied')
    const diagnostics = await evalJs('window.__TAURI_INTERNALS__.invoke("diagnostic_status")')
    assert.equal(diagnostics.available,!failLog,JSON.stringify(diagnostics))
    if (failLog) {
      await until(()=>evalJs('document.querySelector("#failure-dialog")?.open || [...document.querySelectorAll("dialog[open]")].some(d=>d.textContent.includes("無法完整寫入"))'),'Expected diagnostic warning')
      await evalJs('[...document.querySelectorAll("dialog[open]")].find(d=>d.textContent.includes("無法完整寫入")).querySelector("button").click()')
    } else {
      assert.ok(existsSync(diagnostics.path),'The diagnostic log must exist in the isolated appdata directory')
    }
    if (theme === 'dark' && !failPreferences && !failLog) {
      await lockPreferences('refresh')
      const write = await evalJs('window.__TAURI_INTERNALS__.invoke("manage", {arguments:["save-preferences",JSON.stringify({theme:"light",pins:[],pathSeparator:"forward"})],origin:"manual"}).then(()=>"unexpected success", String)')
      assert.match(write,/FileAccess\[read\].*os error 32/)
      await evalJs('[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="重新核對").click()')
      await until(()=>evalJs('document.querySelector(".statusbar").textContent.includes("保留上次設定")'),'Manual refresh did not report the read failure')
      assert.equal(await evalJs('document.documentElement.dataset.theme'),'dark')
      await releasePreferences()
      await until(()=>evalJs('!document.querySelector(".statusbar").textContent.includes("保留上次設定")'),'Background retry did not recover')
      assert.equal(await evalJs('document.documentElement.dataset.theme'),'dark')
      assert.ok(readFileSync(diagnostics.path,'utf8').split('\n').filter(Boolean).map(JSON.parse).some(r=>r.event.action==='preferences' && r.event.error?.osCode===32))
    }
    await evalJs('[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="辨識 1.0 資料").click()')
    assert.equal(await evalJs('document.activeElement.name'),'legacy-data-root')
    await evalJs('document.querySelector(".legacy-preview details").open=true')
    for (const [key, value] of [['legacy-data-root',source],['legacy-host-executable',hostExe]]) {
      await evalJs('(()=>{const f=document.querySelector("[name='+key+']");f.value='+JSON.stringify(value)+';f.dispatchEvent(new Event("input",{bubbles:true}));})()')
    }
    await evalJs('document.querySelector("#legacy-preview-form").requestSubmit()')
    const expected = ['已停止','1.0 Host 正在管理','外部程序占用','狀態未確認']
    await until(async()=> (await evalJs('[...document.querySelectorAll(".legacy-row-title span")].map(x=>x.textContent)')).length===5, 'Vue result did not appear')
    const labels = await evalJs('[...document.querySelectorAll(".legacy-row-title span")].map(x=>x.textContent)')
    for (const label of expected) assert.ok(labels.includes(label),JSON.stringify(labels))
    const overflow = await evalJs('(()=>{const d=document.querySelector(".legacy-preview");return d.scrollWidth>d.clientWidth+1})()')
    assert.equal(overflow,false)
    const screenshot = async name => {
      const image = await cdpCall('Page.captureScreenshot',{format:'png'})
      file(join(sandbox,name+'.png'),Buffer.from(image.data,'base64'))
    }
    await screenshot('native-legacy-preview')
    await evalJs('(()=>{const f=document.querySelector("[name=legacy-scan-policy]");f.value='+JSON.stringify(join(source,'unsupported.json'))+';f.dispatchEvent(new Event("input",{bubbles:true}));document.querySelector("#legacy-preview-form").requestSubmit();})()')
    await until(() => evalJs('document.querySelector(".legacy-preview [role=alert]")?.textContent.includes("下方保留上次核對結果")'), 'Failed lookup must preserve earlier results')
    assert.equal(await evalJs('document.querySelectorAll(".legacy-row").length'),5)
    await cdpCall('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27})
    await cdpCall('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27})
    await until(async()=>!(await evalJs('document.querySelector(".legacy-preview").open')), 'Escape did not close preview')
    assert.equal(await evalJs('document.activeElement.textContent.trim()'),'辨識 1.0 資料')
    assert.deepEqual(files(source), guiBefore.source)
    assert.deepEqual(files(workspace), guiBefore.workspace)
    save(join(sandbox,'native-ui.json'),{labels,overflow,closeWithEscape:true,focusRestored:true,previousResultsPreserved:true,theme,diagnosticsAvailable:diagnostics.available,failLog,failPreferences,terminalFallbackVerified:failLog&&failPreferences})
    }
  }
} finally {
  socket?.close()
  const cleanupErrors = []
  if (preferenceLock?.exitCode === null) {
    preferenceLock.stdin.end('\n')
    try { await until(()=>preferenceLock.exitCode!==null,'Isolated preference lock did not exit') }
    catch(e) { cleanupErrors.push(e.message) }
  }
  if (host?.pid && host.exitCode === null) {
    file(join(pathFor('owned'),'fixture-exit'),'exit isolated child only')
    try { await until(()=>host.exitCode!==null,'Isolated Host did not exit') }
    catch(e) { cleanupErrors.push(e.message) }
  }
  if (gui?.pid && gui.exitCode === null) {
    try {
      execFileSync(pwsh,['-NoProfile','-Command','if ($p=Get-Process -Id '+gui.pid+' -ErrorAction SilentlyContinue) { $p.CloseMainWindow() }'],
        {encoding:'utf8',windowsHide:true})
      await until(()=>gui.exitCode!==null,'Isolated GUI did not exit')
    } catch(e) { cleanupErrors.push(e.message) }
  }
  await close(ports.external.server)
  if (existsSync(sandbox)) save(join(sandbox,'teardown.json'), {hostExit:host?.exitCode, guiExit:gui?.exitCode,externalListenerClosed:!ports.external.server.listening,cleanupErrors})
  assert.deepEqual(cleanupErrors,[])
}
