import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { consoleFallbackReason, consoleSummary } from './console-tier.ts'

// Reason codes the Host writes into console-tier.json (native_console.rs).
const hostReasons = ['ConsoleRuntimeMissing', 'ConsoleRuntimeMismatch', 'ConsoleSystemUnavailable',
  'ConsolePathUnsupported', 'ConsoleWindowNotConfirmed', 'ConsoleTierLimited']

test('every Host fallback reason has its own message', () => {
  const source = readFileSync(new URL('../src-tauri/src/native_console.rs', import.meta.url), 'utf8')
  for (const reason of hostReasons) {
    assert.ok(source.includes(`"${reason}"`), `${reason} is no longer written by the Host`)
    const text = consoleFallbackReason({ tier: 'system', skipped: [{ tier: 'bundled', reason }] })
    assert.ok(text && !text.includes(reason), `${reason} shows its code instead of a message`)
  }
})

test('the preferred console needs no explanation and fallbacks name their reason', () => {
  assert.equal(consoleSummary({ tier: 'bundled', skipped: [] }), 'InstanceHarbor 後台')
  const system = consoleSummary({ tier: 'system', skipped: [{ tier: 'bundled', reason: 'ConsoleRuntimeMissing' }] })
  assert.match(system, /Windows 內建主控台/)
  assert.match(system, /conpty\.dll/)
  const unknown = consoleSummary({ tier: 'system', skipped: [{ tier: 'bundled', reason: 'ConsoleCreateFailed 0x80070005' }] })
  assert.match(unknown, /ConsoleCreateFailed 0x80070005/)
})

test('every core gets a console; an older run record still explains itself', () => {
  const source = readFileSync(new URL('../src-tauri/src/native_console.rs', import.meta.url), 'utf8')
  assert.ok(!source.includes('"ConsoleCoreUnsupported"'), 'the Host no longer limits the console by core family')
  // console-tier.json written by an earlier build can still hold the old reason.
  const core = { tier: 'plain', skipped: ['bundled', 'system', 'window'].map(tier => ({ tier, reason: 'ConsoleCoreUnsupported' })) }
  assert.match(consoleSummary(core), /本次沒有後台.*重新啟動後會使用後台/)
})

test('a plain run explains why no console tier was used', () => {
  const declined = { tier: 'plain', skipped: [
    { tier: 'bundled', reason: 'ConsoleRuntimeMissing' },
    { tier: 'system', reason: 'ConsoleSystemUnavailable' },
    { tier: 'window', reason: 'ConsoleWindowNotConfirmed' },
  ] }
  assert.match(consoleSummary(declined), /沒有內建主控台元件/)
})
