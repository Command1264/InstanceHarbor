<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import brandLogo from './assets/instanceharbor-brand.png?no-inline'
import InstancesPage from './InstancesPage.vue'
import SettingsPage from './SettingsPage.vue'
import ActivityPage from './ActivityPage.vue'
import CoreMaintenanceDialog from './components/CoreMaintenanceDialog.vue'
import BatchCoreDialog from './components/BatchCoreDialog.vue'
import ManagementFolderDialog from './components/ManagementFolderDialog.vue'
import PathPickerButton from './components/PathPickerButton.vue'
import { displayPath } from './path-display'
import OverlayScroll from './components/OverlayScroll.vue'
import OperationButton from './components/OperationButton.vue'
import InitialQueryProgress from './components/InitialQueryProgress.vue'
import CreateCorePicker from './components/CreateCorePicker.vue'
import { catalogRefreshInterval, useCoreCatalog, type CatalogFamily } from './core-catalog'
import {validPinOrder} from './pin-order'
import {suppressBrowserMenu} from './context-menu'
import { provideAppLanguage, type LanguagePreference } from './i18n'
import { refreshSelectionColor, refreshSelectionTheme, resolveSelectionOpacity, resolveTheme, shouldRespondToSystemTheme } from './theme'
import { canOpenConsole, canStop, confirmCreation, coreBatchJob, errorText, manage, nativeBridge, rawError, type CoreBatch, type CoreSource, type CreationProgress, type Instance, type JavaCandidate, type Job, type Logs, type Preferences, type PreferencesSnapshot, type InstanceSnapshot, type RegistryIssue, type Presentation } from './manager'
import './app.css'

type Route='instances'|'activity'|'settings'
type Mode='create'|'force'|'remove'|'logs'|'batch'|null
const coreDialog = ref<InstanceType<typeof CoreMaintenanceDialog>>()
const coreTarget = ref<string>()
const managementFolderDialog = ref<InstanceType<typeof ManagementFolderDialog>>()
const batchCoreDialog = ref<InstanceType<typeof BatchCoreDialog>>()
async function openCore(name: string) {
  if (coreTarget.value) return
  coreTarget.value = name
  await nextTick()
  coreDialog.value?.open()
}
const route=ref<Route>('instances'), settingsName=ref<string>()
const rows=ref<Instance[]>([])
const listLoaded=ref(false)
const listReading=ref(false)
const presentation=ref<Presentation>({preferences:{theme:'system',pins:[]},instances:[]})
const languagePreview=ref<LanguagePreference>()
const { t }=provideAppLanguage(()=>presentation.value.preferences.language,()=>languagePreview.value)
const coreCheckRows=ref<Record<string,CoreSource>>({})
let coreEpoch=0
let lastCoreTargets=''
type CoreCheckReply = { rows: CoreSource[]; checkedAt: number }
const coreReads = new Map<string, Promise<CoreCheckReply>>()
function coreRead(names: string[], origin: 'background' | 'manual', join = true): Promise<CoreCheckReply> {
  if(join){
    const pending = [...new Set(names.map(name=>coreReads.get(name)).filter(value=>value!==undefined))]
    if(pending.length){
      const missing=names.filter(name=>!coreReads.has(name))
      if(missing.length)pending.push(coreRead(missing,origin,false))
      if(pending.length===1)return pending[0]!
      return Promise.allSettled(pending).then(results=>{
        const replies=results.flatMap(result=>result.status==='fulfilled'?[result.value]:[])
        if(!replies.length)throw results.find(result=>result.status==='rejected')?.reason
        const targets=new Set(names)
        return {rows:replies.flatMap(reply=>reply.rows.filter(row=>targets.has(row.name)).map(row=>({...row,checkedAt:reply.checkedAt}))),
          checkedAt:Math.max(...replies.map(reply=>reply.checkedAt))}
      })
    }
  }
  const promise = manage<CoreCheckReply>(['core-check',...names],origin)
  for (const name of names) coreReads.set(name,promise)
  void promise.then(release,release)
  return promise
  function release() {
    for (const name of names) if (coreReads.get(name)===promise) coreReads.delete(name)
  }
}
async function manuallyCheckCore(name:string):Promise<CoreCheckReply>{
  return coreRead([name],'manual')
}
function manuallyCheckCoreBatch(names:string[]){
  return coreRead(names,'manual')
}
async function checkCoreUpdates(names:string[], join = true){
  if(!names.length)return
  const epoch=++coreEpoch
  const targets=new Map<string,string[]>()
  const byName=new Map(presentation.value.instances.map(item=>[item.name,item]))
  for(const name of names){
    const core=byName.get(name)?.core
    const key=core?.state==='known'&&core.family&&core.minecraft
      ? core.family.toLowerCase()+'/'+core.minecraft : 'unverified'
    const group=targets.get(key)??[]
    group.push(name)
    targets.set(key,group)
  }
  await Promise.allSettled([...targets.values()].map(async group=>{
    try{
      const value=await coreRead(group,'background',join)
      if(epoch!==coreEpoch)return
      const next={...coreCheckRows.value}
      for(const row of value.rows){
        if(row.status!=='Unknown')next[row.name]={...row,checkedAt:row.checkedAt??value.checkedAt}
        else if(row.reason==='CoreIdentityChanged'||row.reason==='CoreTargetUnavailable')delete next[row.name]
      }
      if(JSON.stringify(next)!==JSON.stringify(coreCheckRows.value))coreCheckRows.value=next
    }catch(e){ console.error('背景核心核對失敗', e) }
  }))
}
async function refreshAfterCoreChange() {
  await Promise.all([refresh(), refreshJobs(false,true)])
  await checkCoreUpdates(presentation.value.instances.map(item=>item.name),false)
}
const registryIssues=ref<RegistryIssue[]>([])
const registryIssueLabels:Record<RegistryIssue['reason'],string>={
  FileInUse:'檔案被占用', AccessDenied:'拒絕存取', InvalidRecord:'登記內容無效', Unconfirmed:'無法核對', ObservationUnconfirmed:'實例狀態無法核對',
}
const listUnavailable=ref('')
const preferencesRevision=ref('')
function applyPreferences(snapshot:PreferencesSnapshot){
  if(JSON.stringify(snapshot.preferences)!==JSON.stringify(presentation.value.preferences)){
    presentation.value={...presentation.value,preferences:snapshot.preferences}
  }
  preferencesRevision.value=snapshot.revision
  preferencesLoaded.value=true
  preferencesUnavailable.value=false
}
const preferencesLoaded=ref(false)
const preferencesUnavailable=ref(false)
let preferencesRetrying=false
let preferencesReadEpoch=0
async function retryPreferences(){
  preferencesRetrying=true
  const epoch=++preferencesReadEpoch
  const previous=presentation.value.preferences
  try{
    const snapshot=await manage<PreferencesSnapshot>(['preferences-snapshot'],'manual')
    if(epoch===preferencesReadEpoch && presentation.value.preferences===previous){
      applyPreferences(snapshot)
    }
  }catch(error){
    if(epoch===preferencesReadEpoch)preferencesUnavailable.value=true
    throw error
  }finally{preferencesRetrying=false}
}
const visiblePreferences=computed(()=>{
  const names=new Set(presentation.value.instances.map(item=>item.name))
  return {...presentation.value.preferences,pins:presentation.value.preferences.pins.filter(name=>names.has(name))}
})
const jobsLoaded=ref(false), jobsReading=ref(false)
const jobs=ref<Job[]>([]), jobsError=ref(''), feedback=ref(''), activityFeedback=ref('')
const residentJobs=ref<Job[]>([])
const coreBatches=ref<CoreBatch[]>([])
const cancellingCoreBatch=ref<string>()
const coreCancelFailure=ref<string>()
let coreBatchRevision=0
const residentError=ref('')
const submittingBatch=ref(false)
const openingConsoles=ref(false)
const cancellingBatch=ref<string>()
let firstBatchRead=true
const failureDialog=ref<HTMLDialogElement>(), failureMessage=ref('')
const pendingOperationErrors:string[]=[]
function showOperationError(message:string){
  feedback.value=message
  if(failureMessage.value){if(message!==failureMessage.value&&!pendingOperationErrors.includes(message))pendingOperationErrors.push(message);return}
  failureMessage.value=message
  void nextTick(()=>{if(!failureDialog.value?.open)failureDialog.value?.showModal()})
}
const activeRequestIds=ref(new Set<string>())
const projectedJobs=computed(()=>[...residentJobs.value,...jobs.value,...coreBatches.value.map(coreBatchJob)].sort((a,b)=>(b.createdAt??0)-(a.createdAt??0)).map(job=>
  job.state==='Unconfirmed' && activeRequestIds.value.has(job.id) ? {...job,state:'Pending'} : job,
))
const connection=ref<'connected'|'disconnected'|'browser'|'unknown'>(nativeBridge?'unknown':'browser')
const modal=ref<Mode>(null), dialog=ref<HTMLDialogElement>(), cancelButton=ref<HTMLButtonElement>()
const modalError=ref(''), modalMessage=ref(''), modalErrorElement=ref<HTMLElement>()
const provisionBaseline=ref(''), discardProvisionDialog=ref<HTMLDialogElement>()
const settingsDirty=ref(false), pendingRoute=ref<Route|null>(null), routeDialog=ref<HTMLDialogElement>()
const settingsSaving=ref(false)
const target=ref<{name:string;run?:string}>(), logs=ref<Logs>()
const logSource = ref('terminal')
const selectedLog = computed(()=>logs.value?.sources?.find(source=>source.source===logSource.value))
const logLabels: Record<string,string> = {terminal:'標準輸出',stderr:'標準錯誤',host:'宿主診斷',control:'控制通道'}
const logAvailability: Record<string,string> = {Missing:'此工作階段沒有這份紀錄。',Empty:'紀錄目前沒有內容。',Unavailable:'目前無法讀取這份紀錄。'}
const logsReading=ref(false)
let lastLogs:{name:string;run:string;result:Logs}|undefined
const logsError=ref('')
const logsMessage=ref('')
const modalBusy=ref(false), forceBusy=ref(false)
const busyNames=ref(new Set<string>()), stopping=ref<Record<string,string>>({})
const manualStarts=ref(new Set<string>())
const residentItems=computed(()=>residentJobs.value.flatMap(job=>job.items??[]))
const queuedStarts=computed(()=>new Set(residentItems.value
  .filter(item=>item.phase==='Queued').map(item=>item.name)))
