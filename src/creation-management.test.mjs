import assert from 'node:assert/strict'
import { test } from 'node:test'
import { managementChoice, NO_MANAGEMENT_FOLDER, previewedManagementChoice, selectedManagementFolder } from './creation-management.ts'

test('the parent folder is the default and other choices are sent explicitly', () => {
  const options = ['C:/ws/instances/group/paper-new', 'C:/ws/instances/group', 'C:/ws/instances']
  assert.equal(managementChoice(options[1], options), undefined)
  assert.deepEqual(managementChoice(options[0], options), { kind: 'folder', path: options[0] })
  assert.deepEqual(managementChoice(options[2], options), { kind: 'folder', path: options[2] })
  assert.deepEqual(managementChoice(NO_MANAGEMENT_FOLDER, options), { kind: 'none' })
})

test('the selected option round-trips through the draft field', () => {
  const options = ['C:/ws/instances/group/paper-new', 'C:/ws/instances/group', 'C:/ws/instances']
  for (const selected of [...options, NO_MANAGEMENT_FOLDER]) {
    assert.equal(selectedManagementFolder(managementChoice(selected, options), options), selected)
  }
})

test('a failed preview selects the option of the preview still on screen again', () => {
  const options = ['C:/ws/instances/group/paper-new', 'C:/ws/instances/group', 'C:/ws/instances']
  for (const shown of [...options, NO_MANAGEMENT_FOLDER]) {
    const choice = managementChoice(shown, options)
    const previewed = { name: 'paper-new', parent: 'instances/group', port: 25565, ...(choice ? { managementFolder: choice } : {}) }
    assert.equal(selectedManagementFolder(previewedManagementChoice(previewed), options), shown)
  }
  assert.equal(previewedManagementChoice(undefined), undefined)
})
