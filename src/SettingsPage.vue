<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, shallowRef, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import LanguageSetting from './components/LanguageSetting.vue'
import DisplayPreferencesFields from './components/DisplayPreferencesFields.vue'
import FieldDraftActions from './components/FieldDraftActions.vue'
import JavaSearchScope from './components/JavaSearchScope.vue'
import OperationButton from './components/OperationButton.vue'
import { formatDisplayTime, isMessageId, useAppLanguage, type TimeFormat, type LanguagePreference } from './i18n'
import ManagementPathRows, { type PathDraft } from './components/ManagementPathRows.vue'
import PathPickerButton from './components/PathPickerButton.vue'
import { managementPathError, type ManagementPathKind } from './management-folders'
import OverlayScroll from './components/OverlayScroll.vue'
import PolicyFields from './components/PolicyFields.vue'
import RecoveryKeyPanel from './components/RecoveryKeyPanel.vue'
import AboutPanel from './components/AboutPanel.vue'
import WorldLiveControls from './components/WorldLiveControls.vue'
import { formatNodes, nodeProblem, testAdminPayload, type PermissionKind } from './test-admins'
import TestAdminFields, { type LuckPermsArtifact, type TestAdministrator } from './components/TestAdminFields.vue'
import { isWorldRule, policyLabel, validWorldRuleValue, verifiedWorldRuleReceipt, type InitializationSnapshot, type PolicyProfile, type WorldLiveValue, type WorldPreview, type WorldRuleSaveReceipt, type WorldSnapshot } from './policy-settings'
import { applySelectionOpacity, refreshSelectionColor, refreshSelectionTheme, resolveSelectionColor, resolveSelectionOpacity, resolveTheme } from './theme'
import { displayPath } from './path-display'
import { errorText, manage, nativeBridge, rawError, type InstanceMeta, type Jvm, type AutoRestart, type AutoRestartStatus, type LifecycleTiming, type InstanceSettings, type JavaCandidate, type Preferences, type PreferencesSnapshot } from './manager'
const props=defineProps<{instance?:string;instanceState?:string;instanceRun?:string;worldCommandReady?:boolean;worldRuleWork?:string;restartStatus?:AutoRestartStatus|null;instanceMeta?:InstanceMeta;preferences:Preferences;preferencesRevision:string;preferencesLoaded:boolean;preferencesReady:boolean;connected:boolean;javaCandidates:JavaCandidate[];javaChecked:boolean;refreshJava:(origin:"manual"|"background",fresh?:boolean)=>Promise<void>;retryPreferences:()=>Promise<void>}>()
const emit=defineEmits<{(e:"language-preview",v?:LanguagePreference):void;(e:'close'):void;(e:'preferences',snapshot:PreferencesSnapshot):void;(e:'dirty',dirty:boolean):void;(e:'saving',saving:boolean):void;(e:'mutating'):void;(e:'management-changed'):void;(e:'core',name:string):void}>()
type Tab='appearance'|'global-java'|'java'|'restart'|'autorestart'|'terminal'|'initialization'|'world'|'core'|'updates'|'scan'|'recovery'|'test-admins'|'about'
type TestAdminSnapshot={administrators:TestAdministrator[];luckperms:boolean;op:boolean;permissions:string[]|null;keptPermissions?:string[];group:string;revision:string;artifacts:LuckPermsArtifact[]}
type ScanPaths={allowPaths:string[];excludePaths:string[]}
type ScanSnapshot={policy:ScanPaths & {allow:string[];exclude:string[];managedFolders?:boolean};revision:string;base?:string;path?:string;rows?:ScanRow[]}
type ScanRow={name:string;path:string;visible:boolean;changed:boolean;active?:boolean;reason:'Excluded'|'Default'|'Allowed'|'NotAllowed'}
type ScopeIssue = { kind: ManagementPathKind; index: number; reason: string }
type ScopePreview = {
  previewRevision: string | null
  complete: boolean
  reason?: string
  issues: ScopeIssue[]
  rows: ScanRow[]
  candidates: { name: string; directory: string; state: string; reason?: string | null; warning?: string | null }[]
}
type GlobalJavaMode = 'automatic' | 'manual'
type GlobalJava = { java: string; revision: string; mode?: GlobalJavaMode; manualPath?: string }
const tab=ref<Tab>(props.instance?'java':'appearance')
const saving=ref(false), message=ref(''), error=ref('')
// Remember which text was shown as saved or failed, so a language change cannot break the comparison.
const savedText=ref(''), failureText=ref('')
const preferencesDraftRevision=ref(props.preferencesRevision)
const preferencesConflict=ref(false)
const saveSucceeded=ref(false)
const saveNeedsVerification=ref(false)
const savedGlobalJava=ref<GlobalJava>()
const globalJavaPath=ref('')
const globalJavaPathInput=ref<HTMLInputElement>()
const globalJavaMode=ref<GlobalJavaMode>('automatic')
const javaPathPickerBusy=ref(false)
const javaSearchScope=ref<InstanceType<typeof JavaSearchScope>>()
const javaSearchDirty=ref(false)
const testAdmins=ref<TestAdminSnapshot>()
const testAdminRows=ref<TestAdministrator[]>([])
const testAdminLuckperms=ref(true)
const testAdminOp=ref(false)
const testAdminKind=ref<PermissionKind>('all')
const testAdminNodes=ref('')
const testAdminGroup=ref('')
const testAdminNodesError=ref('')
watch([testAdminNodes,testAdminKind],()=>{testAdminNodesError.value=''})
const testAdminFetching=ref(false)
const testAdminDraft=computed(()=>testAdminPayload({rows:testAdminRows.value,luckperms:testAdminLuckperms.value,op:testAdminOp.value,kind:testAdminKind.value,nodes:testAdminNodes.value,group:testAdminGroup.value}))
function showTestAdmins(snapshot: TestAdminSnapshot) {
  testAdmins.value=snapshot
  testAdminRows.value=snapshot.administrators.map(row=>({...row}))
  testAdminLuckperms.value=snapshot.luckperms
  testAdminOp.value=snapshot.op
  testAdminKind.value=snapshot.permissions?'custom':'all'
  testAdminNodes.value=formatNodes(snapshot.permissions??snapshot.keptPermissions)
  testAdminGroup.value=snapshot.group
}
async function fetchLuckPerms(){
  if(testAdminFetching.value || !props.connected)return
  testAdminFetching.value=true
  error.value=''
  try{
    const result=await manage<{artifacts:LuckPermsArtifact[]}>(['test-admin-fetch-luckperms'])
    if(testAdmins.value)testAdmins.value={...testAdmins.value,artifacts:result.artifacts}
    message.value=t('settingsPage.testAdminsFetched')
  }catch(e){
    error.value=t('settingsPage.testAdminsFetchFailed',{reason:errorText(e)})
    try{
      const latest=await manage<TestAdminSnapshot>(['test-admin-settings'])
      if(testAdmins.value)testAdmins.value={...testAdmins.value,artifacts:latest.artifacts}
    }catch(reread){console.warn('LuckPerms cache reread failed',errorText(reread))}
  }finally{testAdminFetching.value=false}
}
function showGlobalJava(snapshot: GlobalJava) {
  savedGlobalJava.value=snapshot
  globalJavaPath.value=snapshot.manualPath ?? snapshot.java
  globalJavaMode.value=snapshot.mode ?? (snapshot.java ? 'manual' : 'automatic')
}
function setJavaFromPicker(value: string) {
  error.value=''
  if (tab.value === 'global-java') globalJavaPath.value=pathLabel(value)
  else if (tab.value === 'java') settings.java=value
}
function pathPickerFailed() {
  error.value=t('management.pickFailed')
}

