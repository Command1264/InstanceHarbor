import { test } from 'node:test'
import assert from 'node:assert/strict'
import { resolveSelectionColor, resolveTheme, shouldRespondToSystemTheme } from './theme.ts'

test('missing and legacy product selection colors resolve to blue', () => {
  assert.equal(resolveSelectionColor(undefined), 'blue')
  assert.equal(resolveSelectionColor('product'), 'blue')
  assert.equal(resolveSelectionColor('system'), 'system')
})

test('explicit light and dark remain stable when the OS color scheme changes', () => {
  for (const prefersDark of [false, true]) {
    assert.equal(resolveTheme('light', prefersDark), 'light')
    assert.equal(resolveTheme('dark', prefersDark), 'dark')
    assert.equal(shouldRespondToSystemTheme('light', false), false)
    assert.equal(shouldRespondToSystemTheme('dark', false), false)
  }
})

test('system theme follows the OS outside a settings preview', () => {
  assert.equal(resolveTheme('system', false), 'light')
  assert.equal(resolveTheme('system', true), 'dark')
  assert.equal(shouldRespondToSystemTheme('system', false), true)
  assert.equal(shouldRespondToSystemTheme('system', true), false)
})
