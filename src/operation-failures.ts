import { isMessageId, type MessageId } from './i18n.ts'
import { javaStartErrorId } from './java-start-error.ts'
import type { BatchItem, Job } from './manager.ts'

export interface BatchFailure { name: string; phase: BatchItem['phase']; reason: string | null }

const failedPhases: readonly BatchItem['phase'][] = ['Failed', 'Unconfirmed', 'StopUnconfirmed']

// Items a finished batch could not complete or verify. Skipped and cancelled items
// are not failures. Returns undefined while the batch is still running.
export function settledBatchFailures(job: Job): BatchFailure[] | undefined {
  if (job.state === 'Pending') return undefined
  return (job.items ?? []).filter(item => failedPhases.includes(item.phase))
    .map(({ name, phase, reason }) => ({ name, phase, reason }))
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
