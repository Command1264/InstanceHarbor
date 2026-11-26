import assert from 'node:assert/strict'
import { test } from 'node:test'
import { draftFrom, valueCount, valuesFrom, valueText, PROFILE_PROPERTIES, PROFILE_RULES } from './settings-profiles.ts'

const stored = {
  java: '',
  jvm: { minMib: 1024, maxMib: 2048, gc: 'g1', advanced: ['-XX:MaxGCPauseMillis=100'] },
  timing: { stopSeconds: 60, readySeconds: null, restartDelayMs: 500 },
  autoRestart: { onCrash: true, onUnexpectedExit: false, delaySeconds: 10, maxAttempts: 3 },
  serverProperties: { 'serverProperties.motd': 'Hello', 'serverProperties.difficulty': 'hard' },
  gamerules: { 'gamerules.max_entity_cramming': '40' },
}

test('a profile survives the editor unchanged', () => {
  assert.deepEqual(valuesFrom(draftFrom({ id: 'p1', name: 'Lobby', values: stored })), stored)
  assert.equal(valueCount(stored), 7)
})

test('a new draft includes nothing and saves empty values', () => {
  const draft = draftFrom()
  assert.equal(Object.keys(draft.fields).length, PROFILE_PROPERTIES.length + PROFILE_RULES.length)
  assert.deepEqual(valuesFrom(draft), {})
})

test('only included rows are stored, and a set Java path is trimmed', () => {
  const draft = draftFrom()
  draft.java = { include: true, automatic: false, path: '  C:/Java/bin/java.exe ' }
  draft.fields['serverProperties.pvp'].include = true
  draft.fields['gamerules.advance_time'] = { include: true, value: 'false' }
  assert.deepEqual(valuesFrom(draft), {
    java: 'C:/Java/bin/java.exe',
    serverProperties: { 'serverProperties.pvp': 'true' },
    gamerules: { 'gamerules.advance_time': 'false' },
  })
})

test('a malformed number is refused instead of silently dropped', () => {
  const draft = draftFrom()
  draft.timing = { include: true, stopSeconds: '1.5', readySeconds: '', restartDelayMs: '' }
  assert.throws(() => valuesFrom(draft), /number/)
})

test('preview values read as text', () => {
  assert.equal(valueText({ minMib: 1024, maxMib: null, gc: 'g1' }), 'minMib=1024, gc=g1')
  assert.equal(valueText('C:/Java/bin/java.exe'), 'C:/Java/bin/java.exe')
  assert.equal(valueText(40), '40')
})
