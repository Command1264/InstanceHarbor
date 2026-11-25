import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { coreReasonId } from './core-display.ts'

test('every reason the backend gives for an unknown core is translated', () => {
  const source = readFileSync(new URL('../src-tauri/src/presentation.rs', import.meta.url), 'utf8')
  const body = source.slice(source.indexOf('fn current_core('), source.indexOf('/// `nativeConsole`'))
  const codes = [...new Set([...body.matchAll(/"reason":"(\w+)"/g)].map(match => match[1]))]
  assert.deepEqual(codes.sort(), ['CorePathUnverified', 'CoreProfileUnverified', 'CoreUnreadable'])
  for (const code of codes) {
    const id = coreReasonId(code)
    assert.notEqual(id, 'instances.coreReason.other', code)
    assert.match(translate('zh-TW', id), /\p{Script=Han}/u, code)
    assert.match(translate('en', id), /^[A-Z]/, code)
  }
})

test('an unknown core reason falls back to a general message and no reason shows nothing', () => {
  assert.equal(coreReasonId('SomethingNew'), 'instances.coreReason.other')
  assert.equal(coreReasonId(undefined), undefined)
  assert.equal(coreReasonId(''), undefined)
})

test('an offline reading of the core version is labelled briefly and explained once', () => {
  assert.equal(translate('zh-TW', 'components.coreMaintenance.offline'), '（離線辨識）')
  assert.equal(translate('en', 'components.coreMaintenance.offline'), ' (read offline)')
  // The explanation says what was not compared, not that a connection failed.
  assert.match(translate('zh-TW', 'instances.coreOffline'), /核心檔.*未與內建清單或官方來源比對/)
  assert.match(translate('en', 'instances.coreOffline'), /read from the core file.*built-in list or the official source/)
})