const { t, locale }=useAppLanguage()
const language=ref<LanguagePreference>(props.preferences.language??'system')
watch(language,value=>{if(tab.value==='appearance')emit('language-preview',value)})
watch(()=>props.preferences.language,value=>{
  if(!saving.value && !dirty.value)language.value=value??'system'
})
const theme=ref(props.preferences.theme)
const corePrefetch=ref(props.preferences.corePrefetch ?? false)
const selectionColor=ref(resolveSelectionColor(props.preferences.selectionColor))
const selectionOpacity=ref(resolveSelectionOpacity(props.preferences.selectionOpacity))
const selectionOpacityInput=ref(String(selectionOpacity.value))
const parsedSelectionOpacity=computed(() => {
  const draft=selectionOpacityInput.value
  return /^[0-9]{1,3}$/.test(draft) && Number(draft)<=100 ? Number(draft) : null
})
const selectionOpacityInvalid=computed(() => parsedSelectionOpacity.value === null)
function setSelectionOpacity(value: number) {
  selectionOpacity.value=value
  selectionOpacityInput.value=String(value)
}
function editSelectionOpacity(event: Event) {
  selectionOpacityInput.value=(event.target as HTMLInputElement).value
  const parsed=parsedSelectionOpacity.value
  if(parsed!==null) selectionOpacity.value=parsed
}
function pasteSelectionOpacity(event: ClipboardEvent) {
  // A single-line input strips newlines from paste, which could turn an invalid paste into a valid number.
  const pasted=event.clipboardData?.getData('text/plain') ?? ''
  if(!/[\r\n]/.test(pasted)) return
  event.preventDefault()
  const input=event.target as HTMLInputElement
  const start=input.selectionStart ?? input.value.length
  input.setRangeText(pasted.replace(/\r\n|\r|\n/g,' '),start,input.selectionEnd ?? start,'end')
  editSelectionOpacity(event)
}
const pathSeparator=ref(props.preferences.pathSeparator ?? 'forward')
const timeFormat=ref<TimeFormat>(props.preferences.timeFormat ?? 'local24')
const consoleTitleTemplate=ref(props.preferences.consoleTitleTemplate ?? '{instance}')
const instanceConsoleTitle=ref(props.preferences.consoleTitleOverrides?.[props.instance ?? ''] ?? '')
function resetDisplayPreferences(){
  corePrefetch.value=props.preferences.corePrefetch ?? false
  selectionColor.value=resolveSelectionColor(props.preferences.selectionColor)
  setSelectionOpacity(resolveSelectionOpacity(props.preferences.selectionOpacity))
  timeFormat.value=props.preferences.timeFormat??'local24'
  consoleTitleTemplate.value=props.preferences.consoleTitleTemplate??'{instance}'
  instanceConsoleTitle.value=props.preferences.consoleTitleOverrides?.[props.instance??'']??''
}
const media=window.matchMedia('(prefers-color-scheme: dark)')
const systemThemeListener=()=>{
  if(theme.value==='system'){
    document.documentElement.dataset.theme=resolveTheme('system',media.matches)
    refreshSelectionTheme()
    if(nativeBridge)void invoke('present_window',{theme:document.documentElement.dataset.theme}).catch(e=>{error.value=t('settingsPage.themePreviewFailed',{reason:errorText(e)})})
  }
}
const settings=reactive<{name:string;java:string;port:number|null;revision:string;canSelectJavaAutomatically?:boolean}>({name:props.instance??'',java:props.instanceMeta?.java??'',port:props.instanceMeta?.port??null,revision:''})
type InstancePending = { state: 'Queued' | 'Applying' | 'Conflict'; values: Partial<InstanceSettings> }
const instancePending = ref<InstancePending | null>(null)
const advancedJvm=ref("")
const advancedJvmOpen = ref(false)
type JvmMemory = number | '' | null
type JvmDraft = { minMib: JvmMemory; maxMib: JvmMemory; gc: Jvm['gc'] }
type JavaDraft = { java: string; port: number | null; jvm: JvmDraft; advancedJvm: string }
const jvm=reactive<JvmDraft>({minMib:null,maxMib:null,gc:null})
const minMemory = computed({
  get: () => jvm.minMib ?? 256,
  set: value => { jvm.minMib = value },
})
const maxMemory = computed({
  get: () => jvm.maxMib ?? 1024,
  set: value => { jvm.maxMib = value },
})
watch([tab, () => props.instance], () => { advancedJvmOpen.value = false })
watch(advancedJvm, value => {
  if (value.trim()) advancedJvmOpen.value = true
})
function showJvm(value?: Jvm | null){
  advancedJvm.value=value?.advanced?.join("\n")??""
  if (advancedJvm.value.trim()) advancedJvmOpen.value = true
  Object.assign(jvm,{minMib:value?.minMib??null,maxMib:value?.maxMib??null,gc:value?.gc??null})
}
const timing=reactive<LifecycleTiming>({stopSeconds:null,readySeconds:null,restartDelayMs:null})
function showTiming(saved?: LifecycleTiming | null){
  Object.assign(timing,{stopSeconds:saved?.stopSeconds??null,readySeconds:saved?.readySeconds??null,restartDelayMs:saved?.restartDelayMs??null})
}
const defaultAutoRestart: AutoRestart = {onCrash:false,onUnexpectedExit:false,delaySeconds:5,maxAttempts:2}
const autoRestart=reactive({...defaultAutoRestart, advanced:false, delays:'2, 5', windowSeconds:600, stableSeconds:300})
function showAutoRestart(saved?: AutoRestart | null){
  const policy=saved??defaultAutoRestart
  const advanced=policy.retryDelaysSeconds!==undefined || policy.windowSeconds!==undefined || policy.stableSeconds!==undefined
  const defaults=Array.from({length:policy.maxAttempts},(_,index)=>[2,5,10][Math.min(index,2)])
  Object.assign(autoRestart, {
    onCrash:policy.onCrash,onUnexpectedExit:policy.onUnexpectedExit,
    delaySeconds:policy.delaySeconds,maxAttempts:policy.maxAttempts,advanced,
    delays:(policy.retryDelaysSeconds??defaults).join(', '),
    windowSeconds:policy.windowSeconds??600,stableSeconds:policy.stableSeconds??300
  })
}
function showInstanceSettings(result: InstanceSettings) {
  const pending = (result as InstanceSettings & { pending?: InstancePending | null }).pending ?? null
  instancePending.value = pending
  Object.assign(settings, result, { java: pending?.values.java ?? result.java, port: pending?.values.port ?? result.port })
  showJvm(pending?.values.jvm ?? result.jvm)
  showTiming(pending?.values.timing ?? result.timing)
  showAutoRestart(pending?.values.autoRestart ?? result.autoRestart)
}
function integerIn(value:number,min:number,max:number){return Number.isInteger(value) && value>=min && value<=max}
const autoRestartPayload=computed<AutoRestart|undefined>(()=>{
  const {onCrash,onUnexpectedExit,delaySeconds,maxAttempts,advanced,delays,windowSeconds,stableSeconds}=autoRestart
  if(!integerIn(delaySeconds,2,600) || !integerIn(maxAttempts,1,advanced?10:5))return
  const base={onCrash,onUnexpectedExit,delaySeconds,maxAttempts}
  if(!advanced)return base
  const entries=delays.split(',').map(value=>value.trim())
  if(entries.length!==maxAttempts || !entries.every(value=>/^\d{1,3}$/.test(value) && integerIn(Number(value),1,300))
    || !integerIn(windowSeconds,60,3600) || !integerIn(stableSeconds,60,3600))return
  return {...base,retryDelaysSeconds:entries.map(Number),windowSeconds,stableSeconds}
})
const restartTime=computed(()=>{
  const at=props.restartStatus?.dueAt
  if(props.restartStatus?.phase==='Scheduled' && typeof at==='number' && Number.isFinite(new Date(at).getTime())){
    return formatDisplayTime(at,locale.value,props.preferences.timeFormat)
  }
})
const restartAttemptCount=computed(()=>{
  const count=props.restartStatus?.attemptCount
  if(count!==undefined)return new Intl.NumberFormat(locale.value).format(count)
})
const formattedJava=computed({get:()=>displayPath(settings.java,props.preferences.pathSeparator),set:(value:string)=>{settings.java=value}})
const javaInput = computed(() => ({
  java: settings.java, port: settings.port, revision: settings.revision,
  jvm: {
    minMib: typeof jvm.minMib === 'number' ? jvm.minMib : null,
    maxMib: typeof jvm.maxMib === 'number' ? jvm.maxMib : null,
    gc: jvm.gc,
    advanced: advancedJvm.value.split(/\r?\n/).map(value => value.trim()).filter(Boolean),
  },
}))
const javaDraft = computed(() => JSON.stringify(javaInput.value))
const probingJava=ref(false)
const javaProbeResult=ref<'verified'|'failed'>()
const javaProbeError=ref('')
let javaProbeEpoch=0
watch([javaDraft,()=>props.instance,tab],()=>{
  javaProbeEpoch++
  javaProbeResult.value=undefined
  javaProbeError.value=''
})
async function probeJava(){
  const name=props.instance
  if(!name || !props.connected || !settings.revision || !settings.java || probingJava.value || saving.value)return
  const draft=javaDraft.value
  const epoch=++javaProbeEpoch
  probingJava.value=true
  const matchesDraft=()=>epoch===javaProbeEpoch && name===props.instance && tab.value==='java' && draft===javaDraft.value
  try {
    const checked=await manage<{valid:boolean}>(['java-check',name,draft])
    if(matchesDraft()){
      if(checked.valid!==true)throw new Error('JavaProbeFailed')
      javaProbeResult.value='verified'
      javaProbeError.value=''
    }
  }catch(failure){
    if(matchesDraft()){
      javaProbeResult.value='failed'
      javaProbeError.value=errorText(failure)
      advancedJvmOpen.value=true
    }
  }finally{probingJava.value=false}
}
function pathLabel(value:string){return displayPath(value,props.preferences.pathSeparator)}
const javaBusy=ref(false)
const pickerDialog=ref<HTMLDialogElement>()
const pendingJava=ref('')
const javaSavedError=ref('')
const javaPickerDraft=computed(()=>props.connected && tab.value==='global-java'
  ? javaSearchScope.value?.previewInput : undefined)
const javaDialogMode=ref<'choose'|'results'>('choose')
const javaDraftSnapshot=ref<{input:string; candidates:JavaCandidate[]}>()
const javaDraftError=ref<{input:string; message:string}>()
let javaPickerEpoch=0
let javaDraftRequest: {input:string; pending:Promise<void>}|undefined
const currentDraftSnapshot=computed(()=>javaDraftSnapshot.value?.input===javaPickerDraft.value ? javaDraftSnapshot.value : undefined)
const candidates=computed(()=>javaPickerDraft.value===undefined ? props.javaCandidates : currentDraftSnapshot.value?.candidates??[])
const pickerChecked=computed(()=>javaPickerDraft.value===undefined ? props.javaChecked : !!currentDraftSnapshot.value)
const javaPickerError=computed(()=>javaPickerDraft.value===undefined ? javaSavedError.value
  : javaDraftError.value?.input===javaPickerDraft.value ? javaDraftError.value?.message : undefined)
const canChooseJava=computed(()=>tab.value!=='global-java' || globalJavaMode.value==='manual')
const selectedJavaValid=computed(()=>javaDialogMode.value==='choose' && canChooseJava.value
  && candidates.value.some(candidate=>candidate.path===pendingJava.value&&candidate.valid))
const init = ref<InitializationSnapshot>()
const initValues = ref<Record<string,string>>({})
const initChoice = ref('PAPER@26.3')
function profileKey(profile: PolicyProfile) { return profile.family + '@' + profile.version }
function showInitialization(snapshot: InitializationSnapshot) {
  init.value = snapshot
  initChoice.value = profileKey(snapshot.profile)
  initValues.value = Object.fromEntries(snapshot.rows.flatMap(row => row.value === null ? [] : [[row.key, row.value]]))
}
function onInitializationChoice(event: Event) {
  const picker = event.target as HTMLSelectElement
  if (dirty.value) {
    picker.value = initChoice.value
    error.value = t('settingsPage.saveOrRestoreFirst')
    return
  }
  const choice = picker.value
  picker.value = initChoice.value
  void readSettings('manual', choice)
}
function setInitializationValue(key: string, value: string | null) {
  if (value === null) delete initValues.value[key]
  else initValues.value[key] = value
}
const scan=ref<ScanSnapshot>()
const scanDraft = reactive<Record<ManagementPathKind, PathDraft[]>>({ allow: [], exclude: [] })
const scanKinds = ['allow', 'exclude'] as const
const scanPickerBusy=ref(false)
const scanIssues=ref<Record<number,string>>({})
const scopeConfirmDialog=ref<HTMLDialogElement>()
const scopeCancel=ref<HTMLButtonElement>()
const scopePlan=ref<{ payload: string; preview: ScopePreview }>()
const scopeChangedRows=computed(()=>scopePlan.value?.preview.rows.filter(row=>row.changed)??[])
const scopeCandidates=computed(()=>scopePlan.value?.preview.candidates.filter(row=>row.reason!=='AlreadyManaged')??[])
const scopeSavingStart=ref(false)
let nextScanPathId = 0
let scanValidationTimer: ReturnType<typeof setTimeout> | undefined
let scanValidationEpoch = 0

function scanPaths(): ScanPaths {
  return {
    allowPaths: scanDraft.allow.map(row => row.path.trim()),
    excludePaths: scanDraft.exclude.map(row => row.path.trim()),
  }
}

function showScan(snapshot:ScanSnapshot){
  scan.value={...scan.value,...snapshot}
  scanDraft.allow=snapshot.policy.allowPaths.map(path => ({ id: nextScanPathId++, path }))
  scanDraft.exclude=snapshot.policy.excludePaths.map(path => ({ id: nextScanPathId++, path }))
}

function scanPayload(){
  return JSON.stringify({ revision:scan.value?.revision, ...scanPaths() })
}

function addScanPath(kind: ManagementPathKind) {
  scanDraft[kind].push({ id: nextScanPathId++, path: '' })
}

function editScanPath(kind: ManagementPathKind, id: number, path: string) {
  const row = scanDraft[kind].find(row => row.id === id)
  if (row) row.path = path
}

function removeScanPath(kind: ManagementPathKind, id: number) {
  scanDraft[kind] = scanDraft[kind].filter(row => row.id !== id)
}

function pathIssueMessage(reason: string) {
  if (reason === 'ScanExclusionOutsideWhitelist') return t('management.excludeOutside')
  if (reason === 'DuplicateScanPath') return t('management.pathDuplicate')
  if (reason === 'BatchImportDirectoryOutsideScope') return t('batchImport.reason.BatchImportDirectoryOutsideScope')
  return t('management.pathInvalid')
}

function scanNameList(names: string[]) {
  return names.join(t('settingsPage.listSeparator')) || t('diagnostics.noSource')
}

function scopeCandidateReason(reason?: string | null) {
  if (!reason) return ''
  const key='batchImport.reason.'+reason
  return isMessageId(key)?t(key):t('batchImport.reason.unknown')
}

function scopeCandidateWarning(warning: string) {
  const key='core.warning.'+warning
  return isMessageId(key)?t(key):t('core.warning.CoreUnverified')
}

function applyScopeIssues(issues: ScopeIssue[]) {
  const messages: Record<number,string> = {}
  for (const issue of issues) {
    const row = scanDraft[issue.kind][issue.index]
    if (row) messages[row.id] = row.path.trim() ? pathIssueMessage(issue.reason) : t('management.pathRequired')
  }
  scanIssues.value = messages
}

async function focusScopeIssue() {
  await nextTick()
  document.querySelector<HTMLElement>('.path-rules [aria-invalid=true]')?.focus()
}

async function showScanError(failure: unknown) {
  const issue = managementPathError(rawError(failure))
  if (issue) {
    applyScopeIssues([issue])
    return
  }
  error.value = errorText(failure)
}

function queueScopeValidation() {
  const epoch = ++scanValidationEpoch
  clearTimeout(scanValidationTimer)
  if (tab.value !== 'scan' || !props.connected || !scan.value || !dirty.value) return
  const input=scanPayload()
  // Keep the last server result for unrelated fields while the edited draft is checked.
  scanValidationTimer=setTimeout(async()=>{
    try{
      const validated=await manage<{ issues:ScopeIssue[] }>(['validate-scan-policy', input], 'background')
      if(epoch===scanValidationEpoch && tab.value==='scan' && scanPayload()===input) applyScopeIssues(validated.issues)
    }catch(e){
      // Save always runs a fresh validation and can show a useful manual failure.
      if(epoch===scanValidationEpoch && tab.value==='scan')console.warn('Background management folder check failed',errorText(e))
    }
  }, 250)
}

watch(scanDraft,()=>{
  if(!scopeConfirmDialog.value?.open) scopePlan.value=undefined
  queueScopeValidation()
},{flush:'sync'})
watch([tab,()=>props.connected],()=>{queueScopeValidation()})
onUnmounted(()=>{clearTimeout(scanValidationTimer);scanValidationEpoch++})