const activeStops=computed(()=>Object.fromEntries(residentItems.value
  .filter(item=>item.runId && ['StopQueued','StopDispatching','Stopping'].includes(item.phase))
  .map(item=>[item.name,item.runId!] as const)))
const residentStarting=computed(()=>new Set([...manualStarts.value,
  ...residentItems.value.filter(item=>item.phase==='Starting'||item.phase==='Dispatching').map(item=>item.name)]))
const stopControls=computed(()=>({...stopping.value,...activeStops.value}))
const lifecycleBusy=computed(()=>new Set([...busyNames.value,
  ...residentItems.value.filter(item=>['StopQueued','StopDispatching','Stopping'].includes(item.phase)).map(item=>item.name)]))
const catalog = useCoreCatalog()
const draft = reactive({
  name: '', parent: 'instances', port: 12010, core: '', source: '' as '' | CatalogFamily, version: '',
})
const creationVersions = computed(() => draft.source ? catalog.versions[draft.source] : undefined)
const creationBuild = computed(() => draft.source && draft.version
  ? catalog.buildEntry(draft.source, draft.version) : undefined)
const canPreviewCreation = computed(() => !draft.source
  || (!!creationVersions.value?.value?.includes(draft.version) && !!creationBuild.value?.value))

function refreshCreationVersions(manual = false) {
  if (draft.source) void catalog.refreshVersions(draft.source, manual)
}
function refreshCreationBuild(manual = false) {
  if (draft.source && creationVersions.value?.value?.includes(draft.version)) {
    void catalog.refreshBuild(draft.source, draft.version, manual)
  }
}
function refreshCatalog() {
  void catalog.refreshVersions('PAPER')
  void catalog.refreshVersions('FOLIA')
  if (modal.value === 'create') refreshCreationBuild()
}
watch(() => [draft.source, draft.version] as const, ([family, version], [previous]) => {
  if (family && family !== previous && catalog.versions[family].value === undefined) refreshCreationVersions()
  if (version) refreshCreationBuild()
})

type CreationPreview = {
  target: string
  parent: string
  whitelist: string | null
  core: { family: string; minecraft: string; build: string | null; sha256: string }
  javaMajor?: number | null
  initializationDefaults?: boolean
  revision: string
}
const creationPreview=ref<CreationPreview>()
const creationProgress=ref<CreationProgress>()
const creationPhase=computed(()=>{
  switch(creationProgress.value?.phase){
    case 'coreBytes': return 'downloadingCore'
    case 'mojangBytes': return 'downloadingMojang'
    default: return creationProgress.value?.phase
  }
})
const creationBytes=computed(()=>{
  const update=creationProgress.value
  return update && 'total' in update ? update : undefined
})
const previewedCreation=ref<ReturnType<typeof creationInput>>()
const creationUnconfirmedWork=ref('')
const createEditingPicker=ref(false)
const createBackButton=ref<HTMLButtonElement>()
const createNameInput=ref<HTMLInputElement>()
const createNameChecked=ref(false)
const createNameIssue=computed(()=>{
  const {name}=draft
  if(!name)return '請輸入實例名稱。'
  if(name.length>64)return '名稱最多可用 64 個字元。'
  if(name.includes('.'))return '句點請改用連字號，例如 test-paper-26-3。'
  if(!/^[a-z0-9]/.test(name))return '請以小寫英文字母或數字開頭。'
  if(!/^[a-z0-9-]+$/.test(name))return '請使用小寫英文字母、數字或連字號。'
  if(/^(?:con|prn|aux|nul|(?:com|lpt)[1-9])$/.test(name)){
    return '這是 Windows 保留名稱，請改用其他實例名稱。'
  }
  return ''
})
function validateCreationName(){
  createNameChecked.value=true
  if(!createNameIssue.value)return true
  createNameInput.value?.focus()
  return false
}
const createPath=(path:string)=>displayPath(path,visiblePreferences.value.pathSeparator)
function creationInput() {
  const build = creationBuild.value?.value
  if (!canPreviewCreation.value || (draft.source && !build)) return
  return {
    name:draft.name,
    parent:draft.parent,
    port:draft.port,
    source: draft.source && build ? { kind: 'official', ...build } : { kind: 'local', path: draft.core },
  }
}
function pickerFailed() {
  void showModalError(t('management.pickFailed'))
}
function cancelCreationPreview() {
  creationPreview.value=undefined
  previewedCreation.value=undefined
  creationUnconfirmedWork.value=''
  modalError.value=''
  modalMessage.value=''
}
watch(draft,cancelCreationPreview,{flush:'sync'})
const removeChoice=ref<'retain'|'delete'>('retain')
const pendingBatch=ref<{action:'start'|'stop'|'restart';targets:Instance[]}>()
const javaCandidates=ref<JavaCandidate[]>([])
const javaChecked=ref(false)
let javaRequest:Promise<void>|undefined
async function discoverJava(origin:'manual'|'background'='background',fresh=false){
  if(!javaRequest || fresh){
    const request=manage<JavaCandidate[]>(['discover-java'],origin).then(found=>{
      if(javaRequest!==request)return
      if(JSON.stringify(javaCandidates.value)!==JSON.stringify(found))javaCandidates.value=found
      javaChecked.value=true
    }).catch(error=>{
      if(fresh && javaRequest===request){
        // A snapshot from the previous search scope can contain excluded Java.
        javaCandidates.value=[]
        javaChecked.value=false
      }
      throw error
    }).finally(()=>{if(javaRequest===request)javaRequest=undefined})
    javaRequest=request
  }
  try{await javaRequest}
  catch(e){
    if(origin==='manual')throw e
    console.error('背景 Java 探索失敗',e)
  }
}
const connected=computed(()=>connection.value==='connected')
const currentSettingsRow=computed(()=>rows.value.find(row=>row.name===settingsName.value))
const currentSettingsState=computed(()=>currentSettingsRow.value?.state)
const worldCommandReady=computed(()=>currentSettingsRow.value?.state==='Listening'
  && currentSettingsRow.value.consoleReady===true
  && !currentSettingsRow.value.stopPhase && !currentSettingsRow.value.stopToken)
