import assert from 'node:assert/strict'
import { test } from 'node:test'
import { groupOf, listEntries, matchesGroup, shownRows, validFilter } from './server-groups.ts'

const rows = ['alpha', 'beta', 'gamma', 'delta', 'omega'].map(name => ({ name, up: name !== 'beta' }))
const data = {
  groups: [{ id: 'g1', name: 'Lobby' }, { id: 'g2', name: 'Games', collapsed: true }, { id: 'g3', name: 'Empty' }],
  members: { alpha: 'g1', beta: 'g1', gamma: 'g2', ghost: 'g1', omega: 'gone' },
}
const shape = entries => entries.map(entry => entry.kind === 'row' ? entry.row.name : `[${entry.section.name ?? '-'} ${entry.section.members.length}/${entry.section.running}${entry.section.collapsed ? ' closed' : ''}]`)
const arrange = (overrides = {}) => listEntries({ visible: rows, all: rows, pins: [], data, running: row => row.up, showEmpty: true, ...overrides })

test('an instance is in its group only while the group exists', () => {
  assert.equal(groupOf(data, 'alpha'), 'g1')
  assert.equal(groupOf(data, 'delta'), null)
  // A member entry that names a deleted group counts as ungrouped.
  assert.equal(groupOf(data, 'omega'), null)
  assert.equal(groupOf(data, 'constructor'), null)
})

test('without groups the list stays flat', () => {
  const entries = listEntries({ visible: rows, all: rows, pins: ['gamma'], data: { groups: [], members: {} }, running: () => true, showEmpty: true })
  assert.deepEqual(shape(entries), ['alpha', 'beta', 'gamma', 'delta', 'omega'])
})

test('groups follow their order, count members and running servers, and hide collapsed rows', () => {
  assert.deepEqual(shape(arrange()), ['[Lobby 2/1]', 'alpha', 'beta', '[Games 1/1 closed]', '[Empty 0/0]', '[- 2/2]', 'delta', 'omega'])
  assert.deepEqual(shownRows(arrange()).map(row => row.name), ['alpha', 'beta', 'delta', 'omega'])
})

test('pinned servers stay on top and still count in their group', () => {
  assert.deepEqual(shape(arrange({ pins: ['delta', 'beta'] })), ['beta', 'delta', '[Lobby 2/1]', 'alpha', '[Games 1/1 closed]', '[Empty 0/0]', '[- 2/2]', 'omega'])
})

test('a search hides empty groups but headers still count the whole group', () => {
  const visible = rows.filter(row => row.name === 'beta')
  assert.deepEqual(shape(arrange({ visible, showEmpty: false })), ['[Lobby 2/1]', 'beta'])
  const section = arrange({ visible, showEmpty: false })[0].section
  assert.deepEqual(section.members.map(row => row.name), ['alpha', 'beta'])
})

test('the group filter limits the list to a group or to ungrouped servers', () => {
  assert.deepEqual(rows.filter(row => matchesGroup(data, row.name, 'group:g1')).map(row => row.name), ['alpha', 'beta'])
  assert.deepEqual(rows.filter(row => matchesGroup(data, row.name, 'ungrouped')).map(row => row.name), ['delta', 'omega'])
  assert.equal(rows.filter(row => matchesGroup(data, row.name, 'all')).length, rows.length)
  assert.equal(validFilter(data, 'group:g2'), 'group:g2')
  assert.equal(validFilter(data, 'group:gone'), 'all')
})
