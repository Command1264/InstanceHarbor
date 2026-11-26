import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { queuesBehindActive } from './refresh-queue.ts'

// #118: a slow instance-snapshot must not turn the 1.5 s poll into back-to-back reads.
test('only a manual refresh waits behind the read in progress', () => {
  assert.equal(queuesBehindActive('manual'), true)
  assert.equal(queuesBehindActive('background'), false)
})

test('the list poll asks the queue rule before queuing', async () => {
  const app = await readFile(new URL('./App.vue', import.meta.url), 'utf8')
  const refresh = app.slice(app.indexOf('async function refresh('), app.indexOf('function invalidateRefresh('))
  const busy = refresh.slice(refresh.indexOf('if(refreshActive){'), refresh.indexOf('refreshActive=true'))
  assert.match(busy, /if\(!queuesBehindActive\(manual\?'manual':'background'\)\)return/)
  assert.ok(busy.indexOf('queuesBehindActive') < busy.indexOf('refreshQueued=true'), busy)
})
