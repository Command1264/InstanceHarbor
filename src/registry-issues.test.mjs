import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { registryDiagnosticText, registryIssueMessage, stateIssueMessage } from './registry-issues.ts'
import { coreBlockedMessage } from './core-diagnostics.ts'

const text = (locale, message) => translate(locale, message.id, message.values)

test('an unreadable state names the actual cause in the banner', () => {
  const issue = (reason, code) => ({ file: 'paper.json', name: 'paper', stage: 'observation', reason, code, osError: null })
  assert.equal(text('zh-TW', registryIssueMessage(issue('AccessDenied', 'OsError'))), '無法讀取實例狀態：Windows 拒絕存取')
  assert.equal(text('en', registryIssueMessage(issue('FileInUse', 'FileAccess'))), 'Could not read the instance state: a file is in use by another program')
  assert.match(text('zh-TW', registryIssueMessage(issue('InvalidStateFile', 'InvalidStateFile'))), /執行紀錄檔/)
  assert.match(text('zh-TW', registryIssueMessage(issue('PathRedirected', 'PathRedirected'))), /重新導向/)
  // Causes without dedicated text show the stable code, so the user can report it.
  assert.equal(text('en', registryIssueMessage(issue('ObservationUnconfirmed', 'ScanDirectoryUnavailable'))),
    'Could not verify the instance state (error code ScanDirectoryUnavailable)')
  // A code that is not an identifier is never shown.
  assert.equal(text('en', registryIssueMessage(issue('ObservationUnconfirmed', 'C:\\Users\\x'))), 'Instance state could not be verified')
})

test('registration problems keep their existing labels, also without a stage', () => {
  assert.equal(text('zh-TW', registryIssueMessage({ file: 'a.json', reason: 'FileInUse', osError: 32 })), '檔案被占用')
  assert.equal(text('zh-TW', registryIssueMessage({ file: 'a.json', stage: 'registration', reason: 'InvalidRecord', osError: null })), '登記內容無效')
  assert.equal(text('zh-TW', registryIssueMessage({ file: 'a.json', stage: 'registration', reason: 'Unconfirmed', osError: null })), '無法核對')
})

test('a list row gives a short cause', () => {
  assert.equal(text('zh-TW', stateIssueMessage({ reason: 'AccessDenied', code: 'OsError', osError: 5 })), '拒絕存取')
  assert.equal(text('en', stateIssueMessage({ reason: 'ObservationUnconfirmed', code: 'UnclassifiedError', osError: null })), 'Error code UnclassifiedError')
  assert.equal(text('en', stateIssueMessage({ reason: 'ObservationUnconfirmed', osError: null })), 'Could not verify')
})

test('copied registry diagnostics keep codes and drop paths and unknown fields', () => {
  const output = JSON.parse(registryDiagnosticText([
    { file: 'paper.json', name: 'paper', stage: 'observation', reason: 'AccessDenied', code: 'OsError', osError: 5,
      access: { stage: 'read', file: 'managed-file' }, path: 'C:/Users/someone' },
    { file: 'C:/secret.json', name: 'C:/secret', stage: 'other', reason: 'Unconfirmed', code: 'has space', osError: null,
      access: { stage: 'C:/x', file: 'D:/y' } },
  ]))
  assert.deepEqual(output, {
    schemaVersion: 1,
    issues: [
      { file: 'paper.json', name: 'paper', stage: 'observation', reason: 'AccessDenied', code: 'OsError', osError: 5,
        access: { stage: 'read', file: 'managed-file' } },
      { reason: 'Unconfirmed', access: {} },
    ],
  })
})

test('the core dialog explains each reason that disables Update core', () => {
  for (const reason of ['CoreMaintenancePending', 'CoreProfileUnverified', 'InstanceNotConfirmedStopped',
    'CoreStopEvidenceMissing', 'PortChanged', 'PortOccupied PID=42', 'PortOccupiedOrUnavailable']) {
    const id = coreBlockedMessage(reason)
    assert.ok(id, reason)
    assert.notEqual(translate('en', id), id, reason)
  }
  assert.equal(coreBlockedMessage('FileAccess[read] path=C:/x: Access is denied. (os error 5)'), undefined)
  assert.equal(coreBlockedMessage(null), undefined)
})

test('the core dialog shows the reason next to the disabled button, not only as a tooltip', async () => {
  const source = await readFile(new URL('./components/CoreMaintenanceDialog.vue', import.meta.url), 'utf8')
  assert.match(source, /id="core-update-blocked"/)
  assert.match(source, /aria-describedby="[^"]*core-update-blocked/)
})

test('an unreadable state has a status colour and is neither stoppable nor startable', async () => {
  const { stateTones } = await import('./instance-state.ts')
  assert.equal(stateTones.StateUnconfirmed, 'error')
  assert.equal(translate('zh-TW', 'instance.state.stateUnconfirmed'), '狀態無法核對')
})
