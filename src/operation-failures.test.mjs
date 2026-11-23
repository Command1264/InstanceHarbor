import assert from 'node:assert/strict'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { batchFailureReasonId, settledBatchFailures } from './operation-failures.ts'

const item = (name, phase, reason = null) => ({ name, phase, reason, runId: null, canCancel: false })
const job = (state, items, action = 'batch-start') => ({ id: 'b1', action, instance: '', state, items })

test('a running batch reports nothing yet', () => {
  assert.equal(settledBatchFailures(job('Pending', [item('a', 'Failed', 'StartRejected'), item('b', 'Starting')])), undefined)
})

test('a settled batch lists every instance that failed or could not be verified, in order', () => {
  const failures = settledBatchFailures(job('Failed', [
    item('a', 'Completed'), item('b', 'Failed', 'ExitedBeforeReady'), item('c', 'Skipped', 'AlreadyRunning'),
    item('d', 'Unconfirmed', 'ReadinessUnconfirmed'), item('e', 'Cancelled'), item('f', 'StopUnconfirmed'),
  ]))
  assert.deepEqual(failures, [
    { name: 'b', phase: 'Failed', reason: 'ExitedBeforeReady' },
    { name: 'd', phase: 'Unconfirmed', reason: 'ReadinessUnconfirmed' },
    { name: 'f', phase: 'StopUnconfirmed', reason: null },
  ])
  assert.deepEqual(settledBatchFailures(job('Completed', [item('a', 'Completed')])), [])
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
