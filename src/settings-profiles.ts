// Settings profiles (#36). The data comes from `settings-profiles`; this file turns
// the editor draft into the stored values and describes preview rows.
import type { AutoRestart, Jvm, LifecycleTiming } from './manager.ts'
import { isMessageId, tr, type MessageId } from './i18n.ts'

export interface SettingsProfileValues {
  /** `''` selects Java automatically; otherwise the path to java.exe. */
  java?: string
  jvm?: Jvm
  timing?: LifecycleTiming
  autoRestart?: AutoRestart
  /** `serverProperties.<key>` → value */
  serverProperties?: Record<string, string>
  /** `gamerules.<key>` → value */
  gamerules?: Record<string, string>
}
export interface SettingsProfile { id: string; name: string; values: SettingsProfileValues }
export interface SettingsProfiles { profiles: SettingsProfile[]; groupProfiles: Record<string, string>; revision?: string }
export interface ProfileChange { key: string; before: unknown; after: unknown; path?: string; deferred?: boolean }
export interface ProfileSkip { key: string; reason: string }
export interface ProfilePreviewItem { name: string; state?: string; changes: ProfileChange[]; skipped: ProfileSkip[]; error?: string }
export interface ProfilePreview { profile: { id: string; name: string }; items: ProfilePreviewItem[]; token: string }
export interface ProfileApplyItem { name: string; state: 'Completed' | 'Failed' | 'Skipped'; applied: string[]; deferred: string[]; skipped: ProfileSkip[]; error?: string }
export interface ProfileApplyResult { request: string; state: string; items: ProfileApplyItem[] }

export const emptySettingsProfiles: SettingsProfiles = { profiles: [], groupProfiles: {} }

type FieldKind = { kind: 'bool' } | { kind: 'int'; min: number; max: number } | { kind: 'text'; max: number } | { kind: 'choice'; choices: string[] }
/** The common `server.properties` fields a profile offers. */
export const PROFILE_PROPERTIES: { key: string; field: FieldKind }[] = [
  { key: 'serverProperties.motd', field: { kind: 'text', max: 120 } },
  { key: 'serverProperties.max-players', field: { kind: 'int', min: 1, max: 1000 } },
  { key: 'serverProperties.difficulty', field: { kind: 'choice', choices: ['peaceful', 'easy', 'normal', 'hard'] } },
  { key: 'serverProperties.gamemode', field: { kind: 'choice', choices: ['survival', 'creative', 'adventure', 'spectator'] } },
  { key: 'serverProperties.pvp', field: { kind: 'bool' } },
  { key: 'serverProperties.view-distance', field: { kind: 'int', min: 2, max: 32 } },
  { key: 'serverProperties.simulation-distance', field: { kind: 'int', min: 2, max: 32 } },
  { key: 'serverProperties.spawn-protection', field: { kind: 'int', min: 0, max: 256 } },
  { key: 'serverProperties.allow-nether', field: { kind: 'bool' } },
  { key: 'serverProperties.allow-flight', field: { kind: 'bool' } },
  { key: 'serverProperties.spawn-monsters', field: { kind: 'bool' } },
  { key: 'serverProperties.white-list', field: { kind: 'bool' } },
  { key: 'serverProperties.enable-command-block', field: { kind: 'bool' } },
]
/** Game rules a profile offers; the next start applies them. */
export const PROFILE_RULES: { key: string; field: FieldKind }[] = [
  { key: 'gamerules.advance_time', field: { kind: 'bool' } },
  { key: 'gamerules.spawn_mobs', field: { kind: 'bool' } },
  { key: 'gamerules.command_block_output', field: { kind: 'bool' } },
  { key: 'gamerules.max_entity_cramming', field: { kind: 'int', min: 0, max: 9999 } },
  { key: 'gamerules.allow_entering_nether_using_portals', field: { kind: 'bool' } },
]

/** One editor row: whether the profile includes it, and its text value. */
export interface DraftEntry { include: boolean; value: string }
export interface ProfileDraft {
  name: string
  java: { include: boolean; automatic: boolean; path: string }
  jvm: { include: boolean; minMib: string; maxMib: string; gc: '' | 'g1' | 'parallel' | 'serial'; advanced: string }
  timing: { include: boolean; stopSeconds: string; readySeconds: string; restartDelayMs: string }
  autoRestart: { include: boolean; onCrash: boolean; onUnexpectedExit: boolean; delaySeconds: string; maxAttempts: string }
  fields: Record<string, DraftEntry>
}

function defaultValue(field: FieldKind) {
  return field.kind === 'bool' ? 'true' : field.kind === 'int' ? String(field.min) : field.kind === 'choice' ? field.choices[0] ?? '' : ''
}

