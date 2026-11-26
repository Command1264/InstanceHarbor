import assert from 'node:assert/strict'
import test from 'node:test'
import { cacheReasonId, formatBytes } from './cache-cleanup.ts'

test('sizes use binary units with one decimal from KiB up', () => {
  assert.equal(formatBytes(0, 'en'), '0 B')
  assert.equal(formatBytes(1023, 'en'), '1,023 B')
  assert.equal(formatBytes(1024, 'en'), '1.0 KiB')
  assert.equal(formatBytes(1536 * 1024, 'en'), '1.5 MiB')
  assert.equal(formatBytes(55242379, 'en'), '52.7 MiB')
  assert.equal(formatBytes(3 * 1024 ** 4, 'en'), '3.0 TiB')
})

test('reported reasons map to messages and unknown ones fall back', () => {
  assert.equal(cacheReasonId('FileInUse'), 'components.cache.reason.fileInUse')
  assert.equal(cacheReasonId('WebViewCacheCustomFolder'), 'components.cache.reason.customFolder')
  assert.equal(cacheReasonId('os error 145'), undefined)
})