const worldRuleWork=computed(()=>JSON.stringify(jobs.value
  .filter(job=>job.instance===settingsName.value && job.action==='world-live-rule')
  .map(({id,state,message})=>[id,state,message])))
const currentSettingsMeta=computed(()=>presentation.value.instances.find(item=>item.name===settingsName.value))
watch(()=>presentation.value.instances.map(item=>item.name).join(','),names=>{
  if(nativeBridge && lastCoreTargets!==names){
    lastCoreTargets=names
    const targets=names ? names.split(',') : []
    const active=new Set(targets)
    if(Object.keys(coreCheckRows.value).some(name=>!active.has(name))){
      coreCheckRows.value=Object.fromEntries(Object.entries(coreCheckRows.value).filter(([name])=>active.has(name)))
    }
    if(targets.length)void checkCoreUpdates(targets)
    else coreEpoch++
  }
})
let refreshActive=false, refreshQueued=false, queuedManual=false, refreshEpoch=0, forceGeneration=0
let jobsEpoch=0
let jobsRead:Promise<void>|undefined
let jobsManual=false
let jobsFollowup=false
let residentRevision=0
let logsEpoch=0
let windowReady=false
let windowOpening=false
async function revealWindow(){
  if(!nativeBridge || windowReady || windowOpening)return
  windowOpening=true
  try{
    await nextTick()
    await invoke('present_window',{theme:presentation.value.preferences.theme})
    windowReady=true
  }catch(e){
    feedback.value='無法顯示桌面視窗：'+errorText(e)
  }finally{windowOpening=false}
}
let timer:ReturnType<typeof setInterval>|undefined
let refreshFeedbackTimer:ReturnType<typeof setTimeout>|undefined
// Manual refresh state is kept apart from operation feedback so neither overwrites the other.
const manualRefreshPending=ref(false), manualRefreshDone=ref(false)
// Background polls publish a failure only after repeated misses; the last verified list stays.
const BACKGROUND_FAILURE_LIMIT=3
let backgroundFailures=0
function clearRefreshFeedback(){clearTimeout(refreshFeedbackTimer);refreshFeedbackTimer=undefined;manualRefreshDone.value=false}
let diagnosticFailureReported=false
let pendingDiagnosticWarning=false
const diagnosticWarning='無法完整寫入應用程式診斷紀錄。請檢查磁碟空間及 %LOCALAPPDATA%/InstanceHarbor/v2/logs 的容量與存取狀態；既有紀錄不會自動刪除。'
function presentDiagnosticWarning(){
  if(pendingDiagnosticWarning && !failureMessage.value && !failureDialog.value?.open){pendingDiagnosticWarning=false;showOperationError(diagnosticWarning)}
}
function afterFailureClose(){failureMessage.value='';const pending=pendingOperationErrors.shift();if(pending)showOperationError(pending);else void nextTick(presentDiagnosticWarning)}
async function checkDiagnostics(){
  if(!nativeBridge||diagnosticFailureReported)return
  try{
    const status=await invoke<{available:boolean;error?:string}>('diagnostic_status')
    if(!status.available){diagnosticFailureReported=true;pendingDiagnosticWarning=true;void nextTick(presentDiagnosticWarning)}
  }catch(error){console.error('無法核對診斷紀錄狀態',error)}
}
let pollTick=0
const requests=new Map<string,string>()
function requestId(key:string){if(!requests.has(key))requests.set(key,crypto.randomUUID());return requests.get(key)!}
function markActiveRequest(id:string,active:boolean){
  const next=new Set(activeRequestIds.value)
  if(active)next.add(id)
  else next.delete(id)
  activeRequestIds.value=next
}
function applyTheme(theme:Preferences['theme']){
  document.documentElement.dataset.theme=resolveTheme(theme,window.matchMedia('(prefers-color-scheme: dark)').matches)
  refreshSelectionTheme()
  if(nativeBridge && windowReady)void invoke('present_window',{theme}).catch(e=>console.error('無法套用視窗主題',e))
}
watch(()=>presentation.value.preferences.theme,theme=>{
  if(route.value!=='settings')applyTheme(theme)
},{immediate:true})
watch([()=>presentation.value.preferences.selectionColor,()=>presentation.value.preferences.selectionOpacity],([selection, opacity])=>{
  if(route.value!=='settings')void refreshSelectionColor(selection,nativeBridge,opacity)
},{immediate:true})
const media=window.matchMedia('(prefers-color-scheme: dark)')
const themeListener=()=>{
  const theme=presentation.value.preferences.theme
  if(shouldRespondToSystemTheme(theme,route.value==='settings'))applyTheme(theme)
}
const accentListener=()=>{
  if(document.documentElement.dataset.selectionColor==='system')
    void refreshSelectionColor('system',nativeBridge,resolveSelectionOpacity(Number(document.documentElement.dataset.selectionOpacity)))
}
async function refresh(manual=false){
  if(!nativeBridge){connection.value='browser';if(manual)feedback.value='此瀏覽器沒有原生管理橋接。請使用 InstanceHarbor 桌面程式。';return}
  if(refreshActive){
    if(manual && manualRefreshPending.value)return
    if(manual){clearRefreshFeedback();manualRefreshPending.value=true}
    refreshQueued=true;queuedManual ||= manual;return
  }
  refreshActive=true;listReading.value=true;const epoch=++refreshEpoch
  if(manual){clearRefreshFeedback();manualRefreshPending.value=true}
  try{
    const origin=manual?'manual':'background'
    const shouldReadPreferences=!preferencesRetrying && (manual || !preferencesLoaded.value || preferencesUnavailable.value)
    const preferenceEpoch=preferencesReadEpoch
    const preferencesRead=shouldReadPreferences
      ? manage<PreferencesSnapshot>(['preferences-snapshot'],origin).then(
        snapshot=>{
          if(epoch!==refreshEpoch || preferencesRetrying || preferenceEpoch!==preferencesReadEpoch)return
          applyPreferences(snapshot)
        },
        ()=>{
          if(epoch===refreshEpoch && !preferencesRetrying && preferenceEpoch===preferencesReadEpoch)preferencesUnavailable.value=true
        },
      )
      : Promise.resolve()
    void preferencesRead.then(revealWindow)
    const [listResult]=await Promise.allSettled([
      manage<InstanceSnapshot>(['instance-snapshot'],origin),
      preferencesRead,
    ])
    if(epoch!==refreshEpoch)return
    if(listResult.status==='rejected')throw listResult.reason
    const {rows:nextRows,issues,instances,capabilities}=listResult.value
    const nextPresentation={instances,capabilities,preferences:presentation.value.preferences}
    if(JSON.stringify(issues)!==JSON.stringify(registryIssues.value))registryIssues.value=issues
    listUnavailable.value=''
    if(JSON.stringify(nextRows)!==JSON.stringify(rows.value))rows.value=nextRows
    if(JSON.stringify(nextPresentation)!==JSON.stringify(presentation.value))presentation.value=nextPresentation
    const live=new Map(nextRows.map(row=>[row.name,row]))
    for(const [name,run] of Object.entries(stopping.value))if(!canStop(live.get(name))||live.get(name)?.run?.id!==run)delete stopping.value[name]
    connection.value='connected'
    listLoaded.value=true
    backgroundFailures=0
    if(manual){
      clearRefreshFeedback()
      if(issues.length)feedback.value='已核對可讀取的實例'
      else if(preferencesUnavailable.value)feedback.value='實例清單已核對，無法讀取外觀設定'
      manualRefreshDone.value=true
      refreshFeedbackTimer=setTimeout(()=>{manualRefreshDone.value=false;refreshFeedbackTimer=undefined},1000)
    }
  }catch(e){
    if(epoch!==refreshEpoch)return
    if(!manual && listLoaded.value && ++backgroundFailures<BACKGROUND_FAILURE_LIMIT)return
    const cause=rawError(e)
    if(/ManagerUnavailable|ManagerPipeUnavailable|ManagerResultUnconfirmed|PeerMismatch|MissingWorkspaceBinding/.test(cause)){
      connection.value='disconnected'
    }
    listUnavailable.value='無法讀取實例清單：'+errorText(e)
  }
  finally{
    if(manual && !queuedManual)manualRefreshPending.value=false
    refreshActive=false
    listReading.value=false
    void checkDiagnostics()
    if(refreshQueued){
      const nextManual=queuedManual
      refreshQueued=false
      queuedManual=false
      void refresh(nextManual)
    }
  }
}
function invalidateRefresh(){
  refreshEpoch++
  if(refreshActive)refreshQueued=true
}
let jobsBackgroundFailures=0
let activityFeedbackTimer:ReturnType<typeof setTimeout>|undefined
function refreshJobs(manual=false,afterMutation=false):Promise<void>{
  if(manual){jobsManual=true;clearTimeout(activityFeedbackTimer);activityFeedback.value='正在讀取操作紀錄'}
  // Background polling joins the current read instead of invalidating slow replies.
  if(jobsRead){jobsFollowup ||= afterMutation;return jobsRead}
  jobsReading.value=true
  jobsRead=(async()=>{
    do{
      jobsFollowup=false
      const epoch=++jobsEpoch
      const residentAtRead=residentRevision
      const coreBatchAtRead=coreBatchRevision
      try{
        const origin=jobsManual?'manual':'background'
        const batchAction=firstBatchRead||jobsManual?'batch-connect':'batch-list'
        firstBatchRead=false
        const [legacy, batches, maintenance]=await Promise.allSettled([
          manage<Job[]>(['jobs'],origin),manage<Job[]>([batchAction],origin),manage<CoreBatch[]>(['core-batch-list'],origin)
        ])
        if(epoch!==jobsEpoch)return
        if(coreBatchAtRead===coreBatchRevision && maintenance.status==='fulfilled'){
          if(JSON.stringify(maintenance.value)!==JSON.stringify(coreBatches.value))coreBatches.value=maintenance.value
          if(maintenance.value.some(batch=>batch.state==='Running')){
            void manage(['manager-connect'],'background').catch(e=>console.error('無法連線至核心維護工作',e))
          }
        }
        if(residentAtRead===residentRevision && batches.status==='fulfilled'){
          if(JSON.stringify(batches.value)!==JSON.stringify(residentJobs.value))residentJobs.value=batches.value
          residentError.value=''
        }else if(residentAtRead===residentRevision && batches.status==='rejected' && (jobsManual || !jobsLoaded.value || jobsBackgroundFailures+1>=BACKGROUND_FAILURE_LIMIT))residentError.value='無法核對批次工作：'+errorText(batches.reason)
        if(legacy.status==='rejected')throw legacy.reason
        if(JSON.stringify(legacy.value)!==JSON.stringify(jobs.value))jobs.value=legacy.value
        if(maintenance.status==='rejected' && (jobsManual || !jobsLoaded.value)){
          jobsError.value='無法讀取核心維護紀錄：'+errorText(maintenance.reason)
        }else if(maintenance.status==='fulfilled')jobsError.value=''
        if(batches.status==='fulfilled' && maintenance.status==='fulfilled'){jobsLoaded.value=true;jobsBackgroundFailures=0}
        else if(!jobsManual)jobsBackgroundFailures++
        if(jobsManual){
          activityFeedback.value=batches.status==='fulfilled' && maintenance.status==='fulfilled'?'操作紀錄已讀取':''
          activityFeedbackTimer=setTimeout(()=>{if(activityFeedback.value==='操作紀錄已讀取')activityFeedback.value=''},1000)
        }
      }catch(e){
        if(epoch!==jobsEpoch)continue
        if(!jobsManual && jobsLoaded.value && ++jobsBackgroundFailures<BACKGROUND_FAILURE_LIMIT)continue
        jobsError.value='無法讀取操作紀錄：'+errorText(e)
        if(jobsManual)activityFeedback.value=''
      }
    }while(jobsFollowup)
  })().finally(()=>{jobsRead=undefined;jobsManual=false;jobsReading.value=false})
  return jobsRead
}
function navigate(next:Route){
  if(route.value==='settings' && settingsSaving.value){
    feedback.value='設定仍在儲存，請等待結果。'
    return
  }
  if(route.value==='settings' && settingsDirty.value && (next!==route.value || !!settingsName.value)){
    pendingRoute.value=next
    routeDialog.value?.showModal()
    return
  }
  route.value=next
  settingsName.value=undefined
  if(next==='activity')void refreshJobs()
}
function confirmRoute(){
  routeDialog.value?.close()
  settingsDirty.value=false
  if(pendingRoute.value)navigate(pendingRoute.value)
  pendingRoute.value=null
}
function open(next:Mode,item?:{name:string;run?:string}){
  logsEpoch++
  modal.value=next
  target.value=item
  modalBusy.value=false
  modalError.value=''
  modalMessage.value=''
  logs.value=undefined
  logsError.value=''
  logsMessage.value=''
  if(next==='create'){createNameChecked.value=false;cancelCreationPreview();Object.assign(draft,{name:'',parent:'instances',port:12010,core:'',source:'',version:''})}
  provisionBaseline.value=JSON.stringify(draft)
  if(next==='remove')removeChoice.value='retain'
  void nextTick(()=>{dialog.value?.showModal();cancelButton.value?.focus()})
  if(next==='logs'&&item){
    logSource.value='terminal'
    const run=rows.value.find(row=>row.name===item.name)?.run?.id
    if(run && lastLogs?.name===item.name && lastLogs.run===run)logs.value=lastLogs.result
    void loadLogs(item.name,run)
  }
}
function close(){
  if(modalBusy.value||createEditingPicker.value||(modal.value==='batch'&&submittingBatch.value))return
  if(modal.value==='create' && JSON.stringify(draft)!==provisionBaseline.value){
    discardProvisionDialog.value?.showModal()
    return
  }
  dialog.value?.close()
  modal.value=null
  logsEpoch++
}
function discardProvision(){
  discardProvisionDialog.value?.close()
  dialog.value?.close()
  modal.value=null
  logsEpoch++
}
async function showModalError(message:string){
  modalMessage.value=''
  modalError.value=message
  if(modal.value!=='create'){
    await nextTick()
    modalErrorElement.value?.focus()
  }
}
async function loadLogs(name:string, run?:string){
  const epoch=++logsEpoch
  const current=()=>epoch===logsEpoch && modal.value==='logs' && target.value?.name===name
  logsReading.value=true
  try{
    const result=await manage<Logs>(['logs',name])
    if(!current())return
    logs.value=result
    if(result.run && result.run===run)lastLogs={name,run,result}
  }catch(e){
    if(current())logsError.value=rawError(e).includes('LogRunChanged')
      ? '工作階段已變更，請重新開啟啟動紀錄。'
      : '無法讀取啟動紀錄：'+errorText(e)
  }finally{
    if(current())logsReading.value=false
  }
}
async function openConsoles(selection:Instance[]){
  if(openingConsoles.value || !connected.value)return
  const targets=selection.filter(canOpenConsole)
  if(!targets.length)return
  openingConsoles.value=true
  try {
    const failures:string[]=[]
    for(const row of targets){
      try {
        await manage(['console-open',row.name,row.run.id])
      } catch(error) {
        failures.push(row.name+'：'+errorText(error))
      }
    }
    if(failures.length)showOperationError('部分或全部後台無法開啟。請核對以下實例的工作階段後再試。'+failures.join('；'))
  } finally {
    openingConsoles.value=false
  }
}
async function openLogFolder(){
  if(modal.value!=='logs'||!target.value)return
  const name=target.value.name
  const epoch=logsEpoch
  const current=()=>epoch===logsEpoch && modal.value==='logs' && target.value?.name===name
  try{
    await manage(['open-log-folder',name])
    if(current())logsMessage.value='已開啟紀錄資料夾'
  }catch(e){
    if(current())logsError.value='無法開啟紀錄資料夾：'+errorText(e)
  }
}
function markBusy(name:string,value:boolean){
  const next=new Set(busyNames.value)
  if(value)next.add(name)
  else next.delete(name)
  busyNames.value=next
}
async function operateOne(action:'start'|'stop'|'restart',row:Instance){
  if(!connected.value||lifecycleBusy.value.has(row.name)||queuedStarts.value.has(row.name)||(action!=='stop'&&residentStarting.value.has(row.name)))return
  if(action!=='stop' && row.outsideManagement)return
  const needsRun=action==='stop'||(action==='restart'&&row.state!=='Stopped')
  if(needsRun&&!row.run)return
  invalidateRefresh()
  const previousRun=row.run?.id??null
  const stopToken=row.stopToken??null
  const key=JSON.stringify([action,row.name,previousRun,stopToken])
  const id=requestId(key)
  const args=[action==='start'?'start-if-current':action,row.name,id]
  if(action==='start')args.push(JSON.stringify({previousRun,stopToken}))
  else if(needsRun)args.push(row.run!.id)
  if(action==='stop')stopping.value[row.name]=row.run!.id
  else manualStarts.value.add(row.name)
  markBusy(row.name,true)
  markActiveRequest(id,true)
  const generation=forceGeneration
  feedback.value=action==='stop'&&!row.consoleReady?'等待伺服器可接受停止指令':({start:'啟動',stop:'停止',restart:'重啟'}[action]+'處理中')
  try{
    await manage<Instance>(args)
    requests.delete(key)
    if(generation===forceGeneration)feedback.value=''
  }catch(e){
    const reason=rawError(e)
    if(action==='start' && /GlobalJavaIncompatible|GlobalJavaUnavailable|AutoJavaUnavailable/.test(reason))requests.delete(key)
    if(reason === 'AutoJavaUnavailable') requests.delete(key)
    if(['stop','restart'].includes(action)&&['StopTimeout','Timeout','HostUnavailable'].some(code=>reason.includes(code))){
      requests.delete(key)
      if(generation===forceGeneration)showOperationError(`${row.name} 停止結果未確認。請核對工作紀錄，必要時使用強制停止。`)
    }else if(generation===forceGeneration)showOperationError(`${row.name} 操作失敗：${errorText(e)}`)
  }finally{
    markActiveRequest(id,false)
    await refresh()
    manualStarts.value.delete(row.name)
    markBusy(row.name,false)
    void refreshJobs(false,true)
  }
}
async function submitResidentBatch(action:'start'|'stop'|'restart',targets:Instance[]){
  if(submittingBatch.value||!targets.length)return
  const snapshot=targets.map(row=>({name:row.name,guard:{previousRun:row.run?.id??null,stopToken:row.stopToken??null}}))
  const raw=JSON.stringify(snapshot)
  const verb='batch-'+action
  const key=verb+':'+raw
  const id=requestId(key)
  submittingBatch.value=true
  try{
    const work=await manage<Job>([verb,id,raw])
    requests.delete(key)
    residentRevision++
    residentJobs.value=[work,...residentJobs.value.filter(job=>job.id!==work.id)]
  }finally{
    submittingBatch.value=false
    void refreshJobs(false,true)
  }
}
async function cancelResidentBatch(id:string){
  if(cancellingBatch.value)return
  cancellingBatch.value=id
  try{
    const work=await manage<Job>(['batch-cancel',id])
    residentRevision++
    residentJobs.value=residentJobs.value.map(job=>job.id===id?work:job)
  }catch(e){showOperationError('無法取消批次的後續操作：'+errorText(e))}
  finally{cancellingBatch.value=undefined;void refreshJobs(false,true)}
}
function publishCoreBatch(batch:CoreBatch) {
  coreBatchRevision++
  const existing=coreBatches.value.findIndex(job=>job.id===batch.id)
  if(existing===-1)coreBatches.value=[batch,...coreBatches.value]
  else coreBatches.value=coreBatches.value.map(job=>job.id===batch.id?batch:job)
}
function coreBatchSubmitted(batch?:CoreBatch) {
  if(batch)publishCoreBatch(batch)
  void refreshJobs(false,true)
}
async function cancelCoreBatch(id:string) {
  if(cancellingCoreBatch.value)return
  cancellingCoreBatch.value=id
  coreCancelFailure.value=undefined
  try {
    publishCoreBatch(await manage<CoreBatch>(['core-batch-cancel',id]))
  }catch(e){
    coreCancelFailure.value=id
    console.error('Cancellation request could not be confirmed',e)
  }finally{
    cancellingCoreBatch.value=undefined
    void refreshJobs(false,true)
  }
}
function requestAction(action:'start'|'stop'|'restart',targets:Instance[],all=false){
  if(submittingBatch.value)return
  if(!all && targets.length===1){void operateOne(action,targets[0]!);return}
  pendingBatch.value={action,targets:[...targets]}
  open('batch')
}
async function confirmBatch(){
  const batch=pendingBatch.value
  if(!batch||submittingBatch.value)return
  try{await submitResidentBatch(batch.action,batch.targets)}
  catch(e){showOperationError('無法確認批次工作已受理：'+errorText(e))}
  finally{dialog.value?.close();modal.value=null;pendingBatch.value=undefined}
}
async function forceStop(){
  const item=target.value
  if(!item?.run||forceBusy.value)return
  invalidateRefresh()
  forceBusy.value=true
  modalBusy.value=true
  forceGeneration++
  const id=crypto.randomUUID()
  markActiveRequest(id,true)
  dialog.value?.close()
  modal.value=null
  feedback.value='正在核對程序並執行強制停止'
  try{
    await manage<{forced?:boolean}>(['force-stop',item.name,id,item.run])
    feedback.value=''
  }catch(e){showOperationError('無法確認強制停止：'+errorText(e)+'。請核對目前狀態後再試。')}
  finally{
    markActiveRequest(id,false)
    forceBusy.value=false
    modalBusy.value=false
    await refresh()
    void refreshJobs(false,true)
  }
}
async function previewCreation(){
  if(modalBusy.value||modal.value!=='create'||createEditingPicker.value)return
  if(!validateCreationName())return
  const input = creationInput()
  if (!input) return
  modalBusy.value=true
  modalError.value=''
  modalMessage.value='正在核對建立路徑'
  try{
    creationPreview.value=await manage<CreationPreview>(['gui-create-preview',JSON.stringify(input)])
    previewedCreation.value=input
  }catch(e){
    const reason = rawError(e)
    if (/CreationPreviewChanged|CoreNoStableBuild|CreationNoStableBuild/.test(reason)) refreshCreationBuild()
    await showModalError(creationError(e))
  }
  finally{modalBusy.value=false;modalMessage.value=''}
  if(creationPreview.value){
    await nextTick()
    createBackButton.value?.focus()
  }
}
function creationError(error: unknown) {
  const reason = rawError(error)
  if (reason.includes('InvalidCreationName')) {
    return '實例名稱不符合命名規則。請使用小寫英文字母、數字或連字號，避開 Windows 保留名稱。'
  }
  if (/CoreNoStableBuild|CreationNoStableBuild/.test(reason)) {
    return '這個版本目前沒有可下載的穩定建置。請另選版本。'
  }
  if (/CoreSourceTimeout|CoreSourceHttpError|CoreSourceTransportError|CoreDownloadUnverified/.test(reason)) {
    return '無法完整取得官方核心來源。請核對網路連線後重試，或選用已驗證的本機 JAR。'
  }
  if (/CoreArtifactDigestMismatch|CoreArtifactSizeMismatch/.test(reason)) {
    return '下載檔案與官方資料不符，無法建立實例。請重新預覽後重試。'
  }
  if (/CoreDownloadUnsupportedLayout|MojangMetadataUnverified/.test(reason)) {
    return '無法核對這個建置所需的 Mojang 檔案。請重新核對來源，或選擇其他版本。'
  }
  if (reason.includes('CoreDownloadCacheInvalid')) {
    return '工作區的核心下載副本無法核對。請選擇其他版本，或使用已核對的本機 JAR。'
  }
  if (reason.includes('MissingMojangArtifact')) {
    return '找不到與這個核心相符的 Mojang JAR。請將同版 JAR 放在核心所在的資料夾，或改選下載指定版本。'
  }
  if (reason.includes('GlobalJavaIncompatible')) {
    return '全域 Java 與這個核心版本不相容。請在「應用程式設定 → 全域 Java」切換為自動配對或選擇相容版本。'
  }
  if (/AutoJavaUnavailable|GlobalJavaUnavailable/.test(reason)) {
    return '找不到與這個核心相容的 Java。請在「應用程式設定 → 全域 Java」核對搜尋範圍或指定相容執行檔。'
  }
  return errorText(error)
}
async function provision(){
  if(modalBusy.value||modal.value!=='create'||!creationPreview.value||!previewedCreation.value)return
  const name=previewedCreation.value.name
  // A background catalog refresh cannot change a confirmed preview's exact artifact.
  const input=JSON.stringify({draft:previewedCreation.value,revision:creationPreview.value.revision})
  const key=JSON.stringify(['gui-create',name,input])
  const id=requestId(key)
  invalidateRefresh()
  modalBusy.value=true
  modalError.value=''
  creationProgress.value={phase:'checking'}
  markActiveRequest(id,true)
  try{
    await confirmCreation(id,input,update=>{creationProgress.value=update})
    requests.delete(key)
    dialog.value?.close()
    modal.value=null
    feedback.value=`${name} 已建立`
    await refresh()
    void refreshJobs(false,true)
  }catch(e){
    const reason=rawError(e)
    if(reason.includes('CreationPreviewChanged')){
      cancelCreationPreview()
      refreshCreationBuild()
    }
    if(/CreationPolicyChanged|CreationRecoveryUnconfirmed/.test(reason))creationUnconfirmedWork.value=id
    await showModalError(creationError(e))
  }finally{creationProgress.value=undefined;markActiveRequest(id,false);modalBusy.value=false}
}
const pinBusy=ref(false)
async function updatePins(pins:string[]){
  if(pinBusy.value || !connected.value || !preferencesLoaded.value || preferencesUnavailable.value)return
  pinBusy.value=true
  const visible=new Set(presentation.value.instances.map(instance=>instance.name))
  const hidden=presentation.value.preferences.pins.filter(name=>!visible.has(name))
  const next={...presentation.value.preferences,pins:[...pins,...hidden]}
  invalidateRefresh()
  try{
    const saved=await manage<PreferencesSnapshot>(['save-preferences',JSON.stringify(next),preferencesRevision.value])
    invalidateRefresh()
    applyPreferences(saved)
  }catch(e){showOperationError('無法儲存釘選：'+errorText(e));void refresh()}
  finally{pinBusy.value=false}
}
function savePinOrder(pins:string[],expected:string[]){
  if(!validPinOrder(visiblePreferences.value.pins,pins,expected)){showOperationError('釘選清單已變更，請重新調整順序。');return}
  void updatePins(pins)
}
function pin(name:string){
  const pins=[...visiblePreferences.value.pins]
  const index=pins.indexOf(name)
  if(index>=0)pins.splice(index,1)
  else pins.push(name)
  void updatePins(pins)
}
function movePin(name:string,direction:number){
  const pins=[...visiblePreferences.value.pins]
  const index=pins.indexOf(name),other=index+direction
  if(index<0||other<0||other>=pins.length)return
  const moved=pins[index]!
  pins[index]=pins[other]!
  pins[other]=moved
  void updatePins(pins)
}
async function removeManagement(){
  const item=target.value
  if(!item)return
  invalidateRefresh()
  modalBusy.value=true
  modalError.value=''
  modalMessage.value='正在移除管理'
  try{
    await manage(['remove-management',item.name,removeChoice.value])
    feedback.value=`${item.name} 已移除管理`
    dialog.value?.close()
    modal.value=null
    await refresh()
  }catch(e){
    await showModalError('無法移除管理：'+errorText(e))
  }
  finally{modalBusy.value=false}
}
let catalogTimer:ReturnType<typeof setInterval> | undefined
onMounted(() => {
  if (!nativeBridge) return
  refreshCatalog()
  catalogTimer = setInterval(refreshCatalog, catalogRefreshInterval)
})
onUnmounted(() => clearInterval(catalogTimer))
onMounted(()=>{
  document.addEventListener('contextmenu',suppressBrowserMenu)
  media.addEventListener('change',themeListener)
  window.addEventListener('focus',accentListener)
  if(nativeBridge)void refreshJobs()
  if(nativeBridge)void discoverJava()
  void refresh()
  timer=setInterval(()=>{
    void refresh()
    if(++pollTick%2===0)void refreshJobs()
  },1500)
})
onUnmounted(()=>{clearRefreshFeedback();document.removeEventListener('contextmenu',suppressBrowserMenu);media.removeEventListener('change',themeListener);window.removeEventListener('focus',accentListener);if(timer)clearInterval(timer)})
</script>
<template>
  <a class="skip" href="#view">跳至主要內容</a>
  <div class="app"><aside class="rail"><div class="brand"><img :src="brandLogo" width="16" height="16" alt=""><span>InstanceHarbor</span></div><nav aria-label="主要導覽"><button :aria-current="route==='instances'?'page':undefined" @click="navigate('instances')">{{ t('nav.instances') }}</button><button :aria-current="route==='activity'?'page':undefined" @click="navigate('activity')">{{ t('nav.activity') }}</button><button :aria-current="route==='settings'&&!settingsName?'page':undefined" @click="navigate('settings')">{{ t('nav.settings') }}</button></nav></aside>
    <main id="view" :class="{'route-settings':route==='settings'}"><div v-if="!nativeBridge" class="notice error" role="alert">此瀏覽器沒有原生管理橋接。請使用 InstanceHarbor 桌面程式；設計預覽另在 v2/preview 提供。</div>
      <aside v-if="route==='instances' && (registryIssues.length || listUnavailable)" class="registry-notice" role="status">
        <p v-if="listUnavailable">{{ listUnavailable }}</p>
        <template v-else>
          <p>部分實例無法核對。請檢查下列項目，再按「重新核對」。</p>
          <OverlayScroll class="registry-issues" label="無法核對的實例"><ul><li v-for="issue in registryIssues" :key="issue.file">{{ issue.file }} · {{ registryIssueLabels[issue.reason] }}</li></ul></OverlayScroll>
        </template>
      </aside>
      <InstancesPage v-if="route==='instances'" :rows="rows" :initial-loading="listReading && !listLoaded && connection==='unknown'" :loaded="listLoaded" :meta="presentation.instances" :core-check-rows="coreCheckRows" :preferences="visiblePreferences" :pin-busy="pinBusy || !preferencesLoaded || preferencesUnavailable" :busy-names="lifecycleBusy" :queued-names="queuedStarts" :starting-names="residentStarting" :stopping="stopControls" :force-busy="forceBusy" :console-busy="openingConsoles" :connected="connected" :jobs="projectedJobs" :feedback="feedback" :refresh-pending="manualRefreshPending" :refresh-done="manualRefreshDone" @action="requestAction" @batch-action="(action,targets)=>requestAction(action,targets,true)" @console="openConsoles" @force="row=>open('force',{name:row.name,run:row.run?.id})" @pin="pin" @pin-move="movePin" @pin-order="savePinOrder" @settings="name=>{settingsName=name;route='settings'}" @core="openCore" @batch-core="names=>batchCoreDialog?.open(names)" @add-management-folder="managementFolderDialog?.open()" @logs="name=>open('logs',{name})" @remove="name=>open('remove',{name})" @create="open('create')" @refresh="refresh(true)"/>
      <ActivityPage v-else-if="route==='activity'" :jobs="projectedJobs" :initial-loading="jobsReading && !jobsLoaded && !jobsError && !residentError" :loaded="jobsLoaded" :time-format="visiblePreferences.timeFormat" :error="jobsError" :batch-error="residentError" :cancelling-batch="cancellingBatch" :cancelling-core-batch="cancellingCoreBatch" :core-cancel-failure="coreCancelFailure" :feedback="activityFeedback" @refresh="refreshJobs(true)" @cancel-batch="cancelResidentBatch" @cancel-core-batch="cancelCoreBatch"/>
      <SettingsPage v-else :instance="settingsName" :instance-state="currentSettingsState" :instance-run="currentSettingsRow?.run?.id" :world-command-ready="worldCommandReady" :world-rule-work="worldRuleWork" :restart-status="currentSettingsRow?.autoRestartStatus" :instance-meta="currentSettingsMeta" :preferences="presentation.preferences" :preferences-revision="preferencesRevision" :preferences-loaded="preferencesLoaded" :preferences-ready="preferencesLoaded && !preferencesUnavailable" :connected="connected" :java-candidates="javaCandidates" :java-checked="javaChecked" :refresh-java="discoverJava" :retry-preferences="retryPreferences" @close="navigate('instances')" @core="openCore" @language-preview="languagePreview=$event" @dirty="settingsDirty=$event" @saving="settingsSaving=$event" @mutating="invalidateRefresh" @management-changed="invalidateRefresh(); refresh(); refreshJobs(false,true)" @preferences="next=>{invalidateRefresh();applyPreferences(next)}"/>
    </main></div>
  <footer class="statusbar" :class="{disconnected:connection==='disconnected'||connection==='browser'}" role="status"><span class="connection-dot" aria-hidden="true"></span>{{ t(connection==='connected'?'connection.connected':connection==='browser'?'connection.browser':connection==='unknown'?'connection.unknown':'connection.disconnected') }}<span v-if="preferencesUnavailable"> · {{ preferencesLoaded ? '無法重新讀取外觀設定，保留上次設定' : '外觀設定尚未讀取' }}</span></footer>
  <CoreMaintenanceDialog v-if="coreTarget" ref="coreDialog" :name="coreTarget" :cached-source="coreCheckRows[coreTarget]" :query="manuallyCheckCore" :time-format="visiblePreferences.timeFormat" @close="coreTarget=undefined" @changed="refreshAfterCoreChange" />
  <ManagementFolderDialog ref="managementFolderDialog" :connected="connected" :path-separator="visiblePreferences.pathSeparator"
    @changed="invalidateRefresh(); refresh(); refreshJobs(false,true)" />
  <BatchCoreDialog ref="batchCoreDialog" :instances="rows" :meta="presentation.instances" :source-rows="coreCheckRows" :query="manuallyCheckCoreBatch"
    :batches="coreBatches" :connected="connected" :blocked-names="new Set([...lifecycleBusy,...queuedStarts,...residentStarting])"
    :cancelling="cancellingCoreBatch" :cancel-failure="coreCancelFailure"
    @submitted="coreBatchSubmitted" @changed="refreshAfterCoreChange" @cancel="cancelCoreBatch" />
  <dialog ref="dialog" class="overlay-dialog" aria-labelledby="operation-dialog-title" :class="{compact:modal==='force'||modal==='remove'}" @cancel.prevent="close">
    <OverlayScroll class="dialog-scroll" label="對話框內容">
    <template v-if="modal==='batch'"><h2 id="operation-dialog-title">確認批次操作</h2><p>將對 {{ pendingBatch?.targets.length ?? 0 }} 台實例執行{{ pendingBatch?.action==='start'?'啟動':pendingBatch?.action==='stop'?'正常停止':'重新啟動' }}。</p><p v-if="pendingBatch?.action==='restart'" class="hint">停止逾時的實例不會自動啟動。</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="submittingBatch" @click="close">取消</button><OperationButton class="primary" label="確認操作" :busy-label="t('action.submitting')" :busy="submittingBatch" @click="confirmBatch"/></div></template>
    <template v-if="modal==='force'"><h2 id="operation-dialog-title">強制停止伺服器</h2><p>要強制停止 {{ target?.name }} 嗎？尚未存檔的世界變更可能遺失。</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="modalBusy" @click="close">取消</button><button class="danger" :disabled="modalBusy" @click="forceStop">強制停止</button></div></template>
    <template v-else-if="modal==='remove'"><h2 id="operation-dialog-title">移除管理</h2><p>選擇 {{ target?.name }} 的管理設定處理方式。兩種方式都保留世界、插件與伺服器檔案；日後可在管理資料夾重新核對。</p><label class="choice"><input v-model="removeChoice" type="radio" value="retain">移除管理，保留管理設定</label><label class="choice"><input v-model="removeChoice" type="radio" value="delete">移除管理並刪除管理設定</label><p v-if="modalMessage" role="status">{{ modalMessage }}</p><p v-if="modalError" ref="modalErrorElement" tabindex="-1" class="form-error" role="alert">{{ modalError }}</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="modalBusy" @click="close">取消</button><button class="danger" :disabled="modalBusy" @click="removeManagement">確認移除</button></div></template>
    <template v-else-if="modal==='logs'"><h2 id="operation-dialog-title">{{ target?.name }} · 啟動紀錄</h2><label v-if="logs?.sources">紀錄來源
        <select v-model="logSource" name="log-source">
          <option v-for="source in logs.sources" :key="source.source" :value="source.source">{{ source.source==='terminal'&&logs.merged?'終端合併輸出':logLabels[source.source] }}</option>
        </select>
      </label>
      <p class="hint selectable">{{ selectedLog?.path ?? logs?.path ?? '尚未取得紀錄路徑' }}</p>
      <p v-if="logs" class="hint">{{ logs.consoleReady?'已取得核心就緒紀錄':'核心就緒尚未核對' }}。插件及代理連線需另外驗證。</p><p v-if="logsError" class="form-error" role="alert">{{ logsError }}</p><p v-if="logsMessage" role="status">{{ logsMessage }}</p><InitialQueryProgress v-if="logsReading && !logs && !logsError" label="正在讀取啟動紀錄" /><OverlayScroll v-else-if="logs" class="logs-scroll" label="啟動紀錄內容"><pre>{{ selectedLog ? selectedLog.text ?? logAvailability[selectedLog.state] : logs?.text ?? '尚無可顯示的紀錄。' }}</pre></OverlayScroll><div class="dialog-actions"><button @click="openLogFolder">開啟紀錄資料夾</button><button ref="cancelButton" autofocus @click="close">關閉</button></div></template>
    <template v-else-if="modal==='create'">
      <h2 id="operation-dialog-title">{{ creationPreview ? '確認建立實例' : '建立實例' }}</h2>
      <template v-if="creationPreview">
        <dl class="create-review">
          <dt>實例路徑</dt><dd class="selectable" translate="no">{{ createPath(creationPreview.target) }}</dd>
          <dt>核心版本</dt><dd>{{ creationPreview.core.family }} {{ creationPreview.core.minecraft }}<span v-if="creationPreview.core.build"> · Build {{ creationPreview.core.build }}</span></dd>
          <dt>Port</dt><dd>{{ draft.port }}</dd>
          <template v-if="creationPreview.javaMajor"><dt>建議 Java</dt><dd>Java {{ creationPreview.javaMajor }}</dd></template>
          <template v-if="creationPreview.whitelist">
            <dt>新增納入資料夾</dt><dd class="selectable" translate="no">{{ createPath(creationPreview.whitelist) }}</dd>
          </template>
        </dl>
        <p v-if="creationPreview.whitelist" class="hint">只加入這台實例的資料夾。</p>
        <p v-if="creationPreview.initializationDefaults === false" class="hint">此版本使用核心內建預設值。</p>
        <p class="hint">建立後保持停止。</p>
        <div v-if="creationPhase" class="creation-progress">
          <div class="creation-progress-heading">
            <p id="creation-progress-label" role="status">{{ t(`creation.${creationPhase}`) }}</p>
            <span v-if="creationBytes" aria-hidden="true">{{ Math.floor(creationBytes.received / creationBytes.total * 100) }}%</span>
          </div>
          <progress aria-labelledby="creation-progress-label"
            :value.attr="creationBytes?.received" :max.attr="creationBytes?.total"></progress>
        </div>
        <p v-if="modalError" class="form-error" role="alert">{{ modalError }}</p>
        <p v-if="creationUnconfirmedWork" class="hint selectable">工作 ID <bdi class="mono" translate="no">{{ creationUnconfirmedWork }}</bdi></p>
        <div class="dialog-actions">
          <button ref="createBackButton" type="button" :disabled="modalBusy" @click="cancelCreationPreview">返回</button>
          <button type="button" class="primary" :disabled="modalBusy" @click="provision">建立</button>
        </div>
      </template>
      <form v-else @submit.prevent="previewCreation">
        <div class="form-row create-name-row">
          <div>
            <label for="create-instance-name">實例名稱
              <input id="create-instance-name" ref="createNameInput" v-model.trim="draft.name" name="instance-name"
                :readonly="modalBusy" required autocomplete="off" spellcheck="false"
                :aria-invalid="createNameChecked && !!createNameIssue || undefined"
                :aria-describedby="createNameChecked && createNameIssue ? 'create-instance-name-error' : undefined"
                @invalid.prevent="validateCreationName">
            </label>
            <p v-if="createNameChecked && createNameIssue" id="create-instance-name-error" class="form-error" role="alert">
              {{ createNameIssue }}
            </p>
          </div>
          <label>Port<input v-model.number="draft.port" name="port" type="number" min="1" max="65535" required :disabled="modalBusy"></label>
        </div>
        <label for="create-parent">上層資料夾</label>
        <div class="create-path-editor">
          <input id="create-parent" v-model.trim="draft.parent" name="create-parent" required autocomplete="off"
            spellcheck="false" :readonly="modalBusy || createEditingPicker">
          <PathPickerButton :current="draft.parent" kind="folder" :disabled="modalBusy || createEditingPicker"
            @picking="createEditingPicker=$event" @failed="pickerFailed" @selected="draft.parent=createPath($event)" />
        </div>
        <CreateCorePicker v-model:family="draft.source" v-model:version="draft.version"
          :disabled="modalBusy || createEditingPicker" :versions="creationVersions" :build="creationBuild"
          @retry-versions="refreshCreationVersions(true)" @retry-build="refreshCreationBuild(true)" />
        <template v-if="!draft.source">
          <label for="create-core">伺服器核心 JAR</label>
          <div class="create-path-editor">
            <input id="create-core" v-model.trim="draft.core" name="core" required autocomplete="off"
              spellcheck="false" :readonly="modalBusy || createEditingPicker">
            <PathPickerButton :current="draft.core" kind="jar" :disabled="modalBusy || createEditingPicker"
              @picking="createEditingPicker=$event" @failed="pickerFailed" @selected="draft.core=createPath($event)" />
          </div>
        </template>
        <p v-if="modalMessage" role="status">{{ modalMessage }}</p>
        <p v-if="modalError" class="form-error" role="alert">{{ modalError }}</p>
        <div class="dialog-actions">
          <button ref="cancelButton" type="button" autofocus :disabled="modalBusy || createEditingPicker" @click="close">取消</button>
          <button class="primary" :disabled="modalBusy || createEditingPicker || !canPreviewCreation">預覽建立</button>
        </div>
      </form>
    </template>
  </OverlayScroll>
  </dialog>
  <dialog ref="routeDialog" class="compact" aria-labelledby="route-dialog-title" @cancel.prevent="routeDialog?.close()"><h2 id="route-dialog-title">放棄變更</h2><p>目前設定有尚未儲存的變更。</p><div class="dialog-actions"><button autofocus @click="routeDialog?.close()">取消</button><button class="danger" @click="confirmRoute">放棄變更</button></div></dialog>
  <dialog ref="discardProvisionDialog" class="compact" aria-labelledby="discard-provision-title" @cancel.prevent="discardProvisionDialog?.close()"><h2 id="discard-provision-title">放棄建立草稿</h2><p>目前欄位有尚未送出的內容。</p><div class="dialog-actions"><button autofocus @click="discardProvisionDialog?.close()">取消</button><button class="danger" @click="discardProvision">放棄草稿</button></div></dialog>
  <dialog ref="failureDialog" @close="afterFailureClose" class="compact" role="alertdialog" aria-labelledby="failure-title" aria-describedby="failure-message"><h2 id="failure-title">無法完成操作</h2><p id="failure-message" class="selectable">{{ failureMessage }}</p><div class="dialog-actions"><button autofocus @click="failureDialog?.close()">關閉</button></div></dialog>
