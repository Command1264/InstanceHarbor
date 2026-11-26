import type { MessageId } from './i18n'
import type { CoreSource } from './manager'

/** One batch target, as `core-batch-submit` reads it. */
export type CoreBatchTarget = {
  name: string
  currentSha256?: string
  /** Paper, Folia and Velocity: the SHA-256 the source states. */
  candidateSha256?: string
  /** Purpur (#113): the official MD5; the SHA-256 is known only after download. */
  candidateMd5?: string
  registrationRevision?: string
}

/** Whether the candidate of a source row is checked by the official Purpur MD5 (#113). */
export function checkedByMd5(source?: CoreSource): boolean {
  return source?.verification === 'purpurMd5' && !source.candidate?.sha256 && !!source.candidate?.md5
}

/** The message of a source row that can enter a batch. */
export function batchUpdateMessage(source: CoreSource): MessageId {
  return checkedByMd5(source) ? 'core.batch.updateToPurpur' : 'core.batch.updateTo'
}

/** Whether a source row names its candidate by a digest the batch accepts. */
export function hasBatchCandidate(source: CoreSource): boolean {
  return !!source.candidate?.sha256 || checkedByMd5(source)
}

/** The batch target of an instance and its checked source row. */
export function batchTarget(name: string, source: CoreSource): CoreBatchTarget {
  const target: CoreBatchTarget = { name, currentSha256: source.sha256, registrationRevision: source.registrationRevision }
  if (checkedByMd5(source)) target.candidateMd5 = source.candidate?.md5
  else target.candidateSha256 = source.candidate?.sha256
  return target
}