async function prepareScopeSave(){
  if(!scan.value||scopeSavingStart.value||scanPickerBusy.value||!dirty.value)return
  const payload=scanPayload()
  scopeSavingStart.value=true
  error.value=''
  try{
    const preview=await manage<ScopePreview>(['preview-management-scope',payload])
    if(tab.value!=='scan'||scanPayload()!==payload)return
    applyScopeIssues(preview.issues)
    if(preview.issues.length){await focusScopeIssue();return}
    if(!preview.complete||!preview.previewRevision){
      error.value=preview.reason==='ManagementScopeCandidateConflict'
        ? t('management.scopeConflict') : t('batchImport.incomplete')
      return
    }
    scopePlan.value={payload,preview}
    await nextTick()
    scopeConfirmDialog.value?.showModal()
    scopeCancel.value?.focus()
  }catch(e){if(tab.value==='scan'&&scanPayload()===payload){await showScanError(e);await focusScopeIssue()}}
  finally{scopeSavingStart.value=false}
}
const world = ref<WorldSnapshot>()
const worldLiveValues = ref<Record<string, WorldLiveValue>>({})
const worldLiveError = ref('')
let worldLiveEpoch = 0
async function readWorldLive() {
  if (!props.instance || !props.connected) return
  const name = props.instance
  const epoch = ++worldLiveEpoch
  const current = () => epoch === worldLiveEpoch && props.instance === name && tab.value === 'world'
  try {
    const values = await manage<Record<string, WorldLiveValue>>(['world-live-values', name], 'background')
    if (current()) {
      if (JSON.stringify(worldLiveValues.value) !== JSON.stringify(values)) worldLiveValues.value = values
      worldLiveError.value = ''
    }
  } catch (failure) {
    if (current() && !Object.keys(worldLiveValues.value).length)
      worldLiveError.value = t('settingsPage.worldRuleRangeFailed', { reason: errorText(failure) })
  }
}
// Rules the Host has not settled yet (a console line is still being typed, or a
// command is in flight) are read again quietly until they settle.
const settlingWorldRule = (value?: WorldLiveValue) => value?.state === 'LiveRetry' || value?.state === 'Applying'
let worldSettleTimer: ReturnType<typeof setInterval> | undefined
watch(() => tab.value === 'world' && Object.values(worldLiveValues.value).some(settlingWorldRule), settling => {
  clearInterval(worldSettleTimer)
  worldSettleTimer = settling ? setInterval(() => void readWorldLive(), 2000) : undefined
}, { immediate: true })
onUnmounted(() => clearInterval(worldSettleTimer))
// The save message follows the rules it reported on until they settle, unless
// another message has replaced it in the meantime.
let ruleSaveMessage: { name: string; fileMessage: string; keys: string[]; text: string } | undefined
watch(worldLiveValues, values => {
  const pending = ruleSaveMessage
  if (!pending || pending.name !== props.instance || message.value !== pending.text) { ruleSaveMessage = undefined; return }
  const rows = Object.fromEntries(pending.keys.filter(key => values[key])
    .map(key => [key, { target: values[key]!.target ?? '', state: values[key]!.state as WorldRuleSaveReceipt['rows'][string]['state'] }]))
  const outcome = worldRuleSummary(rows)
  pending.text = message.value = [pending.fileMessage, ...outcome.messages].filter(Boolean).join(' ')
  if (!pending.keys.some(key => settlingWorldRule(values[key]))) ruleSaveMessage = undefined
}, { deep: true })
const worldValues = ref<Record<string,string>>({})
const worldRuleDraft = ref<Record<string,string>>({})
type WorldFilePlan = { values: Record<string, string>; preview: WorldPreview }
type WorldSavePlan = { name: string; files: WorldFilePlan[]; rules: Record<string,string> }
const worldSavePlan = ref<WorldSavePlan>()
const ruleRequests = new Map<string, string>()
const worldConfirmDialog = ref<HTMLDialogElement>()
const worldChanges = computed(() => Object.fromEntries((world.value?.rows ?? [])
  .filter(row => {
    if (!row.editable || isWorldRule(row) || row.key === 'initialTime' || !Object.hasOwn(worldValues.value, row.key)) return false
    const pending = world.value?.pending
    const saved = pending?.state === 'Conflict' ? row.value : pending?.values[row.key] ?? row.value
    return saved !== worldValues.value[row.key]
  })
  .map(row => [row.key, worldValues.value[row.key]])))
const worldCanSave = computed(() => Object.keys(worldChanges.value).length > 0
  || Object.keys(worldRuleDraft.value).length > 0)
const worldFileDrafts = computed(() => {
  const groups = new Map<string, Record<string, string>>()
  for (const row of world.value?.rows ?? []) {
    const value = worldChanges.value[row.key]
    if (value === undefined) continue
    const file = row.file ?? 'server.properties'
    const group = groups.get(file) ?? {}
    group[row.key] = value
    groups.set(file, group)
  }
  return [...groups.values()]
})
const worldReviewedChanges = computed(() => worldSavePlan.value?.files.flatMap(file => file.preview.changes) ?? [])
const yamlPreflightSummary = computed(() => {
  const preflight = world.value?.yamlPreflight
  const fields = preflight?.changes.length ?? 0
  const files = preflight?.filesChanged?.length ?? 0
  if (fields || files) {
    const summary = []
    if (fields) summary.push(t('settingsPage.yamlExternalChanges', { count: fields }))
    if (files) summary.push(t('settingsPage.yamlFilesChanged', { count: files }))
    return summary.join(' · ')
  }
  if (preflight?.state === 'Compared') return t('settingsPage.yamlNoChanges')
  return preflight?.state === 'Unverified' ? t('settingsPage.yamlUnverified') : t('settingsPage.yamlNoResult')
})
function showWorld(snapshot: WorldSnapshot) {
  world.value = snapshot
  worldValues.value = { ...Object.fromEntries(snapshot.rows.flatMap(row => row.value === null ? [] : [[row.key, row.value]])),
    ...snapshot.pending?.values }
}
function setWorldValue(key: string, value: string | null) {
  if (value === null) return
  const row = world.value?.rows.find(item => item.key === key)
  if (row && isWorldRule(row)) {
    const observed = worldLiveValues.value[key]
    if (value === (observed?.target ?? observed?.value ?? '')) delete worldRuleDraft.value[key]
    else worldRuleDraft.value[key] = value
  } else worldValues.value[key] = value
  worldSavePlan.value = undefined
}
function wheelToScroll(event:WheelEvent){
  if(document.activeElement!==event.target)return
  event.preventDefault()
  const origin=event.target as HTMLElement
  const scroll=origin.closest<HTMLElement>('.overlay-viewport')??origin.closest<HTMLElement>('dialog')
  if(scroll)scroll.scrollTop+=event.deltaY
}
const baseline=ref('')
const savedJavaDraft = computed<JavaDraft | undefined>(() =>
  tab.value === 'java' && baseline.value ? JSON.parse(baseline.value) : undefined,
)
const javaFieldsReady = computed(() => Boolean(settings.revision && savedJavaDraft.value))
function javaSavePayload() {
  const { jvm: options, ...input } = javaInput.value
  const saved = savedJavaDraft.value
  if (saved && instancePending.value?.state !== 'Conflict' &&
      saved.jvm.minMib === options.minMib && saved.jvm.maxMib === options.maxMib &&
      saved.jvm.gc === options.gc && saved.advancedJvm === advancedJvm.value) {
    return JSON.stringify(input)
  }
  return JSON.stringify({ ...input, jvm: options })
}
const discardDialog=ref<HTMLDialogElement>()
const pendingTab=ref<Tab|null>(null)
const pendingClose=ref(false)
let loadEpoch=0
let javaReadEpoch=0
const settingsRetry=shallowRef<{ tab:Tab; instance?:string }>()
const retryingRead=computed(()=>settingsRetry.value?.tab===tab.value && settingsRetry.value.instance===props.instance)
const editableTab=computed(()=>['appearance','updates','test-admins','global-java','java','initialization','world','scan','restart','autorestart','terminal'].includes(tab.value))
const preferencesTab=computed(()=>['appearance','terminal','updates'].includes(tab.value))
const globalJavaDraft=computed(()=>({mode:globalJavaMode.value,manualPath:globalJavaPath.value.trim()}))
const draftSignature=computed(()=>JSON.stringify(tab.value==='appearance'?{theme:theme.value,selectionColor:selectionColor.value,selectionOpacity:selectionOpacity.value,invalidOpacity:selectionOpacityInvalid.value?selectionOpacityInput.value:null,language:language.value,pathSeparator:pathSeparator.value,timeFormat:timeFormat.value,consoleTitleTemplate:consoleTitleTemplate.value}:tab.value==='updates'?corePrefetch.value:tab.value==='test-admins'?testAdminDraft.value:tab.value==='terminal'?instanceConsoleTitle.value:tab.value==='global-java'?globalJavaDraft.value:tab.value==='java'?{java:settings.java,port:settings.port,jvm:{...jvm},advancedJvm:advancedJvm.value}:tab.value==='restart'?timing:tab.value==='autorestart'?autoRestart:tab.value==='initialization'?initValues.value:tab.value==='world'?{files:worldValues.value,rules:worldRuleDraft.value}:tab.value==='scan'?scanPaths():null))
const draftDirty=computed(()=>baseline.value!=='' && draftSignature.value!==baseline.value)
const dirty=computed(()=>draftDirty.value || (tab.value==='global-java' && javaSearchDirty.value))
const globalJavaCanSave=computed(()=>!!savedGlobalJava.value && dirty.value &&
  !javaPathPickerBusy.value && (!javaSearchDirty.value || javaSearchScope.value?.canSave))
const javaCanSave = computed(() => javaFieldsReady.value && typeof settings.port === 'number' &&
  integerIn(settings.port, 1, 65535) && (dirty.value || instancePending.value?.state === 'Conflict'))