</template>


<style scoped>
.creation-progress { display: grid; gap: .5rem; margin-block: 1rem; }
.creation-progress p { margin: 0; }
.creation-progress-heading { display: flex; justify-content: space-between; gap: .5rem; font-variant-numeric: tabular-nums; }
.creation-progress progress {
  appearance: none;
  width: 100%;
  height: .625rem;
  container-type: inline-size;
  overflow: clip;
  border: 0;
  border-radius: 999px;
}
.creation-progress progress::-webkit-progress-bar { background: var(--raised); }
.creation-progress progress::-webkit-progress-value { background: var(--primary); border-radius: 999px; }
.creation-progress progress[value]::-webkit-progress-value { transition: inline-size .12s linear; }
.creation-progress progress:indeterminate::-webkit-progress-value {
  /* Style the native fill using Chrome's progress pattern. Only translate changes between frames. */
  animation: creation-progress-slide 1.25s linear infinite;
}
@keyframes creation-progress-slide {
  from, to { inline-size: 28%; margin-right: auto; }
  from { translate: -100% 0; }
  to { translate: 100cqi 0; }
}
@media (prefers-reduced-motion: reduce) {
  .creation-progress progress:indeterminate::-webkit-progress-value { animation-delay: -.625s; animation-play-state: paused; }
  .creation-progress progress[value]::-webkit-progress-value { transition: none; }
}
.create-name-row { align-items: start; }
.registry-notice { margin: 0 0 .75rem; padding: .5rem .75rem; border: 1px solid var(--border); border-radius: .375rem; flex: none; }
.registry-issues { max-block-size: 5rem; }
.registry-notice p { margin: 0; }
.registry-notice ul { margin: .25rem 0 0; padding-inline-start: 1.125rem; }
.registry-notice li { overflow-wrap: anywhere; }
.create-name-row .form-error { margin: .375rem 0 0; font-size: .8125rem; }
.create-path-editor { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .5rem; margin-block: .375rem 1rem; }
.create-path-editor input { width: 100%; }
.create-review { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: .5rem 1rem; font-size: .875rem; }
.create-review dt { color: var(--muted); }
.create-review dd { margin: 0; overflow-wrap: anywhere; }
</style>
