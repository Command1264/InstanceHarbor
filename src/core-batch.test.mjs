import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { batchTarget, batchUpdateMessage, hasBatchCandidate } from './core-batch.ts'

const sha = (digit) => digit.repeat(64)
const paper = { name: 'paper', status: 'UpdateAvailable', family: 'PAPER', sha256: sha('1'), registrationRevision: sha('2'),
  candidate: { build: '205', sha256: sha('3') } }
const purpur = { name: 'purpur', status: 'UpdateAvailable', family: 'PURPUR', sha256: sha('4'), registrationRevision: sha('5'),
  verification: 'purpurMd5', candidate: { build: '2660', md5: 'e'.repeat(32) } }

test('#113: a Purpur candidate enters a batch by its official MD5', () => {
  assert.equal(hasBatchCandidate(purpur), true)
  assert.deepEqual(batchTarget('purpur', purpur),
    { name: 'purpur', currentSha256: sha('4'), candidateMd5: 'e'.repeat(32), registrationRevision: sha('5') })
  assert.deepEqual(JSON.parse(JSON.stringify(batchTarget('paper', paper))),
    { name: 'paper', currentSha256: sha('1'), candidateSha256: sha('3'), registrationRevision: sha('2') })
  // An MD5 without the Purpur verification never stands in for a SHA-256.
  const unverified = { ...purpur, verification: undefined }
  assert.equal(hasBatchCandidate(unverified), false)
})

test('#113: the batch list says that a Purpur row is checked by the official MD5', () => {
  assert.equal(batchUpdateMessage(paper), 'core.batch.updateTo')
  assert.equal(batchUpdateMessage(purpur), 'core.batch.updateToPurpur')
  assert.match(translate('zh-TW', 'core.batch.updateToPurpur', { build: '2660' }), /Build 2660.*以官方 MD5 核對/)
  assert.match(translate('en', 'core.batch.updateToPurpur', { build: '2660' }), /Build 2660.*official MD5/)
  const dialog = readFileSync(new URL('./components/BatchCoreDialog.vue', import.meta.url), 'utf8')
  assert.doesNotMatch(dialog, /core\.batch\.singleOnly/, 'Purpur is no longer sent to single updates')
})

test('#113: the batch target keys match what the backend reads', () => {
  const source = readFileSync(new URL('../src-tauri/src/core_batch.rs', import.meta.url), 'utf8')
  const struct = source.slice(source.indexOf('struct Target {'), source.indexOf('impl Target'))
  const fields = [...struct.matchAll(/^\s+(\w+): /gm)].map(match => match[1].replace(/_(\w)/g, (_, letter) => letter.toUpperCase()))
  for (const target of [batchTarget('paper', paper), batchTarget('purpur', purpur)]) {
    for (const key of Object.keys(target)) assert.ok(fields.includes(key), key)
  }
})

test('#113: an item whose download does not match the MD5 says so', () => {
  assert.match(translate('zh-TW', 'activity.reason.CoreArtifactDigestMismatch'), /官方雜湊不符/)
  assert.match(translate('en', 'activity.reason.CoreArtifactDigestMismatch'), /did not match/)
})
