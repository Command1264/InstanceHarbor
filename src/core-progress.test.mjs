import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { coreProgressBytes, coreProgressStep, coreProgressValue } from './core-progress.ts'

const MB = 1024 * 1024

test('every step the backend reports has a message in both languages', () => {
  const source = readFileSync(new URL('../src-tauri/src/core_maintenance.rs', import.meta.url), 'utf8')
  const body = source.slice(source.indexOf('pub enum CoreUpdatePhase'), source.indexOf('pub(super) fn no_progress'))
  const variants = [...body.matchAll(/^ {4}([A-Z]\w+)/gm)].map(match => match[1][0].toLowerCase() + match[1].slice(1))
  assert.deepEqual(variants, ['checking', 'downloading', 'reusing', 'verifying', 'backingUp', 'replacing'])
  for (const phase of variants) {
    const id = coreProgressStep({ phase, received: 0, total: null })
    assert.ok(id, phase)
    assert.match(translate('zh-TW', id), /\p{Script=Han}/u, phase)
    assert.match(translate('en', id), /^[A-Z]/, phase)
  }
  assert.equal(coreProgressStep({ phase: 'somethingNew' }), undefined)
  assert.equal(coreProgressStep(undefined), undefined)
})

test('a download with an official size shows a value and both sizes', () => {
  const progress = { phase: 'downloading', received: 23.4 * MB, total: 52.7 * MB }
  assert.deepEqual(coreProgressValue(progress), { value: 23.4 * MB, max: 52.7 * MB })
  const bytes = coreProgressBytes(progress, 'en')
  assert.equal(translate('en', bytes.id, bytes.values), '23.4 / 52.7 MB')
  const done = { phase: 'downloading', received: 52.7 * MB, total: 52.7 * MB }
  assert.deepEqual(coreProgressValue(done), { value: 52.7 * MB, max: 52.7 * MB })
})

test('without an official size the bar is indeterminate and no percentage is guessed', () => {
  const progress = { phase: 'downloading', received: 12.3 * MB, total: null }
  assert.equal(coreProgressValue(progress), undefined)
  const bytes = coreProgressBytes(progress, 'zh-TW')
  assert.equal(translate('zh-TW', bytes.id, bytes.values), '已下載 12.3 MB')
  assert.doesNotMatch(translate('en', bytes.id, bytes.values), /%|\//)
})

test('steps other than downloading have no value and no size', () => {
  for (const phase of ['checking', 'reusing', 'verifying', 'backingUp', 'replacing']) {
    assert.equal(coreProgressValue({ phase }), undefined, phase)
    assert.equal(coreProgressBytes({ phase }, 'en'), undefined, phase)
  }
})
