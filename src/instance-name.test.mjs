import assert from 'node:assert/strict'
import { test } from 'node:test'
import { coreDiagnosticText } from './core-diagnostics.ts'
import { instanceNameIssue } from './instance-name.ts'

test('instance names allow inner periods and explain each refused name', () => {
  for (const name of ['paper-1.14.4', 'a.b', 'v1..2', 'console.1', 'a'.repeat(64)]) {
    assert.equal(instanceNameIssue(name), '', name)
  }
  const cases = {
    '': 'required',
    ['a'.repeat(65)]: 'tooLong',
    '.': 'start',
    '..': 'start',
    '.paper': 'start',
    '-paper': 'start',
    'paper.': 'trailingPeriod',
    'paper..': 'trailingPeriod',
    'Paper-1.14.4': 'start',
    'paper 1': 'characters',
    'paper_1': 'characters',
    con: 'reserved',
    'con.1': 'reserved',
    'nul.json': 'reserved',
    'lpt9.paper': 'reserved',
  }
  for (const [name, issue] of Object.entries(cases)) {
    assert.equal(instanceNameIssue(name), issue, JSON.stringify(name))
  }
})

test('core diagnostics keep a dotted instance name but job IDs stay strict', () => {
  const output = JSON.parse(coreDiagnosticText({ instance: 'paper-1.14.4', pendingRequest: 'request.1', backupIds: ['backup.1'] }))
  assert.equal(output.instance, 'paper-1.14.4')
  assert.equal(output.pendingRequest, undefined)
  assert.deepEqual(output.backupIds, [])
  assert.equal(JSON.parse(coreDiagnosticText({ instance: 'paper.' })).instance, undefined)
})

test('registry diagnostics keep a dotted instance name and its registration file', async () => {
  const { registryDiagnosticText } = await import('./registry-issues.ts')
  const output = JSON.parse(registryDiagnosticText([
    { file: 'paper-1.14.4.json', name: 'paper-1.14.4', reason: 'Unconfirmed', osError: null },
    { file: 'paper..json', name: 'paper.', reason: 'Unconfirmed', osError: null },
  ]))
  assert.equal(output.issues[0].file, 'paper-1.14.4.json')
  assert.equal(output.issues[0].name, 'paper-1.14.4')
  assert.equal(output.issues[1].file, undefined)
  assert.equal(output.issues[1].name, undefined)
})
