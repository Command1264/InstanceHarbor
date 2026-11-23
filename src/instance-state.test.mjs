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

test('every shown state has a status colour, also on a selected row', async () => {
  const { readFile } = await import('node:fs/promises')
  const { stateTones } = await import('./instance-state.ts')
  const css = await readFile(new URL('./app.css', import.meta.url), 'utf8')
  const manager = await readFile(new URL('./manager.ts', import.meta.url), 'utf8')
  const rules = [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .map(([, selector, body]) => ({ selector, body }))
  // Split a selector list on top-level commas only, so `:is(a,b) .x` stays whole.
  const topLevel = selector => {
    const parts = ['']; let depth = 0
    for (const char of selector) {
      depth += char === '(' ? 1 : char === ')' ? -1 : 0
      if (char === ',' && depth === 0) parts.push(''); else parts[parts.length - 1] += char
    }
    return parts
  }
  const has = (selected, state, colour) => rules.some(({ selector, body }) =>
    topLevel(selector).some(part => /\.badge\b/.test(part) && new RegExp(`\\.${state}\\b`).test(part)
      && /tr\.selected/.test(part) === selected)
    && body.includes(`color:var(${colour}`))

  // States named in the UI and states the presentation layer adds.
  const named = [...manager.matchAll(/(\w+): 'instance\.state\./g)].map(match => match[1])
  for (const state of [...named, 'Queued', 'Stopping', 'Starting'])
    assert.ok(Object.hasOwn(stateTones, state), `${state} needs a colour in stateTones`)

  for (const [state, tone] of Object.entries(stateTones)) {
    assert.ok(has(false, state, `--${tone}`), `.badge.${state} uses --${tone}`)
    assert.ok(has(true, state, `--row-selection-${tone}`), `selected .badge.${state} uses --row-selection-${tone}`)
  }
})
