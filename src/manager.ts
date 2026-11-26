import { Channel, invoke } from '@tauri-apps/api/core'
import { tr, type LanguagePreference, type MessageId, type TimeFormat } from './i18n'
import type { SelectionColorPreference } from './theme'
import type { ConsoleChoice } from './console-tier.ts'
import { javaStartErrorId } from './java-start-error.ts'
import type { CoreUpdateProgress } from './core-progress.ts'
import type { RegistryIssue, StateIssue } from './registry-issues.ts'
export { consoleFallbackReason, consoleSummary, type ConsoleChoice, type ConsolePlan } from './console-tier.ts'

export interface Instance {
  name: string
  port: number
  state: string
  ownerPid: number | null
  ownerName?: string
  listenAddress?: string | null
  consoleReady?: boolean
  nativeConsole?: boolean
  console?: ConsoleChoice | null
  outsideManagement?: boolean
  autoRestartStatus?: AutoRestartStatus | null
  stopPhase?: string
  stopToken: string | null
  stateIssue?: StateIssue
  run?: { id: string; java: { pid: number }; host: { pid: number } }
}
export interface CoreIdentity { state: 'known' | 'unknown'; family?: string; minecraft?: string; build?: string; label?: string; reason?: string; unverified?: boolean; offline?: boolean; /** #62: instances naming the same shared core file. */ sharedUsers?: number }
export interface InstanceMeta { name: string; java: string; port: number; path: string; core?: CoreIdentity; managementDiscovered?: boolean }
export interface Preferences { theme: 'system' | 'light' | 'dark'; pins: string[]; selectionColor?: SelectionColorPreference; selectionOpacity?: number; language?: LanguagePreference; pathSeparator?: 'forward' | 'native'; timeFormat?: TimeFormat; consoleTitleTemplate?: string; consoleTitleOverrides?: Record<string,string>; corePrefetch?: boolean }
export interface PreferencesSnapshot { preferences: Preferences; revision: string }
export type { RegistryIssue } from './registry-issues.ts'
export interface InstanceSnapshot extends Omit<Presentation, 'preferences'> { rows: Instance[]; issues: RegistryIssue[] }
export interface Presentation {
  preferences: Preferences
  instances: InstanceMeta[]
  capabilities?: Record<string, unknown>
}
export interface BatchItem {
  name: string
  phase: 'Queued' | 'Dispatching' | 'Starting' | 'Completed' | 'Cancelled' | 'Failed' | 'Unconfirmed'
    | 'StopQueued' | 'StopDispatching' | 'Stopping' | 'StopUnconfirmed' | 'Skipped' | 'Sending' | 'Preparing'
  reason: string | null
  runId: string | null
  canCancel: boolean
}
export function batchItemState(phase: BatchItem['phase']): string {
  if (['Completed', 'Cancelled', 'Failed', 'Skipped', 'Unconfirmed'].includes(phase)) return phase
  return phase === 'StopUnconfirmed' ? 'Unconfirmed' : 'Pending'
}
export interface Job { id: string; action: string; instance: string; state: string; message?: string; createdAt?: number | null; items?: BatchItem[] }
export type CoreBatchPhase = 'Queued' | 'Preparing' | 'Sending' | 'Completed' | 'Cancelled' | 'Failed' | 'Unconfirmed'
export interface CoreBatch {
  id: string
  action: 'core-batch-install'
  createdAt: number
  state: 'Running' | 'Completed' | 'CompletedWithIssues'
  items: { name: string; phase: CoreBatchPhase; reason: string | null; progress?: CoreUpdateProgress; reused?: boolean }[]
}
export function canCancelCoreBatchItem(item: CoreBatch['items'][number]): boolean {
  return item.phase === 'Queued' || item.phase === 'Preparing'
}
export function coreBatchJob(batch: CoreBatch): Job {
  return {...batch, instance: '', items: batch.items.map(item=>({...item,runId:null,canCancel:canCancelCoreBatchItem(item)}))}
}
export interface RemovedInstance { name: string; port: number; path: string | null; folderPresent: boolean; java: string | null; managementDiscovered: boolean }
export interface JavaCandidate { path: string; version: string | null; valid: boolean; source: string; architecture?: string | null; vendor?: string | null }
export interface Jvm { minMib: number | null; maxMib: number | null; gc: 'g1' | 'parallel' | 'serial' | null; advanced?: string[] }
export interface LifecycleTiming { stopSeconds: number | null; readySeconds: number | null; restartDelayMs: number | null }
export interface AutoRestart {
  onCrash: boolean
  onUnexpectedExit: boolean
  delaySeconds: number
  maxAttempts: number
  retryDelaysSeconds?: number[]
  windowSeconds?: number
  stableSeconds?: number
}
export interface AutoRestartStatus {
  phase: 'Disabled' | 'Scheduled' | 'Watching' | 'Paused' | 'Suppressed' | 'Unconfirmed'
  attemptCount?: number
  dueAt?: number | null
}
export interface WorldLiveReceipt {
  request: string
  name: string
  state: 'NoChange' | 'Saved' | 'AwaitingNormalStop' | 'Unconfirmed' | 'Sending'
  observed: string | null
  error: string | null
}
export interface InstanceSettings { name: string; java: string; canSelectJavaAutomatically?: boolean; port: number; jvm?: Jvm | null; timing?: LifecycleTiming | null; autoRestart?: AutoRestart | null; revision: string; pending?: {state: 'Queued' | 'Applying' | 'Conflict'; values: Partial<InstanceSettings>}; queued?: boolean; applied?: boolean }
export interface Logs {
  run?: string | null;
  text: string | null; path: string; consoleReady?: boolean; merged?: boolean;
  sources?: { source: string; state: string; path: string; text: string | null }[]
}