watch(dirty,value=>{emit('dirty',value);if(value){saveSucceeded.value=false;if(message.value && message.value===savedText.value)message.value=''}},{immediate:true})
function worldFieldLabel(key:string){const row=world.value?.rows.find(candidate=>candidate.key===key);return row?policyLabel(row):key}
const saveFailedMessage=computed(()=>!!message.value && message.value===failureText.value)
function markSaved(){baseline.value=draftSignature.value}
function switchTab(next:Tab){if(saving.value||next===tab.value)return;if(dirty.value){pendingTab.value=next;discardDialog.value?.showModal()}else tab.value=next}
function discardAndContinue(){reset();emit('dirty',false);discardDialog.value?.close();if(pendingTab.value)tab.value=pendingTab.value;else if(pendingClose.value)emit('close');pendingTab.value=null;pendingClose.value=false}
function requestClose(){if(saving.value)return;if(dirty.value){pendingTab.value=null;pendingClose.value=true;discardDialog.value?.showModal()}else emit('close')}
function beforeUnload(event:BeforeUnloadEvent){if(dirty.value||saving.value){event.preventDefault();event.returnValue=''}}
onMounted(()=>{
  window.addEventListener('beforeunload',beforeUnload)
  media.addEventListener('change',systemThemeListener)
})
onUnmounted(()=>{
  emit("language-preview")
  loadEpoch++
  javaProbeEpoch++
  javaPickerEpoch++
  window.removeEventListener('beforeunload',beforeUnload)
  media.removeEventListener('change',systemThemeListener)
  const saved=props.preferences.theme
  document.documentElement.dataset.theme=resolveTheme(saved,media.matches)
  void refreshSelectionColor(props.preferences.selectionColor,nativeBridge,props.preferences.selectionOpacity)
  if(nativeBridge)void invoke('present_window',{theme:saved}).catch(e=>console.error('Could not restore the window theme',e))
})
watch(()=>props.instance,()=>{world.value=undefined;worldValues.value={};worldRuleDraft.value={};worldLiveValues.value={};worldLiveError.value='';worldSavePlan.value=undefined;instancePending.value=null;Object.assign(settings,{name:props.instance??'',java:props.instanceMeta?.java??'',port:props.instanceMeta?.port??null,revision:'',canSelectJavaAutomatically:undefined});baseline.value='';tab.value=props.instance?'java':'appearance';message.value='';error.value='';void load()})
watch([() => props.instanceState, () => props.instanceRun], ([state]) => {
  if (tab.value === 'world' && (state === 'Stopped' || state === 'Listening') && !saving.value) void load()
  else if (state === 'Stopped' && !dirty.value && !saving.value) void load()
  else if (tab.value === 'world') void readWorldLive()
})
watch(() => props.worldRuleWork, () => {
  if (tab.value === 'world') void readWorldLive()
})
watch(()=>props.instanceMeta,(meta,previous)=>{
  if(JSON.stringify(meta)===JSON.stringify(previous)||settings.revision||dirty.value)return
  // A metadata refresh must not strand the first settings read without a baseline.
  if(javaReadEpoch){void load();return}
  settings.java=meta?.java??'';settings.port=meta?.port??null
})
watch(()=>props.preferences.theme,value=>{if(!saving.value&&(tab.value!=='appearance'||!dirty.value))theme.value=value})
watch(()=>props.preferences.selectionColor,value=>{
  if(!saving.value&&(tab.value!=='appearance'||!dirty.value))selectionColor.value=resolveSelectionColor(value)
})
watch(()=>props.preferences.selectionOpacity,value=>{
  if(!saving.value&&(tab.value!=='appearance'||!dirty.value))setSelectionOpacity(resolveSelectionOpacity(value))
})
watch(selectionColor,color=>{void refreshSelectionColor(color,nativeBridge,selectionOpacity.value)})
watch(selectionOpacity,applySelectionOpacity)
watch(()=>props.preferences.pathSeparator,value=>{if(!saving.value&&(tab.value!=='appearance'||!dirty.value))pathSeparator.value=value ?? 'forward'})
watch(theme,value=>{
  document.documentElement.dataset.theme=resolveTheme(value,media.matches)
  refreshSelectionTheme()
  if(nativeBridge)void invoke('present_window',{theme:value}).catch(e=>{error.value=t('settingsPage.themePreviewFailed',{reason:errorText(e)})})
})
watch(()=>props.connected,ready=>{if(ready&&!dirty.value&&!saving.value)void load()})
watch(()=>props.preferencesRevision,revision=>{
  if(!preferencesTab.value || dirty.value || saving.value || retryingRead.value)return
  preferencesDraftRevision.value=revision
  resetDisplayPreferences()
  markSaved()
})
watch(tab,()=>{baseline.value='';preferencesConflict.value=false;javaSearchDirty.value=false;message.value='';error.value='';void load()})
function load() { return readSettings() }
async function retrySettings(){
  if(!props.connected || saving.value || retryingRead.value)return
  const request={tab:tab.value,instance:props.instance}
  settingsRetry.value=request
  const previousError=error.value
  const current=()=>settingsRetry.value===request && request.tab===tab.value && request.instance===props.instance
  try{
    if(preferencesTab.value){
      await props.retryPreferences()
      await nextTick()
      if(!current() || !props.connected)return
      preferencesDraftRevision.value=props.preferencesRevision
      preferencesConflict.value=false
      if(!dirty.value){resetDisplayPreferences();markSaved()}
    }else if(!await readSettings('manual'))return
    if(current() && error.value===previousError)error.value=''
  }catch(failure){
    if(current())error.value=t('settingsPage.readFailed',{reason:errorText(failure)})
  }finally{
    if(settingsRetry.value===request)settingsRetry.value=undefined
  }
}
async function readSettings(origin:'manual'|'background'='background', profileChoice = initChoice.value){
  if(origin==='background' && retryingRead.value)return
  const epoch=++loadEpoch
  const targetTab=tab.value
  const targetName=props.instance
  const draftAtStart=draftSignature.value
  const samePage=()=>epoch===loadEpoch && tab.value===targetTab && props.instance===targetName && props.connected
  const current=()=>samePage() && draftSignature.value===draftAtStart
  if(!props.connected)return
  if(targetTab==='java')javaReadEpoch=epoch
  try{
    if(targetTab==='global-java'){
      const result=await manage<GlobalJava>(['global-java-settings'],origin)
      if(!current())return
      showGlobalJava(result)
      markSaved()
      return true
    }
    if(targetTab==='test-admins'){
      const result=await manage<TestAdminSnapshot>(['test-admin-settings'],origin)
      if(!current())return
      showTestAdmins(result)
      markSaved()
      return true
    }
    if(targetTab==='scan'){
      const result=await manage<ScanSnapshot>(['scan-policy'],origin)
      if(!current())return
      showScan(result)
      markSaved()
      return true
    }
    if((targetTab==='java'||targetTab==='restart'||targetTab==='autorestart') && targetName){
      const result=await manage<InstanceSettings>(['instance-settings',targetName],origin)
      if(!current())return
      showInstanceSettings(result)
      markSaved()
    }
    if(targetTab==='initialization'){
      const [family, version] = profileChoice.split('@')
      const result = await manage<InitializationSnapshot>(['get-initialization', family, version], origin)
      if(!current())return
      showInitialization(result)
      markSaved()
    }
    if(targetTab==='world' && targetName){
      const result = await manage<WorldSnapshot>(['world-settings',targetName], origin)
      if (!samePage() || saving.value) return
      if (current() && !dirty.value) {
        showWorld(result)
        markSaved()
      } else if (world.value) {
        world.value.yamlPreflight = result.yamlPreflight
      }
      void readWorldLive()
    }
    if(!current())return
    if(preferencesTab.value){preferencesDraftRevision.value=props.preferencesRevision;resetDisplayPreferences();markSaved()}
    return true
  }catch(e){
    // A background reread keeps the last valid settings on screen; only report when the user asked or nothing is shown yet.
    if(current() && (origin==='manual' || (!baseline.value && (targetTab!=='world'||!world.value))))error.value=t('settingsPage.readFailed',{reason:errorText(e)})
    else console.error('Background settings reload failed',e)
  }
  finally{if(javaReadEpoch===epoch)javaReadEpoch=0}
}
function resetJavaPicker(){
  javaPickerEpoch++
  javaBusy.value=false
  javaSavedError.value=''
}
function handleJavaPickerClose(){
  if(!pickerDialog.value?.open)resetJavaPicker()
}
async function scanJavaPickerDraft(input:string, origin:'manual'|'background'){
  if(input!==javaPickerDraft.value)return
  if(javaDraftRequest?.input===input)return javaDraftRequest.pending
  if(origin==='background' && javaDraftSnapshot.value?.input===input)return
  if(javaDraftRequest){
    await javaDraftRequest.pending
    return scanJavaPickerDraft(input,origin)
  }
  javaDraftError.value=undefined
  const pending=manage<JavaCandidate[]>(['preview-java-discovery',input],origin)
    .then(found=>{
      if(input===javaPickerDraft.value)javaDraftSnapshot.value={input,candidates:found}
    })
    .catch(failure=>{
      if(input===javaPickerDraft.value)javaDraftError.value={input,message:t('settingsPage.javaDiscoverFailed',{reason:errorText(failure)})}
    })
    .finally(()=>{javaDraftRequest=undefined})
  javaDraftRequest={input,pending}
  return pending
}
watch(javaPickerDraft,(input,_previous,cleanup)=>{
  if(input===undefined)return
  const timer=setTimeout(()=>void scanJavaPickerDraft(input,'background'),250)
  cleanup(()=>clearTimeout(timer))
})
function showJavaPicker(){
  void openJavaDialog('choose')
}
async function openJavaDialog(mode:'choose'|'results'){
  if(pickerDialog.value?.open || (mode==='choose' && !canChooseJava.value))return
  resetJavaPicker()
  javaDialogMode.value=mode
  pendingJava.value=tab.value==='global-java' ? globalJavaPath.value : settings.java
  const epoch=javaPickerEpoch
  await nextTick()
  if(epoch!==javaPickerEpoch || (mode==='choose' && !canChooseJava.value))return
  pickerDialog.value?.showModal()
  if(javaPickerDraft.value===undefined && !props.javaChecked){
    void props.refreshJava('manual').catch(failure=>{
      if(epoch===javaPickerEpoch)javaSavedError.value=t('settingsPage.javaDiscoverFailed',{reason:errorText(failure)})
    })
  }
}
async function refreshJavaPicker(){
  if(javaBusy.value)return
  const epoch=javaPickerEpoch
  javaSavedError.value=''
  javaBusy.value=true
  try{
    if(javaPickerDraft.value!==undefined)await scanJavaPickerDraft(javaPickerDraft.value,'manual')
    else await props.refreshJava('manual')
  }
  catch(e){if(epoch===javaPickerEpoch)javaSavedError.value=t('settingsPage.javaDiscoverFailed',{reason:errorText(e)})}
  finally{if(epoch===javaPickerEpoch)javaBusy.value=false}
}
function useJava(){
  if(!selectedJavaValid.value)return
  setJavaFromPicker(pendingJava.value)
  pickerDialog.value?.close()
  message.value=t('settingsPage.javaAddedToDraft')
}
function reset(){
  message.value='';error.value='';saveSucceeded.value=false;saveNeedsVerification.value=false
  if(tab.value==='appearance'){language.value=props.preferences.language??'system';theme.value=props.preferences.theme;pathSeparator.value=props.preferences.pathSeparator ?? 'forward';resetDisplayPreferences()}
  else if(tab.value==='terminal'||tab.value==='updates')resetDisplayPreferences()
  else if(tab.value==='java' && savedJavaDraft.value){
    const saved=savedJavaDraft.value
    settings.java=saved.java
    settings.port=saved.port
    Object.assign(jvm, saved.jvm)
    advancedJvm.value=saved.advancedJvm
  }
  else if(tab.value==='world' && world.value){
    showWorld(world.value)
    worldRuleDraft.value={}
    worldSavePlan.value=undefined
  }
  else if(tab.value==='initialization' && init.value)showInitialization(init.value)
  else if(tab.value==='restart' && baseline.value)showTiming(JSON.parse(baseline.value))
  else if(tab.value==='autorestart' && baseline.value)Object.assign(autoRestart,JSON.parse(baseline.value))
  else if(tab.value==='scan' && scan.value){showScan(scan.value);scanIssues.value={}}
  else if(tab.value==='test-admins' && testAdmins.value)showTestAdmins(testAdmins.value)
  else if(tab.value==='global-java'){
    if(savedGlobalJava.value)showGlobalJava(savedGlobalJava.value)
    javaSearchScope.value?.restore()
  }
  else void load()
  markSaved()
}
function validateWorldRules(rules: Record<string, string>) {
  for (const [key, value] of Object.entries(rules)) {
    const row = world.value?.rows.find(item => item.key === key)
    if (!row || !isWorldRule(row) || !worldLiveValues.value[key] || row.status === 'unsupported')
      throw new Error(t('settingsPage.worldRuleScopeUnverified'))
    if (!validWorldRuleValue(row, value)) throw new Error(t('settingsPage.worldRuleValueCheck', { label: policyLabel(row) }))
  }
}
async function previewWorldFile(name: string, revision: string, values: Record<string,string>) {
  return manage<WorldPreview>(['world-settings', name, JSON.stringify({ revision, values })])
}
function worldRuleSummary(rows: WorldRuleSaveReceipt['rows']) {
  const outcomes = Object.values(rows)
  const queued = outcomes.filter(({state}) => state === 'Queued').length
  // A command still in flight is not verified yet; it reads like a waiting rule.
  const retrying = outcomes.filter(({state}) => state === 'LiveRetry' || (state as string) === 'Applying').length
  const unconfirmed = outcomes.filter(({state}) => state === 'Unconfirmed').length
  const verified = outcomes.length - queued - retrying - unconfirmed
  const messages = []
  if (verified) messages.push(t('settingsPage.worldRulesVerified', { count: verified }))
  if (retrying) messages.push(t('settingsPage.worldRulesLiveRetry', { count: retrying }))
  if (queued) messages.push(t('settingsPage.worldRulesQueued', { count: queued }))
  if (unconfirmed) messages.push(t('settingsPage.worldRulesUnconfirmed', { count: unconfirmed }))
  return { unconfirmed, messages, keys: Object.keys(rows) }
}
async function saveWorld(confirmWorld: boolean) {
  const name = props.instance
  const snapshot = world.value
  if (!name || !snapshot || !worldCanSave.value) return
  saving.value = true
  emit('saving', true)
  message.value = t('action.working')
  error.value = ''
  saveSucceeded.value = false
  saveNeedsVerification.value = false
  let savedFiles = 0
  let filesQueued = false
  let ruleRequestPending = false
  let ruleOutcome: ReturnType<typeof worldRuleSummary> | undefined
  try {
    let plan = worldSavePlan.value
    if (!confirmWorld) {
      const rules = {...worldRuleDraft.value}
      validateWorldRules(rules)
      const files: WorldFilePlan[] = []
      for (const values of worldFileDrafts.value) {
        files.push({ values, preview: await previewWorldFile(name, snapshot.revision, values) })
      }
      plan = { name, files, rules }
      if (files.length) {
        if (!worldConfirmDialog.value) throw new Error(t('settingsPage.worldConfirmUnavailable'))
        worldSavePlan.value = plan
        message.value = ''
        worldConfirmDialog.value.showModal()
        return
      }
    }
    if (!plan || plan.name !== name) return
    worldConfirmDialog.value?.close()
    worldSavePlan.value = undefined
    if (plan.files.some(file => file.preview.revision !== snapshot.revision)) throw new Error('SettingsConflict')
    emit('mutating')
    const remaining = {...worldChanges.value}
    let revision = snapshot.revision
    for (const file of plan.files) {
      let preview = file.preview
      if (revision !== preview.revision) {
        preview = await previewWorldFile(name, revision, file.values)
        if (JSON.stringify(preview.changes) !== JSON.stringify(file.preview.changes))
          throw new Error('SettingsConflict')
      }
      const payload = { revision, values: file.values, preview: preview.preview }
      const saved = await manage<WorldSnapshot>(['save-world-settings', name, JSON.stringify(payload)])
      savedFiles++
      filesQueued ||= Boolean(saved.queued)
      revision = saved.revision
      for (const key of Object.keys(file.values)) delete remaining[key]
      showWorld(saved)
      baseline.value = JSON.stringify({ files: worldValues.value, rules: {} })
      Object.assign(worldValues.value, remaining)
    }
    const ruleKeys = Object.keys(plan.rules).sort()
    if (ruleKeys.length) {
      const rules = Object.fromEntries(ruleKeys.map(key => {
        const value = plan.rules[key]
        return [key, /^\d+$/.test(value) ? String(Number(value)) : value]
      }))
      const payload = JSON.stringify(rules)
      const identity = JSON.stringify([name, payload])
      const request = ruleRequests.get(identity) ?? crypto.randomUUID()
      ruleRequests.set(identity, request)
      ruleRequestPending = true
      const receipt = await manage<WorldRuleSaveReceipt>(['world-live-save-rules', name, request, payload])
      if (!verifiedWorldRuleReceipt(receipt, name, request, rules))
        throw new Error('WorldReceiptMismatch')
      ruleRequestPending = false
      ruleRequests.delete(identity)
      for (const key of ruleKeys) {
        const { state, target } = receipt.rows[key]
        worldLiveValues.value[key] = {...worldLiveValues.value[key], target, state}
        delete worldRuleDraft.value[key]
      }
      ruleOutcome = worldRuleSummary(receipt.rows)
      void readWorldLive()
    }
    markSaved()
    const fileMessage = filesQueued ? t('settingsPage.filesSavedQueued') : savedFiles ? t('settingsPage.filesSaved') : ''
    message.value = [fileMessage, ...(ruleOutcome?.messages ?? [])].filter(Boolean).join(' ') || (savedText.value = t('settings.saved'))
    ruleSaveMessage = ruleOutcome?.keys.length ? { name, fileMessage, keys: ruleOutcome.keys, text: message.value } : undefined
    saveNeedsVerification.value = Boolean(ruleOutcome?.unconfirmed)
    saveSucceeded.value = !saveNeedsVerification.value
  } catch (failure) {
    error.value = errorText(failure)
    if (ruleRequestPending) {
      saveNeedsVerification.value = true
      message.value = savedFiles ? t('settingsPage.filesSavedRulesUnconfirmed') : t('settingsPage.rulesUnconfirmed')
      void readWorldLive()
    } else {
      message.value = failureText.value = savedFiles ? t('settingsPage.partialSaved') : t('settings.saveFailed')
    }
  } finally {
    saving.value = false
    emit('saving', false)
  }
}
async function save(confirmWorld = false, confirmScope = false){
  if(tab.value==='appearance' && selectionOpacityInvalid.value)return
  if(saving.value || !props.connected || !editableTab.value || (preferencesTab.value&&(!props.preferencesReady||preferencesConflict.value)))return
  if(tab.value==='java' && !javaCanSave.value)return
  if(tab.value==='world'){await saveWorld(confirmWorld);return}
  if(tab.value==='global-java' && !globalJavaCanSave.value)return
  if(tab.value==='global-java' && draftDirty.value && globalJavaMode.value==='manual' && !globalJavaPath.value.trim()){
    error.value=t('management.javaManualPathRequired')
    message.value=failureText.value=t('settings.saveFailed')
    saveSucceeded.value=false
    await nextTick()
    globalJavaPathInput.value?.focus()
    return
  }
  if(tab.value==='scan'){
    if(!confirmScope){await prepareScopeSave();return}
    if(!scopePlan.value || scopePlan.value.payload!==scanPayload() || !scopePlan.value.preview.previewRevision)return
  }
  if(tab.value==='autorestart' && !autoRestartPayload.value)return
  // Nodes kept beside "all" may be empty, but must still be valid to be saved.
  const nodeIssue=tab.value!=='test-admins' ? undefined : testAdminKind.value==='custom' ? nodeProblem(testAdminNodes.value) : testAdminNodes.value.trim() ? nodeProblem(testAdminNodes.value) : undefined
  testAdminNodesError.value=''
  if(nodeIssue){
    // Shown under the node field, which is far above the page-level error line.
    testAdminNodesError.value=t(nodeIssue==='empty'?'settingsPage.testAdminsNodesRequired':nodeIssue==='tooMany'?'settingsPage.testAdminsNodesTooMany':'settingsPage.testAdminsNodesInvalid')
    message.value=failureText.value=t('settings.saveFailed')
    saveSucceeded.value=false
    await nextTick()
    const field=document.getElementById('test-admin-nodes')
    if(field instanceof HTMLTextAreaElement && !field.disabled)field.focus()
    else field?.scrollIntoView({block:'center'})
    return
  }
  emit('mutating')
  emit('saving',true)
  saving.value=true;message.value=t('action.working');error.value='';saveSucceeded.value=false;saveNeedsVerification.value=false
  let queued = false
  let searchScopeSaved = false
  try{
    if(preferencesTab.value){
      const next = {...props.preferences}
      if(tab.value==='appearance'){
        const before=baseline.value ? JSON.parse(baseline.value) : {}
        const fields={theme:theme.value,selectionColor:selectionColor.value,selectionOpacity:selectionOpacity.value,language:language.value,
          pathSeparator:pathSeparator.value,timeFormat:timeFormat.value,consoleTitleTemplate:consoleTitleTemplate.value}
        const changed=Object.entries(fields).filter(([key,value])=>before[key]!==value)
        Object.assign(next,Object.fromEntries(changed))
      }else if(tab.value==='updates'){
        next.corePrefetch=corePrefetch.value
      }else if(props.instance){
        const overrides={...props.preferences.consoleTitleOverrides}
        if(instanceConsoleTitle.value.trim())overrides[props.instance]=instanceConsoleTitle.value
        else delete overrides[props.instance]
        next.consoleTitleOverrides=overrides
      }
      const saved=await manage<PreferencesSnapshot>(['save-preferences',JSON.stringify(next),preferencesDraftRevision.value])
      preferencesDraftRevision.value=saved.revision
      emit('preferences',saved)
      await nextTick()
      resetDisplayPreferences()
      emit('language-preview')
    }
    else if(tab.value==='java' && props.instance){
      const saved=await manage<InstanceSettings & {queued?:boolean}>(['save-instance-settings',props.instance,javaSavePayload()])
      showInstanceSettings(saved)
      queued=!!saved.queued
    }
    else if(tab.value==='restart' && props.instance && settings.port!==null){
      const saved=await manage<InstanceSettings & {queued?:boolean}>(['save-instance-settings',props.instance,JSON.stringify({
        java:settings.java,port:settings.port,revision:settings.revision,
        timing:{stopSeconds:typeof timing.stopSeconds==='number'?timing.stopSeconds:null,readySeconds:typeof timing.readySeconds==='number'?timing.readySeconds:null,restartDelayMs:typeof timing.restartDelayMs==='number'?timing.restartDelayMs:null}
      })])
      showInstanceSettings(saved)
      queued=!!saved.queued
    }
    else if(tab.value==='autorestart' && props.instance && settings.port!==null){
      const saved=await manage<InstanceSettings & {queued?:boolean}>(['save-instance-settings',props.instance,JSON.stringify({
        java:settings.java,port:settings.port,revision:settings.revision,autoRestart:autoRestartPayload.value
      })])
      showInstanceSettings(saved)
      queued=!!saved.queued
    }
    else if(tab.value==='initialization' && init.value){
      const saved = await manage<InitializationSnapshot>(['save-initialization', JSON.stringify({
        revision: init.value.revision, profile: init.value.profile, values: initValues.value
      })])
      showInitialization(saved)
    }
    else if(tab.value==='global-java' && savedGlobalJava.value){
      const searchScope = javaSearchScope.value
      const scopeInput = await searchScope?.prepareSave()
      if(scopeInput && searchScope){
        await searchScope.persist(scopeInput)
        searchScopeSaved = true
      }
      if(draftDirty.value){
        showGlobalJava(await manage<GlobalJava>(['save-global-java-settings',JSON.stringify({
          ...globalJavaDraft.value,
          revision:savedGlobalJava.value.revision,
        })]))
      }
    }
    else if(tab.value==='test-admins' && testAdmins.value){
      showTestAdmins(await manage<TestAdminSnapshot>(['save-test-admin-settings',JSON.stringify({...testAdminDraft.value,revision:testAdmins.value.revision})]))
    }
    else if(tab.value==='scan' && scopePlan.value){
      const payload={...JSON.parse(scopePlan.value.payload),previewRevision:scopePlan.value.preview.previewRevision}
      showScan(await manage<ScanSnapshot>(['save-management-scope',JSON.stringify(payload)]))
      scopeConfirmDialog.value?.close()
      emit('management-changed')
    }
    markSaved();message.value=queued?t('settingsPage.savedQueued'):(savedText.value=t('settings.saved'));saveSucceeded.value=true
  }catch(e){
    if (preferencesTab.value && rawError(e).includes('PreferencesSettingsConflict')) preferencesConflict.value=true
    if (tab.value === 'java' && /InvalidJvmOption|JvmProbeFailed/.test(rawError(e))) advancedJvmOpen.value = true
    if (tab.value === 'scan'){
      scopeConfirmDialog.value?.close()
      await showScanError(e)
    }
    else error.value=t('settingsPage.saveFailedReason',{reason:errorText(e)})
    message.value=failureText.value=searchScopeSaved?t('settingsPage.partialSavedJava'):t('settings.saveFailed')
  }finally{
    if(searchScopeSaved)await props.refreshJava('background',true).catch(failure=>console.warn('Java list refresh failed',errorText(failure)))
    saving.value=false;emit('saving',false)
  }
  if(tab.value==='scan' && Object.keys(scanIssues.value).length)await focusScopeIssue()
  if(tab.value==='global-java' && !saveSucceeded.value){
    await nextTick()
    javaSearchScope.value?.focusIssue()
  }
}
void load()
</script>
<template>
  <OverlayScroll class="settings-scroll" :label="t('settings.content')"><header class="page-head"><div><h1>{{ instance ? t('settings.instanceTitle', { name: instance }) : t('nav.settings') }}</h1></div><button v-if="instance" @click="requestClose">{{ t('settings.back') }}</button></header>
    <div class="settings-layout"><nav class="settings-nav" :aria-label="t('settings.category')">
      <template v-if="instance"><button :aria-current="tab==='java'?'page':undefined" @click="switchTab('java')">{{ t('settings.java') }}</button><button :aria-current="tab==='restart'?'page':undefined" @click="switchTab('restart')">{{ t('settings.restart') }}</button><button :aria-current="tab==='autorestart'?'page':undefined" @click="switchTab('autorestart')">{{ t('settings.autoRestart') }}</button><button :aria-current="tab==='terminal'?'page':undefined" @click="switchTab('terminal')">{{ t('settings.terminal') }}</button><button :aria-current="tab==='world'?'page':undefined" @click="switchTab('world')">{{ t('settings.world') }}</button><button :aria-current="tab==='core'?'page':undefined" @click="switchTab('core')">{{ t('settings.core') }}</button></template>
      <template v-else><button :aria-current="tab==='appearance'?'page':undefined" @click="switchTab('appearance')">{{ t('appearance.title') }}</button><button :aria-current="tab==='global-java'?'page':undefined" @click="switchTab('global-java')">{{ t('management.globalJava') }}</button><button :aria-current="tab==='updates'?'page':undefined" @click="switchTab('updates')">{{ t('settings.coreUpdates') }}</button><button :aria-current="tab==='test-admins'?'page':undefined" @click="switchTab('test-admins')">{{ t('settingsPage.testAdmins') }}</button><button :aria-current="tab==='initialization'?'page':undefined" @click="switchTab('initialization')">{{ t('settings.initialization') }}</button><button :aria-current="tab==='scan'?'page':undefined" @click="switchTab('scan')">{{ t('settings.scan') }}</button><button :aria-current="tab==='recovery'?'page':undefined" @click="switchTab('recovery')">{{ t('recovery.heading') }}</button><button :aria-current="tab==='about'?'page':undefined" @click="switchTab('about')">{{ t('components.about.heading') }}</button></template>
      </nav><section class="sheet" :aria-busy="saving">
      <OperationButton v-if="preferencesTab && tab!=='terminal' && connected && (preferencesConflict || !preferencesReady || error)"
        :label="t('action.retryRead')" :busy-label="t('activity.loading')" :busy="retryingRead" @click="retrySettings()" />
      <fieldset class="settings-fields" :disabled="saving || (!connected && tab!=='about') || (preferencesTab && !preferencesLoaded)">
      <template v-if="tab==='appearance'"><h2>{{ t('appearance.title') }}</h2><p v-if="!connected || !preferencesReady" class="form-error" role="alert">{{ t('settingsPage.appearanceUnverified') }}</p><div v-if="preferencesLoaded" class="field"><label for="app-theme">{{ t('appearance.theme') }}</label><div class="editor"><select id="app-theme" v-model="theme" name="theme"><option value="system">{{ t('appearance.systemTheme') }}</option><option value="light">{{ t('appearance.light') }}</option><option value="dark">{{ t('appearance.dark') }}</option></select></div></div>
        <div v-if="preferencesLoaded" class="field">
          <label for="selection-color">{{ t('appearance.selectionColor') }}</label>
          <div class="editor">
            <select id="selection-color" v-model="selectionColor" name="selection-color">
              <option value="system">{{ t('appearance.systemTheme') }}</option>
              <option value="blue">{{ t('appearance.blue') }}</option>
              <option value="red">{{ t('appearance.red') }}</option>
              <option value="green">{{ t('appearance.green') }}</option>
              <option value="magenta">{{ t('appearance.magenta') }}</option>
              <option value="yellow">{{ t('appearance.yellow') }}</option>
              <option value="cyan">{{ t('appearance.cyan') }}</option>
              <option value="purple">{{ t('appearance.purple') }}</option>
            </select>
          </div>
        </div>
        <div v-if="preferencesLoaded" class="field">
          <label for="selection-opacity">{{ t('appearance.selectionOpacity') }}</label>
          <div class="editor">
            <div class="selection-opacity-control">
              <input id="selection-opacity" name="selection-opacity" :value="selectionOpacity" type="range"
                min="0" max="100" step="1" :aria-valuetext="selectionOpacity+'%'"
                :style="{ '--opacity-fill': selectionOpacity+'%' }"
                :disabled="selectionOpacityInvalid || saving"
                @input="setSelectionOpacity(Number(($event.target as HTMLInputElement).value))">
              <input id="selection-opacity-value" name="selection-opacity-value" :value="selectionOpacityInput"
                type="text" inputmode="numeric" autocomplete="off" spellcheck="false"
                :aria-label="t('appearance.selectionOpacityValue')" :aria-invalid="selectionOpacityInvalid"
                :aria-describedby="selectionOpacityInvalid?'selection-opacity-error':undefined" :disabled="saving"
                @input="editSelectionOpacity" @paste="pasteSelectionOpacity"
                @blur="!selectionOpacityInvalid && setSelectionOpacity(selectionOpacity)">
              <span aria-hidden="true">%</span>
            </div>
            <small id="selection-opacity-error" v-if="selectionOpacityInvalid" class="form-error" role="alert">{{ t('appearance.selectionOpacityInvalid') }}</small>
            <div class="selection-color-sample">
              <span>{{ t('appearance.selectionSample') }}</span>
              <span class="badge Listening">{{ t('instance.state.listening') }}</span>
            </div>
          </div>
        </div>
        <div v-if="preferencesLoaded" class="field"><label for="path-separator">{{ t('appearance.pathSeparator') }}</label><div class="editor"><select id="path-separator" v-model="pathSeparator" name="path-separator"><option value="forward">{{ t('appearance.forwardSlash') }}</option><option value="native">{{ t('appearance.backslash') }}</option></select></div></div><LanguageSetting v-if="preferencesLoaded" v-model="language" /><DisplayPreferencesFields v-if="preferencesLoaded" v-model:time-format="timeFormat" v-model:console-title-template="consoleTitleTemplate" /></template>
      <template v-else-if="tab==='java'">
        <h2>{{ t('settings.java') }}</h2>
        <p v-if="instanceState!=='Stopped'" class="notice">{{ t('settingsPage.pendingApplyNotice') }}</p>
        <p v-if="instancePending" class="hint" role="status">{{ instancePending.state === 'Conflict' ? t('settingsPage.pendingConflict') : t('settingsPage.pendingKept') }}</p>
        <p v-if="!settings.revision" class="notice">{{ t('settingsPage.instanceSettingsUnavailable') }}</p>
        <div class="field">
          <label for="instance-java">{{ t('settingsPage.javaExecutable') }}</label>
          <div class="jvm-editor">
            <div class="editor">
              <div class="settings-path-editor">
                <input id="instance-java" v-model.trim="formattedJava" name="java" autocomplete="off"
                  spellcheck="false" :disabled="!javaFieldsReady" :readonly="javaPathPickerBusy"
                  :placeholder="settings.canSelectJavaAutomatically ? t('management.javaInherited') : undefined">
                <PathPickerButton :current="settings.java" kind="java" :disabled="!javaFieldsReady || javaPathPickerBusy"
                  @picking="javaPathPickerBusy=$event" @selected="setJavaFromPicker" @failed="pathPickerFailed" />
              </div>
              <button type="button" :disabled="!javaFieldsReady || javaPathPickerBusy" @click="showJavaPicker">{{ t('management.chooseInstalledJava') }}</button>
            </div>
            <FieldDraftActions field="instance-java" :label="t('settingsPage.javaExecutable')"
              :can-undo="!!savedJavaDraft && settings.java !== savedJavaDraft.java"
              @undo="savedJavaDraft && (settings.java = savedJavaDraft.java)" />
          </div>
        </div>
        <div class="field">
          <label for="instance-port">Port</label>
          <div class="jvm-editor">
            <input id="instance-port" v-model.number="settings.port" name="port" type="number"
              min="1" max="65535" autocomplete="off" :disabled="!javaFieldsReady" @wheel="wheelToScroll">
            <FieldDraftActions field="instance-port" label="Port"
              :can-undo="!!savedJavaDraft && settings.port !== savedJavaDraft.port"
              @undo="savedJavaDraft && (settings.port = savedJavaDraft.port)" />
          </div>
        </div>
        <div class="field">
          <label for="jvm-min">{{ t('settingsPage.minMemoryMib') }}</label>
          <div class="jvm-editor">
            <input id="jvm-min" v-model.number="minMemory" type="number" min="64" max="1048576"
              name="jvm-min" autocomplete="off" :disabled="!javaFieldsReady" @wheel="wheelToScroll"
              @blur="jvm.minMib === '' && (jvm.minMib = null)">
            <FieldDraftActions field="jvm-min" :label="t('settingsPage.minMemory')"
              :can-restore-default="javaFieldsReady && jvm.minMib !== null"
              :can-undo="!!savedJavaDraft && jvm.minMib !== savedJavaDraft.jvm.minMib"
              @restore-default="jvm.minMib = null"
              @undo="savedJavaDraft && (jvm.minMib = savedJavaDraft.jvm.minMib)" />
          </div>
        </div>
        <div class="field">
          <label for="jvm-max">{{ t('settingsPage.maxMemoryMib') }}</label>
          <div class="jvm-editor">
            <input id="jvm-max" v-model.number="maxMemory" type="number" min="64" max="1048576"
              name="jvm-max" autocomplete="off" :disabled="!javaFieldsReady" @wheel="wheelToScroll"
              @blur="jvm.maxMib === '' && (jvm.maxMib = null)">
            <FieldDraftActions field="jvm-max" :label="t('settingsPage.maxMemory')"
              :can-restore-default="javaFieldsReady && jvm.maxMib !== null"
              :can-undo="!!savedJavaDraft && jvm.maxMib !== savedJavaDraft.jvm.maxMib"
              @restore-default="jvm.maxMib = null"
              @undo="savedJavaDraft && (jvm.maxMib = savedJavaDraft.jvm.maxMib)" />
          </div>
        </div>
        <div class="field">
          <label for="jvm-gc">{{ t('settingsPage.gc') }}</label>
          <div class="jvm-editor">
            <select id="jvm-gc" v-model="jvm.gc" name="jvm-gc" :disabled="!javaFieldsReady">
              <option :value="null">{{ t('settingsPage.gcAuto') }}</option><option value="parallel">Parallel GC</option>
              <option value="g1">G1 GC</option><option value="serial">Serial GC</option>
            </select>
            <FieldDraftActions field="jvm-gc" :label="t('settingsPage.gc')"
              :can-restore-default="javaFieldsReady && jvm.gc !== null"
              :can-undo="!!savedJavaDraft && jvm.gc !== savedJavaDraft.jvm.gc"
              @restore-default="jvm.gc = null"
              @undo="savedJavaDraft && (jvm.gc = savedJavaDraft.jvm.gc)" />
          </div>
        </div>
      <details id="jvm-advanced" class="jvm-advanced" :open="advancedJvmOpen"
        @toggle="advancedJvmOpen = ($event.currentTarget as HTMLDetailsElement).open">
        <summary>{{ t('settingsPage.jvmAdvanced') }}</summary>
        <div class="field">
          <label for="jvm-options">{{ t('settingsPage.jvmFlags') }}</label>
          <div class="jvm-editor">
            <div class="editor">
              <textarea id="jvm-options" name="jvm-options" v-model="advancedJvm" rows="4"
                autocomplete="off" spellcheck="false" aria-describedby="jvm-options-hint" :disabled="!javaFieldsReady"></textarea>
              <small id="jvm-options-hint" class="hint">{{ t('settingsPage.jvmFlagsHelp') }}</small>
            </div>
            <FieldDraftActions field="jvm-options" :label="t('settingsPage.jvmFlags')"
              :can-restore-default="javaFieldsReady && advancedJvm !== ''"
              :can-undo="!!savedJavaDraft && advancedJvm !== savedJavaDraft.advancedJvm"
              @restore-default="advancedJvm = ''"
              @undo="savedJavaDraft && (advancedJvm = savedJavaDraft.advancedJvm)" />
          </div>
        </div>
      </details>
      <div class="detail-section">
        <OperationButton :label="t('settings.javaCheck')" :busy-label="t('action.checking')" :busy="probingJava"
          :disabled="!settings.java || !settings.revision || saving" aria-describedby="java-probe-hint" @click="probeJava" />
        <p id="java-probe-hint" class="hint">{{ t('settings.javaCheckHelp') }}</p>
        <p v-if="javaProbeResult==='verified'" role="status" class="save-success">{{ t('settings.javaVerified') }}</p>
        <p v-if="javaProbeResult==='failed'" role="alert" class="form-error">{{ javaProbeError }}</p>
      </div>
      </template>
      <template v-else-if="tab==='initialization'">
        <h2>{{ t('settings.initialization') }}</h2>
        <template v-if="init">
          <div class="field">
            <label for="initialization-profile">{{ t('settingsPage.initProfile') }}</label>
            <select id="initialization-profile" :value="initChoice" @change="onInitializationChoice">
              <option v-for="profile in init.profiles" :key="profileKey(profile)" :value="profileKey(profile)">
                {{ profile.family }} {{ profile.version }}
              </option>
            </select>
          </div>
          <PolicyFields mode="initialization" :rows="init.rows" :values="initValues" @change="setInitializationValue"/>
        </template>
        <OperationButton v-else-if="error" :label="t('action.retryRead')" :busy-label="t('activity.loading')"
          :busy="retryingRead" @click="retrySettings" />
      </template>
      <template v-else-if="tab==='world'">
        <h2>{{ t('settings.world') }}</h2>
        <template v-if="world">
          <p class="hint">{{ world.profile ? world.profile.family + ' ' + world.profile.version : t('settingsPage.worldProfilePending') }}</p>
          <p class="notice">{{ t('settingsPage.worldApplyNotice') }}</p>
          <p v-if="world.pending" class="hint" role="status">{{ world.pending.state === 'Conflict' ? t('settingsPage.pendingConflict') : t('settingsPage.pendingKept') }}</p>
          <details class="world-preflight">
            <summary>{{ t('settingsPage.yamlPreflightTitle') }} <span class="hint">{{ yamlPreflightSummary }}</span></summary>
            <p v-if="world.yamlPreflight?.source === 'Launch' && (world.yamlPreflight.changes.length || world.yamlPreflight.filesChanged?.length)" class="hint">{{ t('settingsPage.yamlLaunchDiff') }}</p>
            <template v-if="world.yamlPreflight?.filesChanged?.length">
              <p class="hint">{{ t('settingsPage.yamlChangedFiles') }}</p>
              <ul>
                <li v-for="file in world.yamlPreflight.filesChanged" :key="file"><code translate="no">{{ file }}</code></li>
              </ul>
            </template>
            <ul v-if="world.yamlPreflight?.changes.length">
              <li v-for="change in world.yamlPreflight.changes" :key="change.file + ':' + change.key">
                <strong>{{ worldFieldLabel(change.key) }}</strong>
                <code class="hint" translate="no">{{ change.file }}</code>
                <p class="world-preflight-values">{{ t('settingsPage.yamlOriginal') }} <bdi translate="no">{{ change.before ?? t('settingsPage.yamlNotRead') }}</bdi>{{ t('settingsPage.yamlReadBack') }} <bdi translate="no">{{ change.after ?? t('settingsPage.yamlNotRead') }}</bdi></p>
              </li>
            </ul>
            <p v-if="world.yamlPreflight?.state === 'Unverified'" class="hint">{{ t('settingsPage.yamlNoData') }}</p>
            <p v-else-if="!world.yamlPreflight" class="hint">{{ t('settingsPage.yamlRecheck') }}</p>
            <p v-else-if="!world.yamlPreflight.changes.length && !world.yamlPreflight.filesChanged?.length" class="hint">{{ t('settingsPage.yamlMatches') }}</p>
          </details>
          <p v-if="worldLiveError" class="form-error" role="alert">{{ worldLiveError }}</p>
          <WorldLiveControls v-if="instance" :name="instance" :run="instanceRun" :ready="!!worldCommandReady"
            :supported="!!worldLiveValues.initialTime" :connected="connected" :saving="saving" @mutating="emit('mutating')"/>
          <PolicyFields mode="world" :rows="world.rows" :values="{...worldValues, ...worldRuleDraft}"
            :stopped="instanceState==='Stopped'" :live-values="worldLiveValues" @change="setWorldValue"/>
        </template>
        <OperationButton v-else-if="error" :label="t('action.retryRead')" :busy-label="t('activity.loading')"
          :busy="retryingRead" @click="retrySettings" />
      </template>
      <template v-else-if="tab==='restart'">
        <h2>{{ t('settings.restart') }}</h2>
        <p v-if="instanceState!=='Stopped'" class="notice">{{ t('settingsPage.pendingApplyNotice') }}</p>
        <p v-if="instancePending" class="hint" role="status">{{ instancePending.state === 'Conflict' ? t('settingsPage.pendingConflict') : t('settingsPage.pendingKept') }}</p>
        <div class="field"><label for="stop-seconds">{{ t('settingsPage.stopTimeout') }}</label><input id="stop-seconds" name="stop-seconds" v-model.number="timing.stopSeconds" type="number" min="5" max="300" placeholder="45" autocomplete="off" :disabled="!baseline" @wheel="wheelToScroll"></div>
        <div class="field"><label for="ready-seconds">{{ t('settingsPage.readyTimeout') }}</label><input id="ready-seconds" name="ready-seconds" v-model.number="timing.readySeconds" type="number" min="10" max="600" placeholder="150" autocomplete="off" :disabled="!baseline" @wheel="wheelToScroll"></div>
        <div class="field"><label for="restart-delay">{{ t('settingsPage.restartBuffer') }}</label><input id="restart-delay" name="restart-delay" v-model.number="timing.restartDelayMs" type="number" min="0" max="5000" placeholder="0" autocomplete="off" :disabled="!baseline" @wheel="wheelToScroll"></div>
      </template>
      <template v-else-if="tab==='autorestart'">
        <h2>{{ t('settings.autoRestart') }}</h2>
        
        <div v-if="restartStatus" class="hint" role="status">
          <p>{{ t(`settings.restartState.${restartStatus.phase}`) }}</p>
          <p v-if="restartAttemptCount!==undefined">{{ t('settings.restartAttemptCount',{ count:restartAttemptCount }) }}</p>
          <p v-if="restartTime">{{ t('settings.restartDue',{ time:restartTime }) }}</p>
        </div>
        <p v-if="instanceState!=='Stopped'" class="notice">{{ t('settingsPage.autoRestartNotice') }}</p>
        <fieldset :disabled="!baseline" class="settings-fields">
          <label class="choice"><input v-model="autoRestart.onCrash" name="restart-crash" type="checkbox">{{ t('settingsPage.restartOnCrash') }}</label>
          <label class="choice"><input v-model="autoRestart.onUnexpectedExit" name="restart-exit" type="checkbox">{{ t('settingsPage.restartOnExit') }}</label>
          <label class="choice"><input v-model="autoRestart.advanced" name="restart-advanced" type="checkbox">{{ t('settings.restartAdvanced') }}</label>
          <div class="field">
            <label for="auto-limit">{{ t('settings.restartLimit') }}</label>
            <input id="auto-limit" v-model.number="autoRestart.maxAttempts" name="auto-limit" type="number"
              min="1" :max="autoRestart.advanced?10:5" autocomplete="off" @wheel="wheelToScroll">
          </div>
          <div v-if="!autoRestart.advanced" class="field">
            <label for="auto-delay">{{ t('settings.restartDelay') }}</label>
            <input id="auto-delay" v-model.number="autoRestart.delaySeconds" name="auto-delay" type="number"
              min="2" max="600" autocomplete="off" @wheel="wheelToScroll">
          </div>
          <template v-if="autoRestart.advanced">
            <div class="field">
              <label for="auto-delays">{{ t('settings.restartSchedule') }}</label>
              <div class="editor">
                <input id="auto-delays" v-model="autoRestart.delays" name="auto-delays" type="text" maxlength="49"
                  autocomplete="off" spellcheck="false" aria-describedby="auto-delays-hint">
                <small id="auto-delays-hint" class="hint">{{ t('settings.restartScheduleHelp') }}</small>
              </div>
            </div>
            <div class="field">
              <label for="auto-window">{{ t('settings.restartWindow') }}</label>
              <input id="auto-window" v-model.number="autoRestart.windowSeconds" name="auto-window" type="number"
                min="60" max="3600" autocomplete="off" @wheel="wheelToScroll">
            </div>
            <div class="field">
              <label for="auto-stable">{{ t('settings.restartStable') }}</label>
              <input id="auto-stable" v-model.number="autoRestart.stableSeconds" name="auto-stable" type="number"
                min="60" max="3600" autocomplete="off" @wheel="wheelToScroll">
            </div>
          </template>
          <p v-if="!autoRestartPayload && dirty" class="form-error" role="status">{{ t('settings.restartInvalid') }}</p>
        </fieldset>
      </template>
      <template v-else-if="tab==='terminal'">
        <h2>{{ t('settings.terminal') }}</h2>
        <p v-if="!preferencesReady && connected && !error" class="form-error" role="alert">{{ t('settingsPage.consoleUnverified') }}</p>
        <OperationButton v-if="connected && (!preferencesReady || preferencesConflict || error)" :label="t('action.retryRead')" :busy-label="t('activity.loading')"
          :busy="retryingRead" @click="retrySettings" />
        <fieldset class="settings-fields" :disabled="!preferencesReady">
          <DisplayPreferencesFields v-if="preferencesLoaded" v-model:time-format="timeFormat"
          v-model:console-title-template="instanceConsoleTitle" :show-time="false"
          :inherited-title="preferences.consoleTitleTemplate ?? '{instance}'"
          :title-values="{ instance: instance ?? '' }" />
        </fieldset>
      </template>
      <template v-else-if="tab==='scan'">
        <h2>{{ t('management.paths') }}</h2>
        <p id="scan-help" class="hint">{{ t('settingsPage.scanHelp') }}</p>
        <template v-if="scan">
          <p v-if="scan.policy.allow.length || scan.policy.exclude.length" class="hint">{{ t('settingsPage.scanNameRules', { allow: scanNameList(scan.policy.allow), exclude: scanNameList(scan.policy.exclude) }) }}</p>
          <ManagementPathRows v-for="kind in scanKinds" :key="kind" :kind="kind" :rows="scanDraft[kind]"
            :busy="scanPickerBusy || scopeSavingStart" :issues="scanIssues"
            @add="addScanPath(kind)" @edit="(id, path) => editScanPath(kind, id, path)"
            @picking="scanPickerBusy=$event" @failed="pathPickerFailed" @remove="id => removeScanPath(kind, id)" />
        </template>
        <OperationButton v-else-if="error" :label="t('action.retryRead')" :busy-label="t('activity.loading')"
          :busy="retryingRead" @click="retrySettings" />
      </template>
      <template v-else-if="tab==='global-java'">
        <h2>{{ t('management.globalJava') }}</h2>
        
        <template v-if="savedGlobalJava">
          <div class="field">
            <label for="global-java-mode">{{ t('management.javaSelection') }}</label>
            <select id="global-java-mode" v-model="globalJavaMode" :disabled="javaPathPickerBusy">
              <option value="automatic">{{ t('management.javaAuto') }}</option>
              <option value="manual">{{ t('management.javaManual') }}</option>
            </select>
          </div>
          <div class="field">
            <label for="global-java-path">{{ t('settingsPage.javaExecutable') }}</label>
            <div class="editor">
              <div class="settings-path-editor">
                <input id="global-java-path" ref="globalJavaPathInput" v-model.trim="globalJavaPath" name="global-java-path"
                  autocomplete="off" spellcheck="false" :disabled="globalJavaMode==='automatic'"
                  :readonly="javaPathPickerBusy">
                <PathPickerButton :current="globalJavaPath" kind="java" :disabled="javaPathPickerBusy || globalJavaMode==='automatic'"
                  @picking="javaPathPickerBusy=$event" @selected="setJavaFromPicker" @failed="pathPickerFailed" />
              </div>
              <button type="button" :disabled="!canChooseJava || javaPathPickerBusy" @click="showJavaPicker">
                {{ t('management.chooseInstalledJava') }}
              </button>
            </div>
          </div>
        </template>
        <OperationButton v-else-if="error" :label="t('action.retryRead')" :busy-label="t('activity.loading')"
          :busy="retryingRead" @click="retrySettings" />
        <JavaSearchScope ref="javaSearchScope" :connected="connected"
          :disabled="saving || javaPathPickerBusy" :path-label="pathLabel" @dirty="javaSearchDirty=$event"
          @results="openJavaDialog('results')" />
      </template>
      <template v-else-if="tab==='updates'">
        <h2>{{ t('settings.coreUpdates') }}</h2>
        <p v-if="!preferencesReady" class="form-error" role="alert">{{ t('settings.corePrefetchUnavailable') }}</p>
        <label class="check"><input v-model="corePrefetch" type="checkbox"> {{ t('settings.corePrefetch') }}</label>
        
      </template>
      <template v-else-if="tab==='test-admins'">
        <h2>{{ t('settingsPage.testAdmins') }}</h2>
        <TestAdminFields v-model:administrators="testAdminRows" v-model:luckperms="testAdminLuckperms" v-model:op="testAdminOp" v-model:kind="testAdminKind" v-model:nodes="testAdminNodes" v-model:group="testAdminGroup"
          :artifacts="testAdmins?.artifacts" :busy="saving || !testAdmins" :fetching="testAdminFetching" :connected="connected" :nodes-error="testAdminNodesError" @fetch="fetchLuckPerms" />
      </template>
      <RecoveryKeyPanel v-else-if="tab==='recovery'" :connected="connected" />
      <AboutPanel v-else-if="tab==='about'" />
      <template v-else><h2>{{ t('settings.core') }}</h2><button v-if="instance" type="button" :disabled="!connected" @click="emit('core',instance)">{{ t('settingsPage.openCoreMaintenance') }}</button></template>
      <p v-if="error && !pickerDialog?.open" role="alert" class="form-error">{{ error }}</p>
    </fieldset></section></div></OverlayScroll>
  <div v-if="tab!=='about'" class="settings-footer"><span role="status" :class="{'save-success':saveSucceeded,'save-warning':saveNeedsVerification,'save-failure':saveFailedMessage}">{{ message }}</span><div class="actions"><button :disabled="saving || !editableTab || (!baseline && !javaSearchDirty)" @click="reset" :title="instance?undefined:t('settings.restoreTitle')">{{ instance?t('action.cancelChanges'):t('action.restore') }}</button><button class="primary" :disabled="saving || !connected || !editableTab || (preferencesTab && (!preferencesReady || preferencesConflict)) || (tab==='appearance' && selectionOpacityInvalid) || (tab==='java' && !javaCanSave) || ((tab==='restart'||tab==='autorestart')&&(!settings.revision||!baseline)) || (tab==='autorestart' && !autoRestartPayload) || (tab==='world' && !worldCanSave) || (tab==='initialization'&&(!init || !dirty)) || (tab==='scan' && (!scan || scopeSavingStart || scanPickerBusy || !dirty)) || (tab==='global-java' && !globalJavaCanSave) || (tab==='test-admins' && (!testAdmins || !dirty || testAdminFetching))" @click="save()">{{ tab==='world' ? t('action.saveShort') : t('action.save') }}</button></div></div>
  <dialog ref="scopeConfirmDialog" class="scope-confirm" aria-labelledby="scope-confirm-title"
    @cancel.prevent="!saving && scopeConfirmDialog?.close()" @close="scopePlan=undefined">
    <h2 id="scope-confirm-title">{{ t('management.scopeConfirm') }}</h2>
    <OverlayScroll class="scope-confirm-scroll" :label="t('management.scopeConfirm')">
      <p v-if="!scopeChangedRows.length && !scopeCandidates.length">{{ t('management.scopeNoChanges') }}</p>
      <section v-if="scopeChangedRows.length">
        <h3>{{ t('management.scopeAffected') }}</h3>
        <ul>
          <li v-for="row in scopeChangedRows" :key="row.name">
            <strong>{{ row.name }}</strong> · {{ t(row.visible ? 'management.scopeIncluded' : 'management.scopeExcluded') }}
            <span v-if="!row.visible && row.active"> · {{ t('management.scopeActive') }}</span>
            <small class="hint selectable">{{ pathLabel(row.path) }}</small>
          </li>
        </ul>
      </section>
      <section v-if="scopeCandidates.length">
        <h3>{{ t('management.scopeCandidates') }}</h3>
        <ul>
          <li v-for="row in scopeCandidates" :key="row.directory">
            <strong>{{ row.name }}</strong> · {{ t(row.state === 'Ready' ? 'management.scopeRegister' : 'management.blocked') }}
            <small class="hint selectable">{{ pathLabel(row.directory) }}</small>
            <small v-if="row.reason" class="hint">{{ scopeCandidateReason(row.reason) }}</small>
            <small v-if="row.state === 'Ready' && row.warning" class="form-error" role="note">{{ scopeCandidateWarning(row.warning) }}</small>
          </li>
        </ul>
      </section>
    </OverlayScroll>
    <div class="dialog-actions">
      <button ref="scopeCancel" type="button" autofocus :disabled="saving" @click="scopeConfirmDialog?.close()">{{ t('action.cancel') }}</button>
      <button type="button" class="primary" :disabled="saving || !connected" @click="save(false,true)">{{ t('action.saveShort') }}</button>
    </div>
  </dialog>
  <dialog ref="worldConfirmDialog" class="compact" aria-labelledby="world-confirm-title" @close="worldSavePlan=undefined">
    <h2 id="world-confirm-title">{{ t('settingsPage.worldConfirmTitle') }}</h2>
    <ul v-if="worldReviewedChanges.length"><li v-for="change in worldReviewedChanges" :key="change.key">
      {{ worldFieldLabel(change.key) }} · {{ change.before || t('settingsPage.empty') }} → {{ change.after || t('settingsPage.empty') }}
      <small v-if="change.path" class="hint">{{ change.path }}</small>
    </li></ul>
    <p v-else>{{ t('settingsPage.worldDraftSame') }}</p>
    <p v-if="instanceState!=='Stopped'">{{ t('settingsPage.worldConfirmQueued') }}</p>
    <p v-if="worldSavePlan && Object.keys(worldSavePlan.rules).length">{{ t('settingsPage.worldConfirmRules', { count: Object.keys(worldSavePlan.rules).length }) }}</p>
    <div class="dialog-actions">
      <button autofocus @click="worldConfirmDialog?.close()">{{ t('action.cancel') }}</button>
      <button class="primary" :disabled="saving || !connected" @click="save(true)">{{ t('action.saveShort') }}</button>
    </div>
  </dialog>
  <dialog ref="pickerDialog" aria-labelledby="java-picker-title" @close="handleJavaPickerClose">
    <h2 id="java-picker-title">{{ t(javaDialogMode==='choose' ? 'management.javaPickerTitle' : 'management.javaResultsTitle') }}</h2>
    <p v-if="javaPickerDraft" class="hint">{{ t('management.javaPickerPreview') }}</p>
    <button class="java-rescan" type="button" :aria-disabled="javaBusy" @click="refreshJavaPicker">
      <span class="java-rescan-label">{{ t(javaBusy ? 'management.discoveringJava' : 'management.rediscoverJava') }}<span v-if="javaBusy" class="java-rescan-spinner" aria-hidden="true"></span></span>
    </button>
    <span class="sr-only" role="status">{{ javaBusy ? t('management.discoveringJava') : '' }}</span>
    <div v-if="!pickerChecked && !javaPickerError && connected" class="java-search-wait">
      <div class="java-search-progress" role="progressbar" :aria-label="t('management.discoveringJava')"></div>
    </div>
    <p v-if="pickerChecked && !candidates.length && !javaPickerError" class="notice">{{ t(javaDialogMode==='choose' ? 'management.javaPickerEmpty' : 'management.javaResultsEmpty') }}</p>
    <OverlayScroll v-if="candidates.length" class="java-list-scroll" :label="t('settingsPage.javaInstalls')">
      <component :is="javaDialogMode==='choose' ? 'div' : 'ul'" class="button-list" :class="{ 'java-results-list': javaDialogMode==='results' }">
        <component :is="javaDialogMode==='choose' ? 'button' : 'li'" v-for="candidate in candidates" :key="candidate.path"
          v-bind="javaDialogMode==='choose' ? {type:'button', disabled:!candidate.valid,
            'aria-pressed':pendingJava===candidate.path, onClick:()=>{pendingJava=candidate.path}} : {}">
          {{ candidate.version ?? t('settingsPage.javaVersionUnknown') }} · {{ candidate.source === 'custom' ? t('management.javaCustomSource') : candidate.source }}
          <small v-if="candidate.vendor || candidate.architecture">{{ [candidate.vendor,candidate.architecture].filter(Boolean).join(' · ') }}</small>
          <small>{{ pathLabel(candidate.path) }}{{ candidate.valid?'':' · '+t('settingsPage.javaUnusable') }}</small>
        </component>
      </component>
    </OverlayScroll>
    <p v-if="javaPickerError" role="alert" class="form-error">{{ javaPickerError }}</p>
    <div class="dialog-actions">
      <button type="button" autofocus @click="pickerDialog?.close()">{{ t(javaDialogMode==='choose' ? 'action.cancel' : 'action.close') }}</button>
      <button v-if="javaDialogMode==='choose'" type="button" class="primary" :disabled="!selectedJavaValid" @click="useJava">{{ t('management.javaPickerUse') }}</button>
    </div>
  </dialog>
  <dialog ref="discardDialog" class="compact" aria-labelledby="discard-settings-title" @cancel.prevent="discardDialog?.close()"><h2 id="discard-settings-title">{{ t('action.discard') }}</h2><p>{{ t('settings.unsaved') }}</p><div class="dialog-actions"><button autofocus @click="discardDialog?.close()">{{ t('action.cancel') }}</button><button class="danger" @click="discardAndContinue">{{ t('action.discard') }}</button></div></dialog>
