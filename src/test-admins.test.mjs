import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  formatNodes, nodeProblem, parseNodes, shouldLookup, testAdminPayload, validMinecraftName,
} from './test-admins.ts'

test('names follow the Minecraft rule before any lookup', () => {
  for (const name of ['abc', 'Fake_Admin', 'a'.repeat(16), '123']) assert.equal(validMinecraftName(name), true, name)
  for (const name of ['', 'ab', 'a'.repeat(17), 'bad-name', 'has space', 'ünï', 'a\n']) assert.equal(validMinecraftName(name), false, name)
})

test('a lookup never overwrites a UUID the user typed', () => {
  assert.equal(shouldLookup({ name: 'Fake_Admin', uuid: '' }, false), true)
  assert.equal(shouldLookup({ name: 'Fake_Admin', uuid: '  ' }, false), true)
  assert.equal(shouldLookup({ name: 'Fake_Admin', uuid: '00000000-0000-4000-8000-000000000001' }, true), true)
  assert.equal(shouldLookup({ name: 'Fake_Admin', uuid: '00000000-0000-4000-8000-000000000001' }, false), false)
  assert.equal(shouldLookup({ name: 'no', uuid: '' }, false), false)
})

test('node text splits into lines and keeps the negation sign', () => {
  assert.deepEqual(parseNodes(' essentials.fly \r\n\n-worldedit.*\n  \n'), ['essentials.fly', '-worldedit.*'])
  assert.equal(formatNodes(['a.b', '-c.d']), 'a.b\n-c.d')
  assert.equal(formatNodes(null), '')
})

test('node problems match what the backend refuses', () => {
  assert.equal(nodeProblem(''), 'empty')
  assert.equal(nodeProblem('\n \n'), 'empty')
  assert.equal(nodeProblem('Essentials.Fly'), 'invalid')
  assert.equal(nodeProblem('--x'), 'invalid')
  assert.equal(nodeProblem('-'), 'invalid')
  assert.equal(nodeProblem('a'.repeat(201)), 'invalid')
  assert.equal(nodeProblem(Array.from({ length: 51 }, (_, index) => `plugin.node${index}`).join('\n')), 'tooMany')
  assert.equal(nodeProblem('essentials.fly\n-worldedit.*\nessentials.fly'), undefined)
})

test('the payload sends custom nodes and the group only while LuckPerms is on', () => {
  const rows = [{ name: ' Fake_Admin ', uuid: ' 00000000-0000-4000-8000-000000000001 ' }]
  const base = { rows, luckperms: true, op: true, kind: 'custom', nodes: 'a.b\n-c.d', group: ' staff ' }
  assert.deepEqual(testAdminPayload(base), {
    administrators: [{ name: 'Fake_Admin', uuid: '00000000-0000-4000-8000-000000000001' }],
    luckperms: true, op: true, permissions: ['a.b', '-c.d'], group: 'staff',
  })
  assert.equal(testAdminPayload({ ...base, kind: 'all' }).permissions, null)
  const off = testAdminPayload({ ...base, luckperms: false })
  assert.deepEqual([off.permissions, off.group, off.op], [null, '', true])
})
