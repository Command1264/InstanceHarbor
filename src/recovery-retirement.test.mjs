import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

// Stage 1 of #96. verify-recovery-pin.ps1 checks the same rules against real processes.
test('a new start never writes a recovery pin', () => {
  const windows = read('../src-tauri/src/windows.rs')
  const plan = windows.match(/struct StartPlan \{[^}]*\}/)[0]
  assert.doesNotMatch(plan, /recovery/, 'the start plan carries no pin or recovery-key lock')
  assert.doesNotMatch(windows, /save\([^;]*PIN_FILE/, 'nothing saves recovery-pin.json')
  assert.doesNotMatch(windows, /initial_host_manager/, 'a start no longer names a Manager to pin')
})

test('a Host still honours a pin left by an earlier build', () => {
  const windows = read('../src-tauri/src/windows.rs')
  assert.match(windows, /let pin: host_recovery::Pin = read\(&pin_path\)\?;/)
  assert.match(windows, /host_recovery::Gate::new\(/)
  const gate = read('../src-tauri/src/host_recovery.rs')
  for (const verb of ['Rebind', 'RecoverChallenge', 'RecoverProof']) {
    assert.match(gate, new RegExp(`\\["${verb}", run`), `${verb} is still accepted`)
  }
})

test('settings show the recovery key tab only when the workspace has a key', () => {
  const page = read('./SettingsPage.vue')
  assert.match(page, /<button v-if="recoveryKeyPresent" [^>]*switchTab\('recovery'\)/)
  assert.match(page, /recoveryKeyPresent\.value=!!\(await manage<\{key:unknown\}>\(\['recovery-key-status'\],'background'\)\)\.key/)
})

test('the recovery key panel says the feature is retiring', () => {
  const panel = read('./components/RecoveryKeyPanel.vue')
  assert.match(panel, /t\('recovery\.retiring'\)/)
  const zh = translate('zh-TW', 'recovery.retiring')
  const en = translate('en', 'recovery.retiring')
  assert.match(zh, /移除/)
  assert.match(en, /removed in a later version/)
})
