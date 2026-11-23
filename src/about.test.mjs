import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { aboutCommitText } from './about.ts'
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
  const opened = [...panel.matchAll(/@click="open\('([a-z]+)'\)"/g)].map(match => match[1]).sort()
  assert.deepEqual(opened, ['issues', 'license', 'notices', 'repository'])
  assert.doesNotMatch(panel, /https?:\/\//, 'the page must not hold or send URLs')
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
