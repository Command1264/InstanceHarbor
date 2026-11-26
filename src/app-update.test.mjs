import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { updateAssetText, updateFailureText, updateResultText } from './app-update.ts'
import { componentsMessages as messages } from './messages/components.ts'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
const translator = language => (id, values = {}) => {
  assert.ok(messages[id], `missing message ${id}`)
  return messages[id][language].replace(/\{(\w+)\}/g, (_, name) => values[name])
}
const zh = translator(0)
const status = (patch = {}) => ({
  currentVersion: '1.1.0', channel: 'stable', autoCheck: true, platform: 'windows-x64', install: 'setup',
  lastAttempt: 100, lastSuccess: 100, lastError: null, due: false, update: null, ...patch,
})
const update = (patch = {}) => ({ version: '1.2.0', tag: 'v1.2.0', prerelease: false, publishedAt: null,
  asset: { name: 'InstanceHarbor-1.2.0-windows-x64-setup.exe', url: 'x', size: 1, digest: 'sha256:x' }, problem: null, ...patch })

test('the result names the new version and marks prereleases', () => {
  assert.equal(updateResultText(status({ update: update() }), zh), '有新版本 1.2.0')
  assert.equal(updateResultText(status({ update: update({ version: '1.2.0-rc.1', prerelease: true }) }), zh), '有新的預發行版 1.2.0-rc.1')
  assert.equal(updateResultText(status(), zh), '已是最新版本')
  assert.equal(updateResultText(status({ update: update() }), translator(1)), 'Version 1.2.0 is available')
})

test('a failed or missing check never claims the build is current', () => {
  assert.equal(updateResultText(undefined, zh), '--')
  assert.equal(updateResultText(status({ lastAttempt: null, lastSuccess: null }), zh), '--')
  assert.equal(updateResultText(status({ lastError: 'UpdateCheckOffline' }), zh), '--')
  // A known update stays shown after a later failure.
  assert.equal(updateResultText(status({ lastError: 'UpdateCheckOffline', update: update() }), zh), '有新版本 1.2.0')
})

test('a release without a file for this platform says so', () => {
  assert.equal(updateAssetText(status({ update: update({ asset: null, problem: 'NoMatchingAsset' }) }), zh), '此版本尚未提供適用於 windows-x64 的檔案')
  assert.match(updateAssetText(status({ update: update({ asset: null, problem: 'UntrustedAsset' }) }), zh), /SHA-256/)
  assert.equal(updateAssetText(status({ update: update() }), zh), '')
  assert.equal(updateAssetText(status(), zh), '')
})

test('every failure reason the backend reports has its own text', () => {
  const backend = read('../src-tauri/src/app_update.rs')
  const codes = new Set([...backend.matchAll(/"(UpdateCheck[A-Za-z]+)"/g)].map(match => match[1]))
  assert.ok(codes.size >= 8, [...codes].join())
  for (const code of codes) {
    for (const language of [0, 1]) {
      const text = updateFailureText(code, translator(language))
      assert.equal(text, messages[`components.update.reason.${code}`][language], code)
    }
  }
  assert.equal(updateFailureText('SomethingElse', zh), '檢查失敗')
})

test('the page opens a release by tag and holds no URL', () => {
  const panel = read('./components/AppUpdatePanel.vue')
  const store = read('./app-update-store.ts')
  assert.doesNotMatch(panel + store, /https?:\/\//)
  assert.match(store, /invoke\('open_update_release', \{ tag \}\)/)
  assert.match(read('../src-tauri/src/main.rs'), /fn open_update_release\(tag: String\)/)
})

test('automatic checks stay quiet; only a manual check shows progress', () => {
  const store = read('./app-update-store.ts')
  const startup = store.slice(store.indexOf('export async function startupUpdateCheck'), store.indexOf('export async function checkForUpdateNow'))
  assert.match(startup, /'app-update-check', 'auto'], 'background', true/)
  assert.doesNotMatch(startup, /updateChecking|updateFeedback/)
  assert.match(read('./App.vue'), /onMounted\(\(\)=>\{void startupUpdateCheck\(\)\}\)/)
})
