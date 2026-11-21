import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createLifecycle } from './lifecycle.js'

function fixture(statuses) {
  const rows = statuses.map((status, id) => ({ id: String(id), name: `demo-${id}`, status }))
  const state = { connected: true, scenario: 'normal', tasks: [], activeStartCount: 0,
    startPumps: new Set(), instanceSettings: Object.fromEntries(rows.map(row => [row.id, { java: 'demo' }])) }
  let now = 0
  const pending = []
  const receipts = []
  const api = createLifecycle({ getState: () => state, resetScenario: () => { state.scenario = 'normal' },
    newRecord(action, ids) { const record = { id: String(receipts.length), action, ids, result: '進行中' }; receipts.push(record); return record },
    rowBy: id => rows.find(row => row.id === id), managedRow: () => true, probeJava: () => ({ status: 'ready' }),
    timer: (callback, delay) => pending.push({ callback, at: now + delay }),
    refreshTaskUI() {}, readableTime: () => 'demo-time', announce() {}, esc: String })
  const tick = () => { pending.sort((a, b) => a.at - b.at); const next = pending.shift(); if (next) { now = next.at; next.callback() } }
  const drain = () => { let limit = 100; while (pending.length && limit-- > 0) tick(); assert.ok(limit > 0) }
  return { api, rows, state, receipts, tick, drain }
}

test('restart sends all stops without serial waits, starts at most two and finishes every target', () => {
  const f = fixture(['running', 'running', 'running'])
  const task = f.api.runLifecycle('restart', ['0', '1', '2'], '重新啟動', 'selected')
  assert.deepEqual(task.stopSent, ['0', '1', '2'])
  assert.ok(f.rows.every(row => row.status === 'stopping'))
  while (!task.startSent.length) f.tick()
  assert.ok(f.state.activeStartCount <= 2)
  f.drain()
  assert.equal(task.done, true)
  assert.ok(task.maxStarting <= 2)
  assert.ok(f.rows.every(row => row.status === 'running'))
})

test('stop timeout keeps the process observation and never schedules a restart', () => {
  const f = fixture(['running'])
  f.state.scenario = 'timeout'
  const task = f.api.runLifecycle('restart', ['0'], '重新啟動', 'selected')
  f.drain()
  assert.deepEqual(task.startSent, [])
  assert.equal(f.rows[0].status, 'running')
  assert.match(task.record.detail, /逾時/)
})

test('force stop finishes once and late normal-stop callbacks cannot change its result', () => {
  const f = fixture(['running'])
  const task = f.api.runLifecycle('stop', ['0'], '正常停止', 'selected')
  const force = task.forceStop.get('0')
  assert.equal(force(), true)
  assert.equal(force(), false)
  f.drain()
  assert.equal(f.rows[0].status, 'stopped')
  assert.equal(task.results.length, 1)
  assert.equal(f.receipts.filter(record => record.action === '強制停止').length, 1)
})

test('separate tasks share start capacity and cancellation does not retract issued stops', () => {
  const f = fixture(['stopped', 'stopped', 'stopped', 'running'])
  const first = f.api.runLifecycle('start', ['0', '1'], '啟動', 'selected')
  const second = f.api.runLifecycle('restart', ['2', '3'], '重新啟動', 'selected')
  assert.equal(f.state.activeStartCount, 2)
  assert.deepEqual(second.stopSent, ['3'])
  second.cancelled = true
  second.cancelPendingStarts()
  f.drain()
  assert.equal(first.done, true)
  assert.equal(second.done, true)
  assert.deepEqual(second.startSent, [])
  assert.equal(f.rows[3].status, 'stopped')
})
