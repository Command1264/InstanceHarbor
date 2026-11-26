import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { endsTab } from './console-session.js'

test('a tab closes once the Host reports that its run ended or was replaced', () => {
  assert.equal(endsTab('ConsoleRunEnded'), true)
  assert.equal(endsTab('ConsoleRunChanged'), true)
  assert.equal(endsTab(new Error('ConsoleRunEnded')), false, 'Tauri rejects with the bare code')
  for (const error of ['HostUnavailable', 'ConsoleTabClosed', 'Timeout', undefined]) {
    assert.equal(endsTab(error), false, String(error))
  }
})

test('the read loop closes the tab through terminal_close when the run ends', () => {
  const source = readFileSync(new URL('./native-console.js', import.meta.url), 'utf8')
  const reader = source.split('async function readSession(')[1]?.split('\nasync function ')[0] ?? ''
  assert.match(reader, /if \(endsTab\(error\)\) \{\s*if \(await closeSession\(session\)\) return/)
  const close = source.split('async function closeSession(')[1]?.split('\nfunction ')[0] ?? ''
  // The Rust side decides whether this was the last tab and then ends the viewer.
  assert.match(close, /invoke\('terminal_close', \{ run: session\.run, version: session\.version \}\)/)
})
