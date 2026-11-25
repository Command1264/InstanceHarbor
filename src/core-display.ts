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

const familyNames: Record<string, string> = {
  paper: 'Paper', folia: 'Folia', spigot: 'Spigot', velocity: 'Velocity', purpur: 'Purpur',
}

/**
 * The display name of a core family. Registrations store `PAPER`, the offline
 * reader returns `Paper`; both are shown as `Paper`. Comparisons keep using the
 * stored value case-insensitively. An unknown family is shown as written.
 */
export function coreFamilyName(family: string | undefined): string {
  if (!family) return ''
  const key = family.toLowerCase()
  return Object.hasOwn(familyNames, key) ? familyNames[key] : family
}

export function coreBuildText(build: string | number): string {
  return `Build ${build}`
}

/** `Paper 26.2 · Build 134`, or `Velocity 4.2.1-SNAPSHOT` without a build. */
export function coreVersionText(core: { family?: string; minecraft?: string; build?: string | null }): string {
  const name = [coreFamilyName(core.family), core.minecraft].filter(Boolean).join(' ')
  return core.build ? `${name} · ${coreBuildText(core.build)}` : name
}
