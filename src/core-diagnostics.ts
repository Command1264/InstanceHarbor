import { isInstanceName } from './instance-name.ts'

type Profile = { family: string; minecraft: string; build: string }
type Diagnostics = {
  instance: string
  installed?: Profile | null
  installedSha256?: string | null
  candidate?: { build: string; sha256: string }
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
    } : undefined,
    checkedAt: checkedAt && Number.isFinite(checkedAt.getTime()) ? checkedAt.toISOString() : undefined,
    pendingRequest: token(value.pendingRequest),
    lastRequest: token(value.lastRequest),
    backupIds: value.backupIds?.slice(0, 64).flatMap(id => token(id) ?? []),
  }, null, 2)
}
