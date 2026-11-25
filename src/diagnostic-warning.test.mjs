import assert from 'node:assert/strict'
import { test } from 'node:test'
import { diagnosticWarning } from './diagnostic-warning.ts'

test('the warning names the log folder of the development workspace', () => {
  for (const locale of ['zh-TW', 'en']) {
    const text = diagnosticWarning(locale, { available: false, error: 'DiagnosticWriteSession (os error 112)', folder: '%LOCALAPPDATA%/InstanceHarbor Dev/logs/agent-71' })
    assert.ok(text.includes('%LOCALAPPDATA%/InstanceHarbor Dev/logs/agent-71'), text)
    assert.ok(text.includes('os error 112'), text)
    assert.ok(!text.includes('%LOCALAPPDATA%/InstanceHarbor/logs'), text)
  }
})

test('a full segment is not reported as a disk problem', () => {
  for (const locale of ['zh-TW', 'en']) {
    const text = diagnosticWarning(locale, { available: false, error: 'DiagnosticDirectoryBudgetReached', folder: '%LOCALAPPDATA%/InstanceHarbor/logs' })
    assert.ok(text.includes('16 MB'), text)
    assert.ok(!/磁碟|disk/i.test(text), text)
  }
})

test('an older native side without a folder falls back to the stable folder', () => {
  const text = diagnosticWarning('en', { available: false, error: 'DiagnosticWriterBusy' })
  assert.ok(text.includes('%LOCALAPPDATA%/InstanceHarbor/logs'), text)
})
