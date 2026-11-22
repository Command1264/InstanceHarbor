import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { policyMessages } from './messages/policy.ts'

// The Rust catalog sends Chinese labels and reasons plus stable codes.
// Every code needs a message, and its zh-TW text must match the catalog exactly.
const catalog = readFileSync(new URL('../src-tauri/src/policy_catalog.rs', import.meta.url), 'utf8')

function catalogFields() {
  const block = catalog.slice(catalog.indexOf('fields! {'), catalog.indexOf('#[derive(Clone, Copy, PartialEq, Eq)]'))
  const fields = []
  let section = ''
  for (const line of block.split(/\r?\n/)) {
    const sectionMatch = line.match(/^ {4}"([A-Za-z]*)" \{/)
    if (sectionMatch) section = sectionMatch[1]
    const fieldMatch = line.match(/^ {8}"([^"]+)" => "([^"]+)",/)
    if (fieldMatch) fields.push({ id: section ? `${section}.${fieldMatch[1]}` : fieldMatch[1], label: fieldMatch[2] })
  }
  return fields
}

function catalogReasons() {
  const block = catalog.slice(catalog.indexOf('pub(super) const REASONS'), catalog.indexOf('pub(super) fn reason_text'))
  return [...block.matchAll(/\("([A-Za-z]+)", "([^"]+)"\)/g)].map(([, code, text]) => ({ code, text }))
}

test('every catalog field has a matching translated label', () => {
  const fields = catalogFields()
  assert.equal(fields.length, 49)
  for (const { id, label } of fields) {
    assert.deepEqual(policyMessages[`policy.field.${id}`]?.[0], label, id)
  }
})

test('every catalog reason code has a matching translated reason', () => {
  const reasons = catalogReasons()
  assert.ok(reasons.length >= 14)
  for (const { code, text } of reasons) {
    assert.deepEqual(policyMessages[`policy.reason.${code}`]?.[0], text, code)
  }
})

test('policy messages do not keep entries the catalog no longer sends', () => {
  const known = new Set([
    ...catalogFields().map(({ id }) => `policy.field.${id}`),
    ...catalogReasons().map(({ code }) => `policy.reason.${code}`),
    'policy.source.rule',
    'policy.source.time',
  ])
  assert.deepEqual(Object.keys(policyMessages).filter(id => !known.has(id)), [])
})