</template>

<style scoped>
.java-search-wait { min-height: 210px; display: grid; align-items: center; margin-top: 12px; }
.java-search-progress { height: .375rem; overflow: hidden; border-radius: 1rem; background: var(--raised); }
.java-search-progress::before {
  content: ''; display: block; width: 35%; height: 100%; border-radius: inherit; background: var(--primary);
  animation: java-search-progress 1.2s ease-in-out infinite alternate;
}
@keyframes java-search-progress { to { transform: translateX(186%); } }
.java-results-list { margin: 0; padding: 0; list-style: none; }
.java-results-list li { padding: .5rem .625rem; border-block-end: 1px solid var(--border); overflow-wrap: anywhere; }
.java-results-list li:last-child { border-block-end: 0; }
.java-results-list small { display: block; color: var(--muted); }
.java-rescan { display: inline-grid; place-items: center; min-inline-size: 7.5em; text-align: center; }
.java-rescan-label { display: inline-flex; align-items: center; gap: .3em; white-space: nowrap; }
.java-rescan-spinner {
  inline-size: .7em;
  block-size: .7em;
  border: 1.5px solid currentColor;
  border-inline-end-color: transparent;
  border-radius: 50%;
  animation: java-rescan-spin .8s linear infinite;
}
@keyframes java-rescan-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  .java-rescan-spinner { animation: none; }
  .java-search-progress::before { animation: none; }
}
.settings-path-editor { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .5rem; }
.scope-confirm[open] { display: flex; flex-direction: column; overflow: hidden; }
.scope-confirm-scroll { min-height: 0; }
.scope-confirm-scroll h3 { margin: .75rem 0 .375rem; font-size: 1rem; }
.scope-confirm-scroll ul { padding-inline-start: 1.25rem; }
.scope-confirm-scroll li { margin-block: .5rem; overflow-wrap: anywhere; }
.scope-confirm-scroll small { display: block; }

