import type { MessageId } from './i18n.ts'

/** Why a registration could not be read, or why a registered instance's state could not be read. */
export type IssueReason = 'FileInUse' | 'AccessDenied' | 'InvalidRecord' | 'Unconfirmed' | 'ObservationUnconfirmed'
  | 'PathRedirected' | 'InvalidStateFile'
export interface StateIssue { reason: IssueReason; code?: string; osError: number | null; access?: { stage: string; file: string } | null }
export interface RegistryIssue extends StateIssue { file: string; name?: string; stage?: 'registration' | 'observation' }
export interface IssueMessage { id: MessageId; values?: Record<string, string> }

const registration: Partial<Record<IssueReason, MessageId>> = {
  FileInUse: 'app.registry.fileInUse',
  AccessDenied: 'app.registry.accessDenied',
  InvalidRecord: 'app.registry.invalidRecord',
}
const observation: Partial<Record<IssueReason, MessageId>> = {
  FileInUse: 'app.registry.state.fileInUse',
  AccessDenied: 'app.registry.state.accessDenied',
  PathRedirected: 'app.registry.state.pathRedirected',
  InvalidStateFile: 'app.registry.state.invalidStateFile',
}
const short: Partial<Record<IssueReason, MessageId>> = {
  FileInUse: 'instances.stateIssue.fileInUse',
  AccessDenied: 'instances.stateIssue.accessDenied',
  PathRedirected: 'instances.stateIssue.pathRedirected',
  InvalidStateFile: 'instances.stateIssue.invalidStateFile',
}

function code(value?: string): string | undefined {
  return typeof value === 'string' && /^[A-Za-z][A-Za-z0-9_]{0,63}$/.test(value) ? value : undefined
}

/** The banner text for one issue. Registrations written by 1.0.1 carry no stage. */
export function registryIssueMessage(issue: RegistryIssue): IssueMessage {
  if (issue.stage === 'observation' || issue.reason === 'ObservationUnconfirmed') {
    const id = observation[issue.reason]
    if (id) return { id }
    const known = code(issue.code)
    return known ? { id: 'app.registry.state.unconfirmedCode', values: { code: known } } : { id: 'app.registry.observationUnconfirmed' }
  }
  return { id: registration[issue.reason] ?? 'app.registry.unconfirmed' }
}

/** The short reason shown under a list row whose state could not be read. */
export function stateIssueMessage(issue: StateIssue): IssueMessage {
  const id = short[issue.reason]
  if (id) return { id }
  const known = code(issue.code)
  return known ? { id: 'instances.stateIssue.code', values: { code: known } } : { id: 'instances.stateIssue.unconfirmed' }
}

/** Shareable diagnostics: only names, stable codes and OS error numbers, never paths. */
export function registryDiagnosticText(issues: RegistryIssue[]): string {
  const name = (value?: string) => typeof value === 'string' && /^[a-z0-9][a-z0-9-]{0,63}$/.test(value) ? value : undefined
  const file = (value?: string) => typeof value === 'string' && /^[a-z0-9][a-z0-9-]{0,63}\.json$/.test(value) ? value : undefined
  const word = (value?: string) => typeof value === 'string' && /^[a-z][a-z-]{0,31}$/.test(value) ? value : undefined
  return JSON.stringify({
    schemaVersion: 1,
    issues: issues.slice(0, 256).map(issue => ({
      file: file(issue.file),
      name: name(issue.name),
      stage: issue.stage === 'registration' || issue.stage === 'observation' ? issue.stage : undefined,
      reason: code(issue.reason),
      code: code(issue.code),
      osError: Number.isInteger(issue.osError) ? issue.osError : undefined,
      access: issue.access ? { stage: word(issue.access.stage), file: word(issue.access.file) } : undefined,
    })),
  }, null, 2)
}
