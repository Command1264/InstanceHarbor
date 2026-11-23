import assert from 'node:assert/strict'
import { test } from 'node:test'
import { duplicateMessageIds, formatDisplayTime, messageParameterMismatches, renderConsoleTitle, resolveLanguage, translate, validateConsoleTitle } from './i18n.ts'

test('a missing or explicit saved language is independent of the OS language', () => {
  // A preference that is missing or not loaded yet follows the system.
  assert.equal(resolveLanguage(undefined, ['en-US']), 'en')
  assert.equal(resolveLanguage(undefined, ['zh-TW']), 'zh-TW')
  assert.equal(resolveLanguage(undefined, []), 'en')
  assert.equal(resolveLanguage('zh-TW', ['en-US']), 'zh-TW')
  assert.equal(resolveLanguage('en', ['zh-TW']), 'en')
})

test('system respects order and skips unsupported, malformed and explicitly simplified tags', () => {
  assert.equal(resolveLanguage('system', ['ja-JP', 'en-US', 'zh-HK']), 'en')
  assert.equal(resolveLanguage('system', ['not_a_language', 'zh-Hant-HK', 'en-GB']), 'zh-TW')
  assert.equal(resolveLanguage('system', ['zh-MO', 'en']), 'zh-TW')
  assert.equal(resolveLanguage('system', ['zh-Hans-HK', 'zh-CN', 'en']), 'en')
  // No supported system language falls back to English.
  assert.equal(resolveLanguage('system', ['ja-JP', 'de-DE']), 'en')
  assert.equal(resolveLanguage('system', []), 'en')
})

test('reviewed messages have paired translations and preserve supplied values', () => {
  assert.deepEqual(messageParameterMismatches(), [])
  assert.deepEqual(duplicateMessageIds(), [])
  assert.equal(translate('en', 'settings.instanceTitle', {name: '測試-{label}'}), '測試-{label} · Instance settings')
  assert.equal(translate('zh-TW', 'settings.instanceTitle', {name: 'Example'}), 'Example · 實例設定')
})

test('activity time honors the selected clock and UTC without changing locale', () => {
  const timestamp = Date.UTC(2026, 1, 17, 22, 4, 5)
  assert.match(formatDisplayTime(timestamp, 'en', 'local12'), /[AP]M/i)
  assert.doesNotMatch(formatDisplayTime(timestamp, 'en', 'local24'), /[AP]M/i)
  assert.match(formatDisplayTime(timestamp, 'en', 'utc'), /22:04:05 UTC$/)
  assert.match(formatDisplayTime(timestamp, 'zh-TW', 'utc'), /22:04:05 UTC$/)
})

test('console titles expand only registered tokens and show unavailable values explicitly', () => {
  assert.equal(renderConsoleTitle('{instance} - {type} :{port}', {
    instance: '測試服\u001b\n', port: '25566',
  }), '測試服 - N/A :25566')
  assert.equal(renderConsoleTitle('{instance}', { instance: '{type}' }), '{type}')
  assert.equal(validateConsoleTitle('{metadata.presetVersion}'), undefined)
  assert.equal(validateConsoleTitle('{bogus}'), 'placeholder')
  assert.equal(validateConsoleTitle('{{instance}}'), 'placeholder')
  assert.equal(validateConsoleTitle('{instance}\n'), 'invalid')
  assert.equal(validateConsoleTitle('𠮷'.repeat(129)), 'invalid')
})