.jvm-editor {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: .5rem;
}
.jvm-editor :is(.editor, input, select, textarea) {
  min-width: 0;
}
.jvm-advanced {
  margin-block: .875rem;
}
.jvm-advanced > summary {
  min-height: 2.125rem;
  padding: .5rem .75rem;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--raised);
}
.jvm-advanced[open] > summary {
  margin-bottom: 0;
}
.jvm-advanced .field {
  border-bottom: 0;
  padding-bottom: 0;
}
.save-warning {
  color: var(--warn);
}
.world-preflight {
  margin-block: .75rem;
}
.world-preflight summary .hint {
  margin-inline-start: .5rem;
}
.world-preflight ul {
  margin: 0;
  padding-inline-start: 1.5rem;
}
.world-preflight code {
  margin-inline-start: .5rem;
}
.world-preflight-values {
  margin-block: .2rem .6rem;
  overflow-wrap: anywhere;
}
.selection-color-sample {
  display: inline-flex;
  align-items: center;
  gap: .5rem;
  width: fit-content;
  padding: .25rem .5rem;
  border: 1px solid var(--row-selection-edge, var(--selection-edge));
  border-radius: .25rem;
  background: var(--row-selection, var(--selection));
  color: var(--row-selection-ink, var(--selection-ink));
}
.selection-opacity-control {
  display: flex;
  align-items: center;
  gap: .75rem;
}
.selection-opacity-control input[type=range] {
  width: min(16rem, 75%);
  min-width: 0;
  flex: 1 1 8rem;
  accent-color: var(--primary);
}
@media (forced-colors: none) {
  @supports selector(input::-webkit-slider-thumb) {
    .selection-opacity-control input[type=range] {
      --opacity-thumb-size: .875rem;
      appearance: none;
      height: 1.5rem;
      margin: 0;
      padding: 0;
      border: 0;
      background: linear-gradient(to right, var(--primary) var(--opacity-fill), var(--control) 0)
        center / calc(100% - var(--opacity-thumb-size)) .375rem no-repeat;
    }
    .selection-opacity-control input[type=range]::-webkit-slider-thumb {
      appearance: none;
      width: var(--opacity-thumb-size);
      height: var(--opacity-thumb-size);
      border-radius: 50%;
      background: var(--primary);
    }
  }
}
.selection-opacity-control input[type=text] {
  width: 2.625rem;
  height: 2.625rem;
  flex: none;
  padding: .25rem;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
</style>
