import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { aboutCommitText, productTitle } from './about.ts'
import { componentsMessages as messages } from './messages/components.ts'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

test('commit text shows the label and explains dirty and unknown builds', () => {
  for (const language of [0, 1]) {
    const t = (id, values = {}) => messages[id][language].replace(/\{(\w+)\}/g, (_, name) => values[name])
    assert.equal(aboutCommitText('33dfd35', t), '33dfd35')
    assert.equal(aboutCommitText('33dfd35-dirty', t), t('components.about.commitDirty', { commit: '33dfd35-dirty' }))
    assert.equal(aboutCommitText('unknown', t), messages['components.about.commitUnknown'][language])
  }
  const english = (id, values = {}) => messages[id][1].replace(/\{(\w+)\}/g, (_, name) => values[name])
  assert.equal(aboutCommitText('33dfd35-dirty', english), '33dfd35-dirty (with uncommitted changes)')
})

test('the About panel opens only the link names the backend accepts', () => {
  const panel = read('./components/AboutPanel.vue')
  const fromPanel = [...panel.matchAll(/@click="open\('([a-z]+)'\)"/g)].map(match => match[1])
  assert.deepEqual(fromPanel.sort(), ['issues', 'license', 'notices', 'repository'])
  assert.doesNotMatch(panel, /https?:\/\//, 'the page must not hold or send URLs')
  // "Core maintenance" opens the BuildTools guide for Spigot (#75) by name too.
  const core = read('./components/CoreMaintenanceDialog.vue')
  const fromCore = [...core.matchAll(/invoke\('open_about_link', \{ link: '([a-z]+)' \}\)/g)].map(match => match[1])
  assert.deepEqual(fromCore, ['buildtools'])
  assert.doesNotMatch(core, /https?:\/\//, 'the dialog must not hold or send URLs')
  const opened = [...fromPanel, ...fromCore].sort()
  const backend = read('../src-tauri/src/about.rs')
  const accepted = [...backend.matchAll(/^\s+"([a-z]+)" => Some\(Self::/gm)].map(match => match[1]).sort()
  assert.deepEqual(accepted, opened)
})

test('build-product.ps1 passes the commit through the variable build.rs reads', () => {
  const variable = read('../src-tauri/src/build_commit.rs').match(/pub const VARIABLE: &str = "([A-Z_]+)";/)[1]
  const script = read('../build-product.ps1')
  assert.ok(script.includes(`$env:${variable}=$commit`), 'build-product.ps1 sets the commit variable')
  assert.match(script, /--version/, 'the build checks the commit ihctl reports')
})

test('ihctl --version keeps the prefix launch-development.ps1 matches', () => {
  assert.match(read('../launch-development.ps1'), /-notlike 'InstanceHarbor Dev CLI \*'/)
  assert.match(read('../src-tauri/src/about.rs'), /format!\("\{title\} CLI \{VERSION\} \(\{COMMIT\}\)"\)/)
})

test('the About tab has no save or restore buttons', () => {
  const page = read('./SettingsPage.vue')
  const footer = page.match(/<div[^>]*class="settings-footer"[^>]*>/)
  assert.ok(footer, 'SettingsPage must render the settings footer')
  assert.match(footer[0], /v-if="tab!=='about'"/)
})

test('the product title names the development workspace like the backend does', () => {
  assert.equal(productTitle({ channel: 'InstanceHarbor', workspace: null }), 'InstanceHarbor')
  assert.equal(productTitle({ channel: 'InstanceHarbor Dev', workspace: 'accept-52b' }), 'InstanceHarbor Dev — accept-52b')
  assert.equal(productTitle(undefined), 'InstanceHarbor')
  const backend = read('../src-tauri/src/desktop_channel.rs')
  assert.ok(backend.includes(String.raw`format!("{title} \u{2014} {name}")`),'desktop_channel.rs uses the same separator')
})

test('the About panel shows the development workspace and the rail names the channel', () => {
  const panel = read('./components/AboutPanel.vue')
  assert.match(panel, /info\?\.workspace/)
  assert.match(panel, /components\.about\.workspace/)
  const app = read('./App.vue')
  assert.doesNotMatch(app, /<span>InstanceHarbor<\/span>/, 'the rail must not hard-code the stable name')
  assert.match(app, /productTitle\(/)
})

test('a narrow rail wraps the channel name between words and puts the workspace on its own line', () => {
  const app = read('./App.vue')
  assert.match(app, /<span v-if="channelInfo\?\.workspace" class="brand-workspace">/)
  const css = read('./app.css')
  // Breaking anywhere split "InstanceHarbor" itself at the 160px rail.
  assert.doesNotMatch(css, /\.brand span\{[^}]*overflow-wrap:anywhere/)
  assert.doesNotMatch(css, /\.brand-name\{[^}]*(overflow-wrap|word-break)/)
  assert.match(css, /\.brand-workspace\{[^}]*display:block[^}]*overflow-wrap:anywhere/)
})
