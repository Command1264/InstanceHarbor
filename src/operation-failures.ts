import { isMessageId, type MessageId } from './i18n.ts'
import { javaStartErrorId } from './java-start-error.ts'
import type { BatchItem, Job } from './manager.ts'

export interface BatchFailure { name: string; phase: BatchItem['phase']; reason: string | null }

const failedPhases: readonly BatchItem['phase'][] = ['Failed', 'Unconfirmed', 'StopUnconfirmed']

// Instances in a batch that failed or could not be verified and are not in
// `reported` yet. They are added to `reported`, so each one is reported once,
// even while the rest of the batch is still running. Skipped and cancelled
// instances are not failures.
export function newBatchFailures(job: Job, reported: Set<string>): BatchFailure[] {
  const fresh = (job.items ?? []).filter(item => failedPhases.includes(item.phase) && !reported.has(item.name))
  for (const item of fresh) reported.add(item.name)
  return fresh.map(({ name, phase, reason }) => ({ name, phase, reason }))
}

export function batchFailureReasonId(reason: string | null, phase: BatchItem['phase']): MessageId {
  if (reason) {
    const id = 'activity.reason.' + reason
    if (isMessageId(id)) return id
    const java = javaStartErrorId(reason)
    if (java) return java
    return 'activity.checkDiagnostics'
  }
  return phase === 'Failed' ? 'activity.checkDiagnostics' : 'activity.state.Unconfirmed'
}
