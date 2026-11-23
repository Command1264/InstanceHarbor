import assert from 'node:assert/strict'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { javaStartErrorId } from './java-start-error.ts'

test('start preflight Java codes map to their own messages', () => {
  assert.equal(javaStartErrorId('JavaExecutableMissing'), 'manager.error.javaExecutableMissing')
  assert.equal(javaStartErrorId('JavaExecutableUnreadable: Access is denied. (os error 5)'), 'manager.error.javaExecutableUnreadable')
  // A raw OS message no longer reaches the user for a removed registered Java.
  assert.equal(javaStartErrorId('系統找不到指定的路徑。 (os error 3)'), undefined)
  assert.equal(javaStartErrorId('InvalidJavaExecutable'), undefined)
})

test('the Java start messages name the settings section that fixes them in both languages', () => {
  for (const id of ['manager.error.javaExecutableMissing', 'manager.error.javaExecutableUnreadable']) {
    for (const locale of ['zh-TW', 'en']) {
      const text = translate(locale, id)
      assert.ok(text.includes(translate(locale, 'instances.settings')), `${locale} ${id}: ${text}`)
      assert.ok(text.includes(translate(locale, 'settings.java')), `${locale} ${id}: ${text}`)
    }
  }
})
