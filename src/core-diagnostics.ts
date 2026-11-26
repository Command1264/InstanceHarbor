import type { MessageId } from './i18n.ts'
import { isInstanceName } from './instance-name.ts'

type Profile = { family: string; minecraft: string; build: string }
type Diagnostics = {
  instance: string
  installed?: Profile | null
  installedSha256?: string | null
  candidate?: { build: string; sha256?: string; md5?: string }
  checkedAt?: number
  pendingRequest?: string
  lastRequest?: string
  backupIds?: string[]
}

function token(value?: string): string | undefined {
  return typeof value === 'string' && /^[a-z0-9][a-z0-9-]{0,63}$/.test(value) ? value : undefined
}
function profilePart(value?: string): string | undefined {
  return typeof value === 'string' && /^[a-z0-9][a-z0-9._+-]{0,31}$/i.test(value) ? value : undefined
}
function digest(value?: string | null): string | undefined {
  return typeof value === 'string' && /^[a-f0-9]{64}$/i.test(value) ? value.toLowerCase() : undefined
}
function md5(value?: string | null): string | undefined {
  return typeof value === 'string' && /^[a-f0-9]{32}$/i.test(value) ? value.toLowerCase() : undefined
}

export function coreDiagnosticText(value: Diagnostics): string {
  const checkedAt = typeof value.checkedAt === 'number'
    ? new Date(value.checkedAt)
    : undefined
  return JSON.stringify({
    schemaVersion: 1,
    instance: isInstanceName(value.instance) ? value.instance : undefined,
    installed: value.installed ? {
      family: profilePart(value.installed.family),
      minecraft: profilePart(value.installed.minecraft),
      build: profilePart(value.installed.build),
      sha256: digest(value.installedSha256),
    } : undefined,
    candidate: value.candidate ? {
      build: profilePart(value.candidate.build),
      sha256: digest(value.candidate.sha256),
      md5: md5(value.candidate.md5),
    } : undefined,
    checkedAt: checkedAt && Number.isFinite(checkedAt.getTime()) ? checkedAt.toISOString() : undefined,
    pendingRequest: token(value.pendingRequest),
    lastRequest: token(value.lastRequest),
    backupIds: value.backupIds?.slice(0, 64).flatMap(id => token(id) ?? []),
  }, null, 2)
}

const coreBlocked: Record<string, MessageId> = {
  CoreMaintenancePending: 'manager.error.coreMaintenancePending',
  CoreProfileUnverified: 'components.coreMaintenance.blocked.profileUnverified',
  InstanceNotConfirmedStopped: 'manager.error.instanceNotConfirmedStopped',
  CoreStopEvidenceMissing: 'components.coreMaintenance.blocked.stopEvidenceMissing',
  PortChanged: 'components.coreMaintenance.blocked.portChanged',
  PortOccupied: 'components.coreMaintenance.blocked.portOccupied',
  PortOccupiedOrUnavailable: 'components.coreMaintenance.blocked.portOccupied',
}

/** Why `core-state` does not allow a core update, when the reason has dedicated text. */
export function coreBlockedMessage(reason?: string | null): MessageId | undefined {
  const known = typeof reason === 'string' ? reason.split(/[\s:([]/)[0] : ''
  return Object.hasOwn(coreBlocked, known) ? coreBlocked[known] : undefined
}