export function draftFrom(profile?: SettingsProfile): ProfileDraft {
  const values = profile?.values ?? {}
  const fields: Record<string, DraftEntry> = {}
  for (const { key, field } of [...PROFILE_PROPERTIES, ...PROFILE_RULES]) {
    const saved = key.startsWith('gamerules.') ? values.gamerules?.[key] : values.serverProperties?.[key]
    fields[key] = { include: saved !== undefined, value: saved ?? defaultValue(field) }
  }
  const number = (value: number | null | undefined) => value == null ? '' : String(value)
  return {
    name: profile?.name ?? '',
    java: { include: values.java !== undefined, automatic: !values.java, path: values.java ?? '' },
    jvm: { include: !!values.jvm, minMib: number(values.jvm?.minMib), maxMib: number(values.jvm?.maxMib), gc: values.jvm?.gc ?? '', advanced: (values.jvm?.advanced ?? []).join('\n') },
    timing: { include: !!values.timing, stopSeconds: number(values.timing?.stopSeconds), readySeconds: number(values.timing?.readySeconds), restartDelayMs: number(values.timing?.restartDelayMs) },
    autoRestart: { include: !!values.autoRestart, onCrash: values.autoRestart?.onCrash ?? true, onUnexpectedExit: values.autoRestart?.onUnexpectedExit ?? false, delaySeconds: number(values.autoRestart?.delaySeconds ?? 10), maxAttempts: number(values.autoRestart?.maxAttempts ?? 3) },
    fields,
  }
}

function optionalNumber(text: string): number | null {
  const trimmed = text.trim()
  if (!trimmed) return null
  const value = Number(trimmed)
  if (!Number.isInteger(value) || value < 0) throw new Error('number')
  return value
}

/** The stored values of a draft; only included rows are kept. Throws `Error('number')` on a malformed number. */
export function valuesFrom(draft: ProfileDraft): SettingsProfileValues {
  const values: SettingsProfileValues = {}
  if (draft.java.include) values.java = draft.java.automatic ? '' : draft.java.path.trim()
  if (draft.jvm.include) {
    values.jvm = { minMib: optionalNumber(draft.jvm.minMib), maxMib: optionalNumber(draft.jvm.maxMib), gc: draft.jvm.gc || null }
    const advanced = draft.jvm.advanced.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
    if (advanced.length) values.jvm.advanced = advanced
  }
  if (draft.timing.include) values.timing = { stopSeconds: optionalNumber(draft.timing.stopSeconds), readySeconds: optionalNumber(draft.timing.readySeconds), restartDelayMs: optionalNumber(draft.timing.restartDelayMs) }
  if (draft.autoRestart.include) {
    values.autoRestart = { onCrash: draft.autoRestart.onCrash, onUnexpectedExit: draft.autoRestart.onUnexpectedExit, delaySeconds: optionalNumber(draft.autoRestart.delaySeconds) ?? 0, maxAttempts: optionalNumber(draft.autoRestart.maxAttempts) ?? 0 }
  }
  for (const [key, entry] of Object.entries(draft.fields)) {
    if (!entry.include) continue
    const target = key.startsWith('gamerules.') ? (values.gamerules ??= {}) : (values.serverProperties ??= {})
    target[key] = entry.value.trim()
  }
  return values
}

/** How many settings a profile carries. */
export function valueCount(values: SettingsProfileValues): number {
  return ['java', 'jvm', 'timing', 'autoRestart'].filter(key => values[key as keyof SettingsProfileValues] !== undefined).length
    + Object.keys(values.serverProperties ?? {}).length + Object.keys(values.gamerules ?? {}).length
}

const instanceLabels: Record<string, MessageId> = {
  java: 'profiles.field.java', jvm: 'profiles.field.jvm', timing: 'profiles.field.timing', autoRestart: 'profiles.field.autoRestart',
}
export function fieldLabel(key: string): string {
  if (Object.hasOwn(instanceLabels, key)) return tr(instanceLabels[key]!)
  const id = 'policy.field.' + key
  return isMessageId(id) ? tr(id) : key
}
export function reasonText(code: string): string {
  const own = 'profiles.reason.' + code, policy = 'policy.reason.' + code
  return isMessageId(own) ? tr(own) : isMessageId(policy) ? tr(policy) : code
}
const choiceLabels: Record<string, MessageId> = {
  true: 'components.policy.value.true', false: 'components.policy.value.false', survival: 'components.policy.value.survival', creative: 'components.policy.value.creative', adventure: 'components.policy.value.adventure',
  spectator: 'components.policy.value.spectator', peaceful: 'components.policy.value.peaceful', easy: 'components.policy.value.easy', normal: 'components.policy.value.normal', hard: 'components.policy.value.hard',
}
/** A short text for a before/after value in the preview. */
export function valueText(value: unknown): string {
  if (value === null || value === undefined || value === '') return tr('profiles.value.none')
  if (typeof value === 'string') return Object.hasOwn(choiceLabels, value) ? tr(choiceLabels[value]!) : value
  if (typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>).filter(([, item]) => item !== null && item !== undefined)
    return entries.map(([key, item]) => `${key}=${Array.isArray(item) ? item.join(' ') : String(item)}`).join(', ') || tr('profiles.value.none')
  }
  return String(value)
}

/** The activity row of one server in a settings-profile apply. */
export interface ProfileActivity { profile?: string; applied?: string[]; deferred?: string[]; skipped?: ProfileSkip[]; error?: string | null }
export function profileActivityMessage(row: ProfileActivity): string {
  const list = (keys: string[]) => keys.map(fieldLabel).join(tr('profiles.listSeparator'))
  const parts = [tr('profiles.activity', { profile: row.profile ?? '' })]
  if (row.applied?.length) parts.push(tr('profiles.activityApplied', { fields: list(row.applied) }))
  if (row.deferred?.length) parts.push(tr('profiles.activityDeferred', { fields: list(row.deferred) }))
  if (row.skipped?.length) parts.push(tr('profiles.activitySkipped', { fields: list(row.skipped.map(skip => skip.key)) }))
  if (row.error) parts.push(row.error)
  return parts.join(' · ')
}
