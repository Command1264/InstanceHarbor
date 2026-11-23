import assert from 'node:assert/strict'
import { test } from 'node:test'
import { coreDiagnosticText } from './core-diagnostics.ts'

test('core diagnostics retain stable job IDs and strip paths, secrets and unknown fields', () => {
  const output = JSON.parse(coreDiagnosticText({
    instance: 'paper-agent',
    installed: { family: 'PAPER', minecraft: '26.3', build: '205', path: 'C:/private/config' },
    installedSha256: 'a'.repeat(64),
    candidate: { build: '206', sha256: 'https://name:secret@upstream' },
    pendingRequest: 'request-1',
    lastRequest: 'request-2',
    backupIds: ['backup-1', 'C:/secret'],
    checkedAt: 0,
    error: 'C:/secret',
  }))
  assert.deepEqual(output, {
    schemaVersion: 1,
    instance: 'paper-agent',
    installed: { family: 'PAPER', minecraft: '26.3', build: '205', sha256: 'a'.repeat(64) },
    candidate: { build: '206' },
    pendingRequest: 'request-1',
    lastRequest: 'request-2',
    backupIds: ['backup-1'],
    checkedAt: '1970-01-01T00:00:00.000Z',
  })
})