export const nativeBridge = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
export type CreationProgress =
  | { phase: 'checking' | 'downloadingCore' | 'downloadingMojang' | 'creating' }
  | { phase: 'coreBytes' | 'mojangBytes'; received: number; total: number }

export async function confirmCreation(work: string, draft: string, onProgress: (update: CreationProgress) => void) {
  const progress = new Channel<CreationProgress>(onProgress)
  try {
    await invoke('create_with_progress', { work, draft, progress })
  } finally {
    progress.onmessage = () => {}
  }
}

/** A core install or restore that reports each step to `onProgress` (#60). */
export async function manageCoreWithProgress<T>(arguments_: string[], onProgress: (update: CoreUpdateProgress) => void): Promise<T> {
  if (!nativeBridge) throw new Error(tr('manager.error.noNativeBridge'))
  const progress = new Channel<CoreUpdateProgress>(onProgress)
  try {
    return await invoke<T>('core_maintenance_with_progress', { arguments: arguments_, progress })
  } finally {
    progress.onmessage = () => {}
  }
}

export function manage<T>(arguments_: string[],origin:'manual'|'background'='manual'): Promise<T> {
  if (!nativeBridge) return Promise.reject(new Error(tr('manager.error.noNativeBridge')))
  return invoke<T>('manage', { arguments: arguments_,origin })
}
const stateMessageIds: Record<string, MessageId> = {
  Stopped: 'instance.state.stopped', Listening: 'instance.state.listening', Starting: 'instance.state.starting', Stopping: 'instance.state.stopping',
  PortOccupied: 'instance.state.portOccupied', RunUnconfirmed: 'instance.state.runUnconfirmed', StateUnconfirmed: 'instance.state.stateUnconfirmed',
}
export function stateName(state: string) { return Object.hasOwn(stateMessageIds, state) ? tr(stateMessageIds[state]) : state }
const actionMessageIds: Record<string, MessageId> = {
  create: 'manager.action.create',
  import: 'manager.action.import',
  'import-batch': 'manager.action.importBatch',
  start: 'manager.action.start',
  'batch-start': 'manager.action.batchStart',
  'batch-stop': 'manager.action.batchStop',
  'batch-restart': 'manager.action.batchRestart',
  'start-if-current': 'manager.action.start',
  stop: 'manager.action.stop',
  restart: 'manager.action.restart',
  'force-stop': 'manager.action.forceStop',
  'core-install': 'manager.action.coreInstall',
  'core-batch-install': 'manager.action.coreBatchInstall',
  'core-restore': 'manager.action.coreRestore',
  'core-reconcile': 'manager.action.coreReconcile',
  'remove-management': 'manager.action.removeManagement',
  'delete-server': 'manager.action.deleteServer',
  adopt: 'manager.action.adopt',
}
export function actionName(action: string) { return Object.hasOwn(actionMessageIds, action) ? tr(actionMessageIds[action]) : action }
export function canStop(row?: Instance) { return !!row?.run && ['Listening', 'Starting'].includes(row.state) }
export function canOpenConsole(row: Instance): row is Instance & {run: NonNullable<Instance['run']>} {
  return row.nativeConsole === true && canStop(row)
}
export function canRestart(row?: Instance) { return !row?.outsideManagement && (row?.state === 'Stopped' || canStop(row)) }
export function rawError(error: unknown) { return error instanceof Error ? error.message : String(error) }
export function errorText(error: unknown) {
  const raw = rawError(error)
  if (raw.includes('TestAdministratorNotFound')) return tr('manager.error.testAdminNotFound')
  if (raw.includes('TestAdministratorLookupRateLimited')) return tr('manager.error.testAdminLookupRateLimited')
  if (raw.includes('TestAdministratorLookupFailed')) return tr('manager.error.testAdminLookupFailed')
  if (raw.includes('InvalidTestAdministratorName')) return tr('manager.error.testAdminInvalidName')
  if (raw.includes('InvalidTestAdministratorUuid')) return tr('manager.error.testAdminInvalidUuid')
  if (raw.includes('DuplicateTestAdministrator')) return tr('manager.error.testAdminDuplicate')
  if (raw.includes('TooManyTestAdministratorPermissions')) return tr('manager.error.testAdminTooManyPermissions')
  if (raw.includes('TooManyTestAdministrators')) return tr('manager.error.testAdminTooMany')
  if (raw.includes('InvalidTestAdministratorGroup')) return tr('manager.error.testAdminInvalidGroup')
  if (raw.includes('InvalidTestAdministratorPermission')) return tr('manager.error.testAdminInvalidPermission')
  if (raw.includes('EmptyTestAdministratorPermissions')) return tr('manager.error.testAdminEmptyPermissions')
  if (raw.includes('RegistryConflictUnconfirmed')) return tr('manager.error.registryConflictUnconfirmed')
  if (raw.includes('InstanceSnapshotChanged')) return tr('manager.error.instanceSnapshotChanged')
  if (raw.startsWith('FileAccess[')) {
    const file = ['preferences.json', 'global-java.json', 'scan-policy.json'].find(name => raw.includes(name + ':'))
    if (file) {
      if (/os error (32|33)\)/.test(raw)) return tr('manager.error.fileInUse', { file })
      if (/os error 5\)/.test(raw)) return tr('manager.error.fileAccessDenied', { file })
      if (/os error (2|3)\)/.test(raw)) return tr('manager.error.fileNotFound', { file })
      return tr('manager.error.fileUnavailable', { file })
    }
  }
  if (/PreferencesInvalidJson|InvalidGlobalJavaSettings: global-java|InvalidScanPolicy: scan-policy/.test(raw)) return tr('manager.error.preferencesInvalid')
  if (raw.includes('BatchTargetPending')) return tr('manager.error.batchTargetPending')
  if (raw.includes('BatchNeedsReconciliation')) return tr('manager.error.batchNeedsReconciliation')
  if (raw.includes('ManagerJournalUnavailable')) return tr('manager.error.managerJournalUnavailable')
  if (raw.includes('ManagerUnavailable') || raw.includes('ManagerPipeUnavailable')) return tr('manager.error.managerUnavailable')
  if (raw.includes('ManagerQueueFull')) return tr('manager.error.managerQueueFull')
  if (raw.includes('InvalidBatchTargets')) return tr('manager.error.invalidBatchTargets')
  if (raw.includes('ManagerBusy')) return tr('manager.error.managerBusy')
  if (raw.includes('StopTimeout') || raw.includes('HostUnavailable')) return tr('manager.error.stopTimeout')
  if (raw.includes('DeleteServerRecycledButRegistered')) return tr('manager.error.deleteServerRecycledButRegistered')
  if (raw.includes('DeleteServerRecycledButPathListed')) return tr('manager.error.deleteServerRecycledButPathListed')
  if (raw.includes('DeleteServerNameMismatch')) return tr('manager.error.deleteServerNameMismatch')
  if (raw.includes('DeleteServerProtectedFolder')) return tr('manager.error.deleteServerProtectedFolder')
  if (raw.includes('DeleteServerNetworkPath')) return tr('manager.error.deleteServerNetworkPath')
  if (raw.includes('DeleteServerContainsLink')) return tr('manager.error.deleteServerContainsLink')
  if (raw.includes('DeleteServerContainsOtherInstance')) return tr('manager.error.deleteServerContainsOtherInstance')
  if (raw.includes('DeleteServerOtherInstanceUnconfirmed')) return tr('manager.error.deleteServerOtherInstanceUnconfirmed')
  if (raw.includes('DeleteServerCancelled')) return tr('manager.error.deleteServerCancelled')
  if (raw.includes('DeleteServerIncomplete')) return tr('manager.error.deleteServerIncomplete')
  if (raw.includes('DeleteServerRecycleFailed')) return tr('manager.error.deleteServerRecycleFailed')
  if (raw.includes('ServerPortRegistryMismatch')) return tr('manager.error.serverPortRegistryMismatch')
  if (raw.includes('InstanceMustBeStopped')) return tr('manager.error.instanceMustBeStopped')
  if (raw.includes('WorldGenerationOnly')) return tr('manager.error.worldGenerationOnly')
  if (raw.includes('WorldSettingsReadbackMismatch')) return tr('manager.error.worldSettingsReadbackMismatch')
  if (raw.includes('SettingsConflict')) return tr('manager.error.settingsConflict')
  if (raw.includes('ResourcePackSha1Required')) return tr('manager.error.resourcePackSha1Required')
  if (raw.includes('ResourcePackUrlRequired')) return tr('manager.error.resourcePackUrlRequired')
  if (raw.includes('ResourcePackGroupUnconfirmed') || raw.includes('RichResourcePackPrompt')) return tr('manager.error.resourcePackGroupUnconfirmed')
  if (raw.includes('InvalidResourcePack')) return tr('manager.error.invalidResourcePack')
  if (raw.includes('WorldSettingsMultipleFiles')) return tr('manager.error.worldSettingsMultipleFiles')
  if (raw.includes('WorldPreviewRequired') || raw.includes('WorldPreviewChanged')) return tr('manager.error.worldPreviewRequired')
  if (raw.includes('WorldPreviewExpired')) return tr('manager.error.worldPreviewExpired')
  if (raw.includes('WorldRunChanged') || raw.includes('WorldRunNotReady')) return tr('manager.error.worldRunChanged')
  if (raw.includes('WorldConsoleUnavailable')) return tr('manager.error.worldConsoleUnavailable')
  if (raw.includes('WorldConsoleInputNotVerified')) return tr('manager.error.worldConsoleInputNotVerified')
  if (raw.includes('WorldConsoleInputBusy')) return tr('console.inputBusy')
  if (/World(Reply|Outcome|Command)Unconfirmed/.test(raw)) return tr('manager.error.worldOutcomeUnconfirmed')
  if (raw.includes('InvalidScanPath')) return tr('manager.error.invalidScanPath')
  if (raw.includes('ScanWouldHideActiveInstance')) return tr('manager.error.scanWouldHideActiveInstance')
  if (raw.includes('ImportTreeTooLarge')) return tr('manager.error.importTreeTooLarge')
  if (raw.includes('ScanPolicyTooLarge')) return tr('manager.error.scanPolicyTooLarge')
  if (raw.includes('ManagementScopePreviewChanged')) return tr('manager.error.managementScopePreviewChanged')
  if (raw.includes('BatchImportDirectoryOutsideScope')) return tr('manager.error.batchImportDirectoryOutsideScope')
  if (raw.includes('DirectoryRedirected')) return tr('manager.error.directoryRedirected')
  if (raw.includes('ImportOutsideInstanceRoot')) return tr('manager.error.importOutsideInstanceRoot')
  if (raw.includes('ImportPreviewChanged')) return tr('manager.error.importPreviewChanged')
  if (raw.includes('CoreInUse')) return tr('manager.error.coreInUse')
  if (raw.includes('CoreCatalogUnavailable')) return tr('manager.error.coreCatalogUnavailable')
  if (raw.includes('CoreArtifactAmbiguous')) return tr('manager.error.coreArtifactAmbiguous')
  if (raw.includes('UnverifiedVelocityCore')) return tr('manager.error.unverifiedVelocityCore')
  if (raw.includes('ExpectedLoopbackVelocityAddress')) return tr('manager.error.expectedLoopbackVelocityAddress')
  if (raw.includes('InvalidVelocityBind')) return tr('manager.error.invalidVelocityBind')
  if (raw.includes('ImportPathAlreadyRegistered') || raw.includes('ImportRegistrationPathChanged')) return tr('manager.error.importPathAlreadyRegistered')
  if (raw.includes('InstanceDirectoryInUse')) return tr('manager.error.instanceDirectoryInUse')
  if (raw.includes('ImportManagementUnconfirmed')) return tr('manager.error.importManagementUnconfirmed')
  if (raw.includes('PortAlreadyRegistered')) return tr('manager.error.portAlreadyRegistered')
  if (raw.includes('PortOccupied')) return tr('manager.error.portOccupied')
  const javaStart = javaStartErrorId(raw)
  if (javaStart) return tr(javaStart)
  if (raw.includes('JavaShimNotAllowed')) return tr('manager.error.javaShimNotAllowed')
  if (raw.includes('InvalidLifecycleTiming')) return tr('manager.error.invalidLifecycleTiming')
  if (raw.includes('UnverifiedCreationCore')) return tr('manager.error.unverifiedCreationCore')
  if (raw.includes('InvalidLocalCreationCore')) return tr('manager.error.invalidLocalCreationCore')
  if (raw.includes('UnsupportedCreationCore')) return tr('manager.error.unsupportedCreationCore')
  if (raw.includes('CreationPreviewChanged')) return tr('manager.error.creationPreviewChanged')
  if (raw.includes('CreationPathExcluded')) return tr('manager.error.creationPathExcluded')
  if (raw.includes('InstanceExists')) return tr('manager.error.instanceExists')
  if (raw.includes('CreationPolicyChanged') || raw.includes('CreationRecoveryUnconfirmed')) return tr('manager.error.creationPolicyChanged')
  if (raw.includes('CreationRecoveryPending')) return tr('manager.error.creationRecoveryPending')
  if (raw.includes('ScanDirectoryUnavailable')) return tr('manager.error.scanDirectoryUnavailable')
  if (raw.includes('MissingMojangArtifact') || raw.includes('MojangArtifactMismatch')) return tr('manager.error.missingMojangArtifact')
  if (raw.includes('UnexpectedMojangArtifact')) return tr('manager.error.unexpectedMojangArtifact')
  if (raw.includes('CoreSourceMissingContact') || raw.includes('CoreSourceInvalidContact')) return tr('manager.error.coreSourceMissingContact')
  if (raw.includes('CoreCandidateUnverified')) return tr('manager.error.coreCandidateUnverified')
  if (raw.includes('CoreCandidateBusy')) return tr('manager.error.coreCandidateBusy')
  if (raw.includes('CoreMaintenancePending')) return tr('manager.error.coreMaintenancePending')
  if (raw.includes('CoreArtifactChanged')) return tr('manager.error.coreArtifactChanged')
  if (raw.includes('InvalidAutoRestartPolicy')) return tr('manager.error.invalidAutoRestartPolicy')
  if (raw.includes('InvalidJvmOption')) return tr('manager.error.invalidJvmOption')
  if (raw.includes('JvmProbeFailed')) return tr('manager.error.jvmProbeFailed')
  if (raw.includes('InvalidJvmMemory')) return tr('manager.error.invalidJvmMemory')
  if (raw.includes('InvalidJavaExecutable')) return tr('manager.error.invalidJavaExecutable')
  if (raw.includes('InvalidJavaScanDirectory')) return tr('manager.error.invalidJavaScanDirectory')
  if (raw.includes('JavaScanDirectoryConflict')) return tr('manager.error.javaScanDirectoryConflict')
  if (raw.includes('AutoJavaUnavailable')) return tr('manager.error.autoJavaUnavailable')
  if (raw.includes('GlobalJavaConflict')) return tr('manager.error.globalJavaConflict')
  if (raw.includes('CreationDefaultsConflict')) return tr('manager.error.creationDefaultsConflict')
  if (raw.includes('InvalidCreationDefaults')) return tr('manager.error.invalidCreationDefaults')
  if (raw.includes('GlobalJavaIncompatible')) return tr('manager.error.globalJavaIncompatible')
  if (raw.includes('GlobalJavaUnavailable') || raw.includes('InvalidJavaPath')) return tr('manager.error.globalJavaUnavailable')
  if (raw.includes('JavaEnvironmentOverride')) return tr('manager.error.javaEnvironmentOverride')
  if (raw.includes('JavaProbe')) return tr('manager.error.javaProbe')
  if (raw.includes('InstanceNotConfirmedStopped')) return tr('manager.error.instanceNotConfirmedStopped')
  if (raw.includes('ResultUnconfirmed') || raw.includes('StopResultUnconfirmed')) return tr('manager.error.resultUnconfirmed')
  if (raw.includes('RunUnconfirmed')) return tr('manager.error.runUnconfirmed')
  if (raw.includes('StartIntentChanged')) return tr('manager.error.startIntentChanged')
  if (raw.includes('AlreadyManaged')) return tr('manager.error.alreadyManaged')
  if (raw.includes('InvalidServerPort')) return tr('manager.error.invalidServerPort')
  if (raw.includes('RequestConflict')) return tr('manager.error.requestConflict')
  if (raw.includes('ExpectedLoopbackServerAddress')) return tr('manager.error.expectedLoopbackServerAddress')
  if (/os error 5\b/i.test(raw)) return tr('manager.error.osAccessDenied')
  if (/os error (2|3)\b/i.test(raw)) return tr('manager.error.osNotFound')
  return tr('manager.error.fallback')
}
export type CoreSource = {
  name: string
  status: string
  family?: string
  minecraft?: string
  build?: string
  sha256?: string
  registrationRevision?: string
  remoteBuild?: string
  /** A Purpur candidate has only the official MD5 until it is downloaded (#41). */
  candidate?: { build: string; sha256?: string; md5?: string }
  /** The core file name after this update, when it follows the build (#112). */
  renameTo?: string | null
  /** `purpurMd5` when the source publishes MD5 only. */
  verification?: string
  reason?: string
  checkedAt?: number
  /** Spigot (#75): the Jenkins build the JAR states, when it states one. */
  localBuild?: string | null
  /** Spigot (#75): whether `latest.json` is this version line, by its build. */
  latestMinecraft?: { status: 'Same' | 'Newer' | 'Unknown'; build?: string; reason?: string }
}
