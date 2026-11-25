import assert from 'node:assert/strict'
import { test } from 'node:test'
import { managementChoice, NO_MANAGEMENT_FOLDER } from './creation-management.ts'

test('the parent folder is the default and other choices are sent explicitly', () => {
  const options = ['C:/ws/instances/group/paper-new', 'C:/ws/instances/group', 'C:/ws/instances']
  assert.equal(managementChoice(options[1], options), undefined)
  assert.deepEqual(managementChoice(options[0], options), { kind: 'folder', path: options[0] })
  assert.deepEqual(managementChoice(options[2], options), { kind: 'folder', path: options[2] })
  assert.deepEqual(managementChoice(NO_MANAGEMENT_FOLDER, options), { kind: 'none' })
})
