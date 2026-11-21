import { Channel, invoke } from '@tauri-apps/api/core'
import type { LanguagePreference, TimeFormat } from './i18n'
import type { SelectionColorPreference } from './theme'

export interface Instance {
  name: string
  port: number
  state: string
  ownerPid: number | null
  ownerName?: string
  listenAddress?: string | null
  consoleReady?: boolean
  nativeConsole?: boolean
  outsideManagement?: boolean
  autoRestartStatus?: AutoRestartStatus | null
  stopPhase?: string
  stopToken: string | null
  run?: { id: string; java: { pid: number }; host: { pid: number } }
}
export interface CoreIdentity { state: 'known' | 'unknown'; family?: string; minecraft?: string; build?: string; label?: string; reason?: string }
export interface InstanceMeta { name: string; java: string; port: number; path: string; core?: CoreIdentity }
export interface Preferences { theme: 'system' | 'light' | 'dark'; pins: string[]; selectionColor?: SelectionColorPreference; selectionOpacity?: number; language?: LanguagePreference; pathSeparator?: 'forward' | 'native'; timeFormat?: TimeFormat; consoleTitleTemplate?: string; consoleTitleOverrides?: Record<string,string>; corePrefetch?: boolean }
export interface PreferencesSnapshot { preferences: Preferences; revision: string }
export interface RegistryIssue { file: string; reason: 'FileInUse' | 'AccessDenied' | 'InvalidRecord' | 'Unconfirmed' | 'ObservationUnconfirmed'; osError: number | null }
export interface InstanceSnapshot extends Omit<Presentation, 'preferences'> { rows: Instance[]; issues: RegistryIssue[] }
export interface Presentation {
  preferences: Preferences
  instances: InstanceMeta[]
  capabilities?: Record<string, boolean>
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
  items: { name: string; phase: CoreBatchPhase; reason: string | null }[]
}
export function canCancelCoreBatchItem(item: CoreBatch['items'][number]): boolean {
  return item.phase === 'Queued' || item.phase === 'Preparing'
}
export function coreBatchJob(batch: CoreBatch): Job {
  return {...batch, instance: '', items: batch.items.map(item=>({...item,runId:null,canCancel:canCancelCoreBatchItem(item)}))}
}
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

