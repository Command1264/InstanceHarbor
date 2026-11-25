import type { MessageId } from './i18n.ts'

/**
 * Stable codes `presentation::core_metadata` returns when it cannot name the
 * current core. A new code needs a translation; core-display.test.mjs checks
 * the list against the Rust source.
 */
const coreReasonIds: Record<string, MessageId> = {
  CorePathUnverified: 'instances.coreReason.CorePathUnverified',
  CoreUnreadable: 'instances.coreReason.CoreUnreadable',
  CoreProfileUnverified: 'instances.coreReason.CoreProfileUnverified',
}

/** The message for an unknown core, or undefined when no reason was given. */
export function coreReasonId(reason: string | undefined): MessageId | undefined {
  if (!reason) return undefined
  return Object.hasOwn(coreReasonIds, reason) ? coreReasonIds[reason] : 'instances.coreReason.other'
}
