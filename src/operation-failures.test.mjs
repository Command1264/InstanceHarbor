import assert from 'node:assert/strict'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { batchFailureReasonId, newBatchFailures } from './operation-failures.ts'

const item = (name, phase, reason = null) => ({ name, phase, reason, runId: null, canCancel: false })
const job = (state, items, action = 'batch-start') => ({ id: 'b1', action, instance: '', state, items })

test('a running batch reports each failure as soon as it appears, once', () => {
  const reported = new Set()
  const running = job('Pending', [item('a', 'Failed', 'JavaExecutableMissing'), item('b', 'Starting'), item('c', 'Queued')])
  assert.deepEqual(newBatchFailures(running, reported), [{ name: 'a', phase: 'Failed', reason: 'JavaExecutableMissing' }])
  assert.deepEqual(newBatchFailures(running, reported), [])
  const later = job('Pending', [item('a', 'Failed', 'JavaExecutableMissing'), item('b', 'Unconfirmed', 'ReadinessUnconfirmed'), item('c', 'Starting')])
  assert.deepEqual(newBatchFailures(later, reported), [{ name: 'b', phase: 'Unconfirmed', reason: 'ReadinessUnconfirmed' }])
})

test('only failed or unverified instances count, in batch order', () => {
  const failures = newBatchFailures(job('Failed', [
    item('a', 'Completed'), item('b', 'Failed', 'ExitedBeforeReady'), item('c', 'Skipped', 'AlreadyRunning'),
    item('d', 'Unconfirmed', 'ReadinessUnconfirmed'), item('e', 'Cancelled'), item('f', 'StopUnconfirmed'),
  ]), new Set())
  assert.deepEqual(failures.map(failure => failure.name), ['b', 'd', 'f'])
  assert.deepEqual(newBatchFailures(job('Completed', [item('a', 'Completed')]), new Set()), [])
})

test('batch failure reasons use the activity reason, the Java start message, or the diagnostics hint', () => {
  assert.equal(batchFailureReasonId('ExitedBeforeReady', 'Failed'), 'activity.reason.ExitedBeforeReady')
  assert.equal(batchFailureReasonId('JavaExecutableMissing: C:/x/java.exe', 'Failed'), 'manager.error.javaExecutableMissing')
  assert.equal(batchFailureReasonId(null, 'Unconfirmed'), 'activity.state.Unconfirmed')
  assert.equal(batchFailureReasonId('SomethingNew', 'Failed'), 'activity.checkDiagnostics')
})

test('the batch failure summary names the action and count in both languages', () => {
  for (const locale of ['zh-TW', 'en']) {
    const text = translate(locale, 'app.batchFailed', { action: translate(locale, 'app.batch.start'), count: 3 })
    assert.ok(text.includes('3') && text.includes(translate(locale, 'app.batch.start')), text)
  }
})