export function manage<T>(arguments_: string[],origin:'manual'|'background'='manual'): Promise<T> {
  if (!nativeBridge) return Promise.reject(new Error('此瀏覽器沒有原生管理橋接。請使用 InstanceHarbor 桌面程式。'))
  return invoke<T>('manage', { arguments: arguments_,origin })
}
export const stateNames: Record<string, string> = {
  Stopped: '已停止', Listening: '正在監聽', Starting: '啟動中',
  PortOccupied: 'Port 已被占用', RunUnconfirmed: '工作階段未確認',
}
export function stateName(state: string) { return stateNames[state] ?? state }
const actionNames:Record<string,string>={create:'建立',import:'匯入','import-batch':'批次匯入',start:'啟動','batch-start':'批次啟動','batch-stop':'批次正常停止','batch-restart':'批次重新啟動','start-if-current':'啟動',stop:'正常停止',restart:'重新啟動','force-stop':'強制停止','core-install':'更新核心','core-batch-install':'批次核心更新','core-restore':'還原核心','core-reconcile':'核對核心維護','remove-management':'移除管理',adopt:'重新接入'}
export function actionName(action:string){return actionNames[action]??action}
export function canStop(row?: Instance) { return !!row?.run && ['Listening', 'Starting'].includes(row.state) }
export function canOpenConsole(row: Instance): row is Instance & {run: NonNullable<Instance['run']>} {
  return row.nativeConsole === true && canStop(row)
}
export function canRestart(row?: Instance) { return !row?.outsideManagement && (row?.state === 'Stopped' || canStop(row)) }
export function rawError(error: unknown) { return error instanceof Error ? error.message : String(error) }
export function errorText(error: unknown) {
  const raw = rawError(error)
  if (raw.includes('RegistryConflictUnconfirmed')) return '有實例登記無法讀取，無法排除名稱、資料夾或 Port 衝突。請修復登記檔後重新核對。'
  if (raw.includes('InstanceSnapshotChanged')) return '實例登記正在變更。請稍候重新核對。'
  if (raw.startsWith('FileAccess[')) {
    const file = ['preferences.json', 'global-java.json', 'scan-policy.json'].find(name => raw.includes(name + ':'))
    if (file) {
      if (/os error (32|33)\)/.test(raw)) return file + ' 正被其他程序占用。請釋放檔案後再試。'
      if (/os error 5\)/.test(raw)) return 'Windows 拒絕存取 ' + file + '。請核對檔案權限後再試。'
      if (/os error (2|3)\)/.test(raw)) return '找不到 ' + file + '。請核對工作區資料。'
      return '無法存取 ' + file + '。請核對應用程式診斷紀錄後再試。'
    }
  }
  if (/PreferencesInvalidJson|InvalidGlobalJavaSettings: global-java|InvalidScanPolicy: scan-policy/.test(raw)) return '已儲存的設定檔內容無法讀取。請修正或還原原檔後重新讀取。'
  if (raw.includes('BatchTargetPending')) return '此實例已有等待或執行中的操作，請查看操作紀錄。'
  if (raw.includes('BatchNeedsReconciliation')) return '先前的批次結果未確認，管理服務已暫停新的批次啟動。'
  if (raw.includes('ManagerJournalUnavailable')) return '無法讀寫批次工作紀錄，請保留日誌並核對工作區。'
  if (raw.includes('ManagerUnavailable') || raw.includes('ManagerPipeUnavailable')) return '批次管理服務暫時無法連線，請稍後重新核對。'
  if (raw.includes('ManagerQueueFull')) return '待處理的批次工作已達上限，請等待目前工作完成。'
  if (raw.includes('InvalidBatchTargets')) return '每批須包含 1 到 64 台不同實例，請調整選取範圍。'
  if (raw.includes('ManagerBusy')) return '管理服務正在處理其他操作，請稍後重試。'
  if (raw.includes('StopTimeout') || raw.includes('HostUnavailable')) return '停止結果尚未確認。請核對工作階段後再試。'
  if (raw.includes('InstanceMustBeStopped')) return '請先正常停止伺服器，再修改設定。'
  if (raw.includes('WorldGenerationOnly')) return '世界生成設定只能在建立新世界時套用。'
  if (raw.includes('WorldSettingsReadbackMismatch')) return '儲存後讀回的設定與預期不同。請核對檔案和既有備份。'
  if (raw.includes('SettingsConflict')) return '設定已由其他操作變更。請重新讀取並核對草稿後再試。'
  if (raw.includes('ResourcePackSha1Required')) return '請同時提供資源包網址和 40 位十六進位 SHA-1。'
  if (raw.includes('ResourcePackUrlRequired')) return '請先設定資源包網址，或清除其他資源包設定。'
  if (raw.includes('ResourcePackGroupUnconfirmed') || raw.includes('RichResourcePackPrompt')) return '無法核對現有資源包設定，請檢查伺服器設定檔。'
  if (raw.includes('InvalidResourcePack')) return '請檢查資源包網址、UUID、提示與是否要求使用。'
  if (raw.includes('WorldSettingsMultipleFiles')) return '請分別套用各設定檔的變更。這次的草稿仍然保留。'
  if (raw.includes('WorldPreviewRequired') || raw.includes('WorldPreviewChanged')) return '請重新核對設定差異，再套用變更。'
  if (raw.includes('WorldPreviewExpired')) return '這次差異已逾時，請重新核對後再套用。'
  if (raw.includes('WorldRunChanged') || raw.includes('WorldRunNotReady')) return '目前工作階段已變更或尚未就緒，請重新核對。'
  if (raw.includes('WorldConsoleUnavailable')) return '此工作階段的命令通道尚未支援世界規則套用。'
  if (raw.includes('WorldConsoleInputNotVerified')) return '無法確認後台輸入狀態。請核對後台與工作階段，再重新檢查。'
  if (/World(Reply|Outcome|Command)Unconfirmed/.test(raw)) return '無法確認這次操作的結果。請先核對工作紀錄。'
  if (raw.includes('InvalidScanPath')) return '請輸入本機完整目錄，或以工作區為起點的相對目錄，不可使用上層路徑。'
  if (raw.includes('ScanWouldHideActiveInstance')) return '規則會隱藏尚未確認停止的實例。請先確認停止，或調整排除範圍。'
  if (raw.includes('ImportTreeTooLarge')) return '目錄項目超過本次核對上限，請縮小匯入來源。'
  if (raw.includes('ScanPolicyTooLarge')) return '掃描規則超過容量上限，請減少規則後再試。'
  if (raw.includes('ManagementScopePreviewChanged')) return '管理範圍或掃描結果已變更。請重新按「儲存設定」核對。'
  if (raw.includes('BatchImportDirectoryOutsideScope')) return '請選擇目前工作區 instances 或其下的資料夾。'
  if (raw.includes('DirectoryRedirected')) return '指定目錄經過連結或重新導向，請改選實際目錄。'
  if (raw.includes('ImportOutsideInstanceRoot')) return '請選擇目前工作區 instances 下的伺服器目錄。'
  if (raw.includes('ImportPreviewChanged')) return '伺服器的匯入來源已變更，請重新核對。'
  if (raw.includes('UnverifiedVelocityCore')) return '無法核對 Velocity 核心，請選擇已驗證的 Velocity JAR。'
  if (raw.includes('ExpectedLoopbackVelocityAddress')) return '請核對 velocity.toml 的 bind，使用 127.0.0.1 與有效的 Port。'
  if (raw.includes('InvalidVelocityBind')) return '請核對 velocity.toml 的 bind，使用有效的 IPv4 位址與 Port。'
  if (raw.includes('ImportPathAlreadyRegistered') || raw.includes('ImportRegistrationPathChanged')) return '此路徑已登記或與保留的管理設定不符，請核對名稱及目錄。'
  if (raw.includes('InstanceDirectoryInUse')) return '這個資料夾已有執行中的宿主，請先核對該工作階段。'
  if (raw.includes('ImportManagementUnconfirmed')) return '目錄可能仍由其他方式管理，或有世界檔案正在使用。請先核對停止狀態及來源。'
  if (raw.includes('PortAlreadyRegistered')) return '這個 Port 已登記給其他實例。請選擇未使用的 Port。'
  if (raw.includes('PortOccupied')) return 'Port 已被占用。請選擇其他 Port。'
  if (raw.includes('JavaShimNotAllowed')) return '請選擇實際的 java.exe，不能使用轉接腳本或 shim。'
  if (raw.includes('InvalidLifecycleTiming')) return '請核對停止、就緒期限及重啟緩衝的數值範圍。'
  if (raw.includes('UnverifiedCreationCore')) return '目前無法核對這份核心 JAR，請選擇已驗證的版本。'
  if (raw.includes('InvalidLocalCreationCore')) return '無法讀取本機核心。請選擇可存取的 JAR 檔案。'
  if (raw.includes('UnsupportedCreationCore')) return '這個核心不支援建立實例。請選擇支援的伺服器核心。'
  if (raw.includes('CreationPreviewChanged')) return '建立條件已變更。請重新預覽建立路徑與管理範圍。'
  if (raw.includes('CreationPathExcluded')) return '目標路徑受到排除規則限制。請選擇其他資料夾，或先調整管理範圍。'
  if (raw.includes('InstanceExists')) return '實例名稱或目標資料夾已存在。請使用其他名稱或資料夾。'
  if (raw.includes('CreationPolicyChanged') || raw.includes('CreationRecoveryUnconfirmed')) return '無法確認建立結果。請保留這次預覽及工作 ID，再按「建立」核對原工作。'
  if (raw.includes('CreationRecoveryPending')) return '此實例已有尚待復原的建立工作。請查看操作紀錄，並核對原工作。'
  if (raw.includes('ScanDirectoryUnavailable')) return '找不到可使用的上層資料夾。請核對路徑與存取權限。'
  if (raw.includes('MissingMojangArtifact') || raw.includes('MojangArtifactMismatch')) return '請選擇與核心相符的 Mojang JAR。'
  if (raw.includes('UnexpectedMojangArtifact')) return '這個核心不需要 Mojang JAR，請清空該欄位。'
  if (raw.includes('CoreSourceMissingContact') || raw.includes('CoreSourceInvalidContact')) return 'PaperMC 要求提供有效的聯絡網址。請設定本次啟動的來源聯絡網址後重試。'
  if (raw.includes('CoreCandidateUnverified')) return '無法核對候選核心。請選用已驗證的同版 JAR 與 SHA-256。'
  if (raw.includes('CoreCandidateBusy')) return '候選核心正在下載或供其他實例安裝，請稍後重試。'
  if (raw.includes('CoreMaintenancePending')) return '先前維護的結果未確認。請在核心維護視窗核對後再操作。'
  if (raw.includes('CoreArtifactChanged')) return '核心檔案已變更。請重新核對目前版本與候選檔案。'
  if (raw.includes('InvalidAutoRestartPolicy')) return '請輸入 2 到 600 秒的等待時間，以及 1 到 5 次的重試上限。'
  if (raw.includes('InvalidJvmOption')) return '進階旗標無效、重複或超出範圍。請核對欄位下方列出的格式。'
  if (raw.includes('JvmProbeFailed')) return '所選 Java 無法以這組 JVM 參數啟動。請檢查記憶體與旗標後再儲存。'
  if (raw.includes('InvalidJvmMemory')) return '請核對記憶體上下限，最小記憶體不可大於最大記憶體。'
  if (raw.includes('InvalidJavaExecutable')) return '選擇的 Java 執行檔無法使用。請重新探索或輸入有效路徑。'
  if (raw.includes('InvalidJavaScanDirectory')) return '請核對 Java 搜尋範圍的資料夾路徑。'
  if (raw.includes('JavaScanDirectoryConflict')) return 'Java 搜尋範圍已被其他程序修改，請重新讀取後再儲存。'
  if (raw.includes('AutoJavaUnavailable')) return '找不到與這個核心相容的 Java。請在實例設定指定已安裝的 Java。'
  if (raw.includes('GlobalJavaConflict')) return '全域 Java 設定已在別處變更。請重新讀取設定後再儲存。'
  if (raw.includes('GlobalJavaIncompatible')) return '全域 Java 版本與伺服器核心不相容。請調整全域 Java 或為此實例指定相容版本。'
  if (raw.includes('GlobalJavaUnavailable') || raw.includes('InvalidJavaPath')) return '無法使用指定的 Java。請在全域 Java 設定選擇有效的 java.exe。'
  if (raw.includes('JavaEnvironmentOverride')) return '目前環境包含額外 JVM 選項。請從未設定額外 JVM 選項的環境重新開啟，再次核對。'
  if (raw.includes('JavaProbe')) return '無法完成 Java 執行驗證。請檢查路徑、權限與版本後重試。'
  if (raw.includes('InstanceNotConfirmedStopped')) return '實例尚未確認為已停止。請重新核對狀態後再試。'
  if (raw.includes('ResultUnconfirmed') || raw.includes('StopResultUnconfirmed')) return '操作結果未確認。請查看工作紀錄與目前工作階段。'
  if (raw.includes('RunUnconfirmed')) return '工作階段身分無法確認或已改變。請重新核對實例狀態。'
  if (raw.includes('StartIntentChanged')) return '啟動前的工作階段或強制停止狀態已改變。請重新核對實例後再試。'
  if (raw.includes('AlreadyManaged')) return '此實例已在管理清單中。請重新核對清單。'
  if (raw.includes('InvalidServerPort')) return '伺服器 Port 設定無效。請核對 server.properties 後再試。'
  if (raw.includes('RequestConflict')) return '請求識別碼與先前操作不符。請重新核對工作紀錄。'
  if (raw.includes('ExpectedLoopbackServerAddress')) return '伺服器位址不是預期的本機位址。請核對 server.properties。'
  if (/os error 5\b/i.test(raw)) return 'Windows 拒絕存取檔案。請先核對檔案占用與操作紀錄；結果未確認時不要重複建立同名實例。'
  if (/os error (2|3)\b/i.test(raw)) return '找不到指定的檔案或資料夾。請核對路徑後重試。'
  return '管理服務無法完成操作。請核對實例狀態與操作紀錄後再試。'
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
  candidate?: { build: string; sha256: string }
  reason?: string
  checkedAt?: number
  nativeVersion?: { status: string; behind?: number }
}
