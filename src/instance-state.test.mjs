import assert from 'node:assert/strict'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { presentedState } from './instance-state.ts'

const row = (state, extra = {}) => ({ name: 'a', state, run: { id: 'r1' }, ...extra })
const none = { queued: false, starting: false, stopping: false }

test('a requested stop shows Stopping until the run is confirmed stopped', () => {
  for (const state of ['Listening', 'Starting', 'RunUnconfirmed']) {
    assert.equal(presentedState(row(state), { ...none, stopping: true }), 'Stopping', state)
    for (const stopPhase of ['Queued', 'Sent', 'Typed'])
      assert.equal(presentedState(row(state, { stopPhase }), none), 'Stopping', `${state} ${stopPhase}`)
  }
  assert.equal(presentedState(row('Stopped', { stopPhase: 'Sent' }), { ...none, stopping: true }), 'Stopped')
  // Another process on the port is still reported as such.
  assert.equal(presentedState(row('PortOccupied', { stopPhase: 'Sent' }), none), 'PortOccupied')
})

test('without a stop request the observed state is shown unchanged', () => {
  for (const state of ['Listening', 'Starting', 'RunUnconfirmed', 'Stopped', 'PortOccupied'])
    assert.equal(presentedState(row(state), none), state)
  assert.equal(presentedState(row('Stopped'), { ...none, queued: true }), 'Queued')
  assert.equal(presentedState(row('Stopped'), { ...none, starting: true }), 'Starting')
})

test('Stopping has a label in both languages', () => {
  assert.equal(translate('zh-TW', 'instance.state.stopping'), '停止中')
  assert.equal(translate('en', 'instance.state.stopping'), 'Stopping')
})
