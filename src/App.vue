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
import RemovedInstancesDialog from './components/RemovedInstancesDialog.vue'
import PathPickerButton from './components/PathPickerButton.vue'
import CopyButton from './components/CopyButton.vue'
import { displayPath } from './path-display'
import OverlayScroll from './components/OverlayScroll.vue'
import OperationButton from './components/OperationButton.vue'
import InitialQueryProgress from './components/InitialQueryProgress.vue'
import CreateCorePicker from './components/CreateCorePicker.vue'
import { catalogRefreshInterval, useCoreCatalog, type CatalogFamily } from './core-catalog'
import {validPinOrder} from './pin-order'
import {suppressBrowserMenu} from './context-menu'
import { batchFailureReasonId, newBatchFailures, type BatchFailure } from './operation-failures'
import { provideAppLanguage, type LanguagePreference } from './i18n'
import { refreshSelectionColor, refreshSelectionTheme, resolveSelectionOpacity, resolveTheme, shouldRespondToSystemTheme } from './theme'
import { canOpenConsole, canStop, confirmCreation, consoleFallbackReason, type ConsolePlan, coreBatchJob, errorText, manage, nativeBridge, rawError, type CoreBatch, type CoreSource, type CreationProgress, type Instance, type JavaCandidate, type Job, type Logs, type Preferences, type PreferencesSnapshot, type InstanceSnapshot, type RegistryIssue, type Presentation } from './manager'
import './app.css'

type Route='instances'|'activity'|'settings'
type Mode='create'|'force'|'remove'|'logs'|'batch'|'console-window'|null
const coreDialog = ref<InstanceType<typeof CoreMaintenanceDialog>>()
const coreTarget = ref<string>()
const managementFolderDialog = ref<InstanceType<typeof ManagementFolderDialog>>()
const removedInstancesDialog = ref<InstanceType<typeof RemovedInstancesDialog>>()
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
const { t, locale }=provideAppLanguage(()=>presentation.value.preferences.language,()=>languagePreview.value)
// The tray tooltip follows the list; it still updates while the window is hidden in the tray.
const runningCount=computed(()=>rows.value.filter(row=>canStop(row)).length)
watch([runningCount,locale],([running,language])=>{
  if(nativeBridge)void invoke('tray_status',{running,language}).catch(e=>console.error('Could not update the tray tooltip',e))
},{immediate:true})
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
    }catch(e){ console.error('Background core check failed', e) }
  }))
}
async function refreshAfterCoreChange() {
  await Promise.all([refresh(), refreshJobs(false,true)])
  await checkCoreUpdates(presentation.value.instances.map(item=>item.name),false)
}
const registryIssues=ref<RegistryIssue[]>([])
const registryIssueLabels=computed<Record<RegistryIssue['reason'],string>>(()=>({
  FileInUse:t('app.registry.fileInUse'), AccessDenied:t('app.registry.accessDenied'), InvalidRecord:t('app.registry.invalidRecord'), Unconfirmed:t('app.registry.unconfirmed'), ObservationUnconfirmed:t('app.registry.observationUnconfirmed'),
}))
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
const jobs=ref<Job[]>([]), jobsError=ref(''), feedback=ref(''), activityFeedback=ref<''|'reading'|'read'>('')
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
const failureDialog=ref<HTMLDialogElement>(), failureMessage=ref(''), failureItems=ref<BatchFailure[]>([])
type OperationFailure={message:string;items:BatchFailure[];batch?:{id:string;action:string}}
const pendingOperationErrors:OperationFailure[]=[]
let failureBatch:OperationFailure['batch']
function batchFailureMessage(batch:{action:string},count:number){
  const action=({'batch-start':'app.batch.start','batch-stop':'app.batch.stop','batch-restart':'app.batch.restart'} as const)[batch.action as 'batch-start']
  return t('app.batchFailed',{action:action?t(action):batch.action,count})
}
// Operation errors appear only in this dialog, never in the status line above the list.
function showOperationError(message:string,items:BatchFailure[]=[],batch?:OperationFailure['batch']){
  if(failureMessage.value){
    // More failures from the batch on screen join the open dialog.
    if(batch && failureBatch?.id===batch.id){
      failureItems.value=[...failureItems.value,...items]
      failureMessage.value=batchFailureMessage(batch,failureItems.value.length)
      return
    }
    const queued=batch&&pendingOperationErrors.find(entry=>entry.batch?.id===batch.id)
    if(queued){queued.items=[...queued.items,...items];queued.message=batchFailureMessage(batch,queued.items.length);return}
    const duplicate=(entry:{message:string;items:BatchFailure[]})=>entry.message===message&&!entry.items.length&&!items.length
    if(!duplicate({message:failureMessage.value,items:failureItems.value})&&!pendingOperationErrors.some(duplicate))pendingOperationErrors.push({message,items,batch})
    return
  }
  failureMessage.value=message
  failureItems.value=items
  failureBatch=batch
  void nextTick(()=>{if(!failureDialog.value?.open)failureDialog.value?.showModal()})
}
// Batches sent from this window report each failed instance as soon as it fails,
// while the other instances keep starting.
const watchedBatches=new Map<string,Set<string>>()
watch(residentJobs,list=>{
  for(const job of list){
    const reported=watchedBatches.get(job.id)
    if(!reported)continue
    const failures=newBatchFailures(job,reported)
    if(job.state!=='Pending')watchedBatches.delete(job.id)
    if(failures.length)showOperationError(batchFailureMessage(job,failures.length),failures,{id:job.id,action:job.action})
  }
})
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
const logLabels = computed<Record<string,string>>(()=>({terminal:t('app.log.terminal'),stderr:t('app.log.stderr'),host:t('app.log.host'),control:t('app.log.control')}))
const logAvailability = computed<Record<string,string>>(()=>({Missing:t('app.log.missing'),Empty:t('app.log.empty'),Unavailable:t('app.log.unavailable')}))
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
  testAdministrators: 'default' as 'default' | 'grant' | 'skip',
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
  testAdministrators?: { requested: boolean; configured: number; grant: { count: number; group: string; fileName: string } | null }
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
  if(!name)return t('app.name.required')
  if(name.length>64)return t('app.name.tooLong')
  if(name.includes('.'))return t('app.name.period')
  if(!/^[a-z0-9]/.test(name))return t('app.name.start')
  if(!/^[a-z0-9-]+$/.test(name))return t('app.name.characters')
  if(/^(?:con|prn|aux|nul|(?:com|lpt)[1-9])$/.test(name)){
    return t('app.name.reserved')
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
    ...(draft.testAdministrators === 'default' ? {} : { testAdministrators: draft.testAdministrators === 'grant' }),
  }
}
const creationTestAdmins = computed(() => {
  const state = creationPreview.value?.testAdministrators
  if (!state) return undefined
  if (state.grant) return state.grant.group
    ? t('app.create.testAdminsGrantedGroup', { count: state.grant.count, group: state.grant.group, file: state.grant.fileName })
    : t('app.create.testAdminsGranted', { count: state.grant.count, file: state.grant.fileName })
  return t(state.configured ? 'app.create.testAdminsDeclined' : 'app.create.testAdminsNone')
})
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
const removeChoice=ref<'retain'|'delete'|'recycle'>('retain')
const removeConfirmName=ref('')
const removeMeta=computed(()=>presentation.value.instances.find(item=>item.name===target.value?.name))
const removeReady=computed(()=>removeChoice.value!=='recycle'||(!!target.value&&removeConfirmName.value===target.value.name))
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
    console.error('Background Java discovery failed',e)
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
    feedback.value=t('app.windowError',{error:errorText(e)})
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
function presentDiagnosticWarning(){
  if(pendingDiagnosticWarning && !failureMessage.value && !failureDialog.value?.open){pendingDiagnosticWarning=false;showOperationError(t('app.diagnosticWarning'))}
}
function afterFailureClose(){failureMessage.value='';failureItems.value=[];failureBatch=undefined;const pending=pendingOperationErrors.shift();if(pending)showOperationError(pending.message,pending.items,pending.batch);else void nextTick(presentDiagnosticWarning)}
async function checkDiagnostics(){
  if(!nativeBridge||diagnosticFailureReported)return
  try{
    const status=await invoke<{available:boolean;error?:string}>('diagnostic_status')
    if(!status.available){diagnosticFailureReported=true;pendingDiagnosticWarning=true;void nextTick(presentDiagnosticWarning)}
  }catch(error){console.error('Could not check the diagnostic log state',error)}
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
  if(nativeBridge && windowReady)void invoke('present_window',{theme}).catch(e=>console.error('Could not apply the window theme',e))
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
  if(!nativeBridge){connection.value='browser';if(manual)feedback.value=t('app.bridgeMissing');return}
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
      if(issues.length)feedback.value=t('app.refresh.readable')
      else if(preferencesUnavailable.value)feedback.value=t('app.refresh.preferencesUnavailable')
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
    listUnavailable.value=t('app.listError',{error:errorText(e)})
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
  if(manual){jobsManual=true;clearTimeout(activityFeedbackTimer);activityFeedback.value='reading'}
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
            void manage(['manager-connect'],'background').catch(e=>console.error('Could not connect to core maintenance work',e))
          }
        }
        if(residentAtRead===residentRevision && batches.status==='fulfilled'){
          if(JSON.stringify(batches.value)!==JSON.stringify(residentJobs.value))residentJobs.value=batches.value
          residentError.value=''
        }else if(residentAtRead===residentRevision && batches.status==='rejected' && (jobsManual || !jobsLoaded.value || jobsBackgroundFailures+1>=BACKGROUND_FAILURE_LIMIT))residentError.value=t('app.batchCheckError',{error:errorText(batches.reason)})
        if(legacy.status==='rejected')throw legacy.reason
        if(JSON.stringify(legacy.value)!==JSON.stringify(jobs.value))jobs.value=legacy.value
        if(maintenance.status==='rejected' && (jobsManual || !jobsLoaded.value)){
          jobsError.value=t('app.maintenanceReadError',{error:errorText(maintenance.reason)})
        }else if(maintenance.status==='fulfilled')jobsError.value=''
        if(batches.status==='fulfilled' && maintenance.status==='fulfilled'){jobsLoaded.value=true;jobsBackgroundFailures=0}
        else if(!jobsManual)jobsBackgroundFailures++
        if(jobsManual){
          activityFeedback.value=batches.status==='fulfilled' && maintenance.status==='fulfilled'?'read':''
          activityFeedbackTimer=setTimeout(()=>{if(activityFeedback.value==='read')activityFeedback.value=''},1000)
        }
      }catch(e){
        if(epoch!==jobsEpoch)continue
        if(!jobsManual && jobsLoaded.value && ++jobsBackgroundFailures<BACKGROUND_FAILURE_LIMIT)continue
        jobsError.value=t('app.activityReadError',{error:errorText(e)})
        if(jobsManual)activityFeedback.value=''
      }
    }while(jobsFollowup)
  })().finally(()=>{jobsRead=undefined;jobsManual=false;jobsReading.value=false})
  return jobsRead
}
function navigate(next:Route){
  if(route.value==='settings' && settingsSaving.value){
    feedback.value=t('app.settingsSaving')
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
  if(next==='create'){createNameChecked.value=false;cancelCreationPreview();Object.assign(draft,{name:'',parent:'instances',port:12010,core:'',source:'',version:'',testAdministrators:'default'})}
  provisionBaseline.value=JSON.stringify(draft)
  if(next==='remove'){removeChoice.value='retain';removeConfirmName.value=''}
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
async function loadLogs(name:string, run?:string, background=false){
  const epoch=background?logsEpoch:++logsEpoch
  const current=()=>epoch===logsEpoch && modal.value==='logs' && target.value?.name===name
  if(!background)logsReading.value=true
  try{
    const result=await manage<Logs>(['logs',name],background?'background':'manual')
    if(!current())return
    if(result.run && result.run===run)lastLogs={name,run,result}
    // A quiet refresh changes nothing on screen unless the log itself changed.
    if(background && JSON.stringify(result)===JSON.stringify(logs.value))return
    const view=logsScroll.value?.viewport
    const following=!view || view.scrollHeight-view.scrollTop-view.clientHeight<8
    logs.value=result
    if(background && following){
      await nextTick()
      if(view)view.scrollTop=view.scrollHeight
    }
  }catch(e){
    // A quiet refresh keeps the last log; only a changed session needs the user.
    if(current() && (!background || rawError(e).includes('LogRunChanged')))logsError.value=rawError(e).includes('LogRunChanged')
      ? t('app.logs.runChanged')
      : t('app.logs.readError',{error:errorText(e)})
  }finally{
    if(current() && !background)logsReading.value=false
  }
}
// While its server runs, the open log follows new output every two seconds.
const logsScroll=ref<{viewport?:HTMLDivElement}>()
const followedLogRun=computed(()=>{
  if(modal.value!=='logs'||!target.value)return undefined
  const row=rows.value.find(item=>item.name===target.value?.name)
  return row?.run && ['Starting','Listening'].includes(row.state)?row.run.id:undefined
})
let logsFollowTimer:ReturnType<typeof setInterval>|undefined
watch(followedLogRun,run=>{
  clearInterval(logsFollowTimer)
  logsFollowTimer=run?setInterval(()=>{
    const name=target.value?.name
    if(name && !logsReading.value && !logsError.value)void loadLogs(name,run,true)
  },2000):undefined
})
onUnmounted(()=>clearInterval(logsFollowTimer))
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
        failures.push(t('app.console.failure',{name:row.name,error:errorText(error)}))
      }
    }
    if(failures.length)showOperationError(t('app.console.failed',{failures:failures.join(t('app.console.separator'))}))
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
    if(current())logsMessage.value=t('app.logs.folderOpened')
  }catch(e){
    if(current())logsError.value=t('app.logs.folderError',{error:errorText(e)})
  }
}
function markBusy(name:string,value:boolean){
  const next=new Set(busyNames.value)
  if(value)next.add(name)
  else next.delete(name)
  busyNames.value=next
}
// A start that would need a separate console window asks first, every time.
const consoleWindowStart=ref<{row:Instance;reason:string}>()
async function confirmConsoleWindow(row:Instance){
  let plan:ConsolePlan
  try{plan=await manage<ConsolePlan>(['console-plan',row.name])}
  // The Host checks again at start and never opens a window without this confirmation.
  catch(e){console.warn('Console plan unavailable',e);return false}
  if(!plan.needsConfirmation)return false
  consoleWindowStart.value={row,reason:consoleFallbackReason(plan)??''}
  open('console-window',{name:row.name})
  return true
}
function startFromConsoleDialog(consoleWindow:boolean){
  const pending=consoleWindowStart.value
  if(!pending)return
  dialog.value?.close()
  modal.value=null
  consoleWindowStart.value=undefined
  void operateOne('start',pending.row,{consoleWindow,asked:true})
}
async function operateOne(action:'start'|'stop'|'restart',row:Instance,options:{consoleWindow?:boolean;asked?:boolean}={}){
  if(!connected.value||lifecycleBusy.value.has(row.name)||queuedStarts.value.has(row.name)||(action!=='stop'&&residentStarting.value.has(row.name)))return
  if(action!=='stop' && row.outsideManagement)return
  if(action==='start'&&!options.asked&&await confirmConsoleWindow(row))return
  const consoleWindow=action==='start'&&options.consoleWindow===true
  const needsRun=action==='stop'||(action==='restart'&&row.state!=='Stopped')
  if(needsRun&&!row.run)return
  invalidateRefresh()
  const previousRun=row.run?.id??null
  const stopToken=row.stopToken??null
  const key=JSON.stringify([action,row.name,previousRun,stopToken,consoleWindow])
  const id=requestId(key)
  const args=[action==='start'?'start-if-current':action,row.name,id]
  if(action==='start')args.push(JSON.stringify(consoleWindow?{previousRun,stopToken,consoleWindow}:{previousRun,stopToken}))
  else if(needsRun)args.push(row.run!.id)
  if(action==='stop')stopping.value[row.name]=row.run!.id
  else manualStarts.value.add(row.name)
  markBusy(row.name,true)
  markActiveRequest(id,true)
  const generation=forceGeneration
  feedback.value=action==='stop'&&!row.consoleReady?t('app.op.waitingStop'):t(({start:'app.op.starting',stop:'app.op.stopping',restart:'app.op.restarting'} as const)[action])
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
      if(generation===forceGeneration)showOperationError(t('app.stopUnconfirmed',{name:row.name}))
    }else if(generation===forceGeneration)showOperationError(t('app.opFailed',{name:row.name,error:errorText(e)}))
    if(generation===forceGeneration)feedback.value=''
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
    if(!watchedBatches.has(work.id))watchedBatches.set(work.id,new Set())
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
  }catch(e){showOperationError(t('app.cancelBatchError',{error:errorText(e)}))}
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
  catch(e){showOperationError(t('app.batchAcceptError',{error:errorText(e)}))}
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
  feedback.value=t('app.forceStopping')
  try{
    await manage<{forced?:boolean}>(['force-stop',item.name,id,item.run])
    feedback.value=''
  }catch(e){feedback.value='';showOperationError(t('app.forceStopError',{error:errorText(e)}))}
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
  modalMessage.value=t('app.create.checkingPath')
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
    return t('app.create.invalidName')
  }
  if (/CoreNoStableBuild|CreationNoStableBuild/.test(reason)) {
    return t('app.create.noStable')
  }
  if (/CoreSourceTimeout|CoreSourceHttpError|CoreSourceTransportError|CoreDownloadUnverified/.test(reason)) {
    return t('app.create.sourceIncomplete')
  }
  if (/CoreArtifactDigestMismatch|CoreArtifactSizeMismatch/.test(reason)) {
    return t('app.create.digestMismatch')
  }
  if (/CoreDownloadUnsupportedLayout|MojangMetadataUnverified/.test(reason)) {
    return t('app.create.mojangUnverified')
  }
  if (reason.includes('CoreDownloadCacheInvalid')) {
    return t('app.create.cacheInvalid')
  }
  if (reason.includes('MissingMojangArtifact')) {
    return t('app.create.missingMojangJar')
  }
  if (reason.includes('GlobalJavaIncompatible')) {
    return t('app.create.javaIncompatible')
  }
  if (/AutoJavaUnavailable|GlobalJavaUnavailable/.test(reason)) {
    return t('app.create.javaUnavailable')
  }
  if (reason.includes('TestPermissionArtifactMissing')) {
    return t('app.create.testAdminsMissing')
  }
  if (reason.includes('TestPermissionArtifactMismatch')) {
    return t('app.create.testAdminsMismatch')
  }
  if (reason.includes('UnsupportedTestPermissionPlatform')) {
    return t('app.create.testAdminsUnsupported')
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
    feedback.value=t('app.create.done',{name})
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
  }catch(e){showOperationError(t('app.pin.saveError',{error:errorText(e)}));void refresh()}
  finally{pinBusy.value=false}
}
function savePinOrder(pins:string[],expected:string[]){
  if(!validPinOrder(visiblePreferences.value.pins,pins,expected)){showOperationError(t('app.pin.changed'));return}
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
  if(!item||!removeReady.value)return
  const recycle=removeChoice.value==='recycle'
  invalidateRefresh()
  modalBusy.value=true
  modalError.value=''
  modalMessage.value=t(recycle?'app.remove.recycling':'app.remove.removing')
  try{
    if(recycle)await manage(['delete-server',item.name,removeConfirmName.value])
    else await manage(['remove-management',item.name,removeChoice.value])
    feedback.value=t(recycle?'app.remove.recycled':'app.remove.done',{name:item.name})
    dialog.value?.close()
    modal.value=null
    await refresh()
  }catch(e){
    modalMessage.value=''
    await showModalError(t(recycle?'app.remove.recycleError':'app.remove.error',{error:errorText(e)}))
    if(recycle)void refresh()
  }
  finally{modalBusy.value=false}
}
async function openServerFolder(name:string){
  try{await manage(['open-server-folder',name])}
  catch(e){feedback.value=t('instances.openServerFolderError',{error:errorText(e)})}
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
  <a class="skip" href="#view">{{ t('skip.main') }}</a>
  <div class="app"><aside class="rail"><div class="brand"><img :src="brandLogo" width="16" height="16" alt=""><span>InstanceHarbor</span></div><nav :aria-label="t('nav.main')"><button :aria-current="route==='instances'?'page':undefined" @click="navigate('instances')">{{ t('nav.instances') }}</button><button :aria-current="route==='activity'?'page':undefined" @click="navigate('activity')">{{ t('nav.activity') }}</button><button :aria-current="route==='settings'&&!settingsName?'page':undefined" @click="navigate('settings')">{{ t('nav.settings') }}</button></nav></aside>
    <main id="view" :class="{'route-settings':route==='settings'}"><div v-if="!nativeBridge" class="notice error" role="alert">{{ t('app.bridgeNotice') }}</div>
      <aside v-if="route==='instances' && (registryIssues.length || listUnavailable)" class="registry-notice" role="status">
        <p v-if="listUnavailable">{{ listUnavailable }}</p>
        <template v-else>
          <p>{{ t('app.registry.intro') }}</p>
          <OverlayScroll class="registry-issues" :label="t('app.registry.label')"><ul><li v-for="issue in registryIssues" :key="issue.file">{{ issue.file }} · {{ registryIssueLabels[issue.reason] }}</li></ul></OverlayScroll>
        </template>
      </aside>
      <InstancesPage v-if="route==='instances'" :rows="rows" :initial-loading="listReading && !listLoaded && connection==='unknown'" :loaded="listLoaded" :meta="presentation.instances" :core-check-rows="coreCheckRows" :preferences="visiblePreferences" :pin-busy="pinBusy || !preferencesLoaded || preferencesUnavailable" :busy-names="lifecycleBusy" :queued-names="queuedStarts" :starting-names="residentStarting" :stopping="stopControls" :force-busy="forceBusy" :console-busy="openingConsoles" :console-available="presentation.capabilities?.nativeConsole===true" :connected="connected" :jobs="projectedJobs" :feedback="feedback" :refresh-pending="manualRefreshPending" :refresh-done="manualRefreshDone" @action="requestAction" @batch-action="(action,targets)=>requestAction(action,targets,true)" @console="openConsoles" @force="row=>open('force',{name:row.name,run:row.run?.id})" @pin="pin" @pin-move="movePin" @pin-order="savePinOrder" @settings="name=>{settingsName=name;route='settings'}" @core="openCore" @batch-core="names=>batchCoreDialog?.open(names)" @add-management-folder="managementFolderDialog?.open()" @logs="name=>open('logs',{name})" @remove="name=>open('remove',{name})" @open-folder="openServerFolder" @removed="removedInstancesDialog?.open()" @create="open('create')" @refresh="refresh(true)"/>
      <ActivityPage v-else-if="route==='activity'" :jobs="projectedJobs" :initial-loading="jobsReading && !jobsLoaded && !jobsError && !residentError" :loaded="jobsLoaded" :time-format="visiblePreferences.timeFormat" :error="jobsError" :batch-error="residentError" :cancelling-batch="cancellingBatch" :cancelling-core-batch="cancellingCoreBatch" :core-cancel-failure="coreCancelFailure" :feedback="activityFeedback" @refresh="refreshJobs(true)" @cancel-batch="cancelResidentBatch" @cancel-core-batch="cancelCoreBatch"/>
      <SettingsPage v-else :instance="settingsName" :instance-state="currentSettingsState" :instance-run="currentSettingsRow?.run?.id" :world-command-ready="worldCommandReady" :world-rule-work="worldRuleWork" :restart-status="currentSettingsRow?.autoRestartStatus" :instance-meta="currentSettingsMeta" :preferences="presentation.preferences" :preferences-revision="preferencesRevision" :preferences-loaded="preferencesLoaded" :preferences-ready="preferencesLoaded && !preferencesUnavailable" :connected="connected" :java-candidates="javaCandidates" :java-checked="javaChecked" :refresh-java="discoverJava" :retry-preferences="retryPreferences" @close="navigate('instances')" @core="openCore" @language-preview="languagePreview=$event" @dirty="settingsDirty=$event" @saving="settingsSaving=$event" @mutating="invalidateRefresh" @management-changed="invalidateRefresh(); refresh(); refreshJobs(false,true)" @preferences="next=>{invalidateRefresh();applyPreferences(next)}"/>
    </main></div>
  <footer class="statusbar" :class="{disconnected:connection==='disconnected'||connection==='browser'}" role="status"><span class="connection-dot" aria-hidden="true"></span>{{ t(connection==='connected'?'connection.connected':connection==='browser'?'connection.browser':connection==='unknown'?'connection.unknown':'connection.disconnected') }}<span v-if="preferencesUnavailable"> · {{ t(preferencesLoaded ? 'connection.preferencesStale' : 'connection.preferencesMissing') }}</span></footer>
  <CoreMaintenanceDialog v-if="coreTarget" ref="coreDialog" :name="coreTarget" :cached-source="coreCheckRows[coreTarget]" :query="manuallyCheckCore" :time-format="visiblePreferences.timeFormat" @close="coreTarget=undefined" @changed="refreshAfterCoreChange" />
  <RemovedInstancesDialog ref="removedInstancesDialog" :connected="connected" :java-candidates="javaCandidates" :java-checked="javaChecked" :refresh-java="discoverJava" :path-separator="visiblePreferences.pathSeparator"
    @changed="invalidateRefresh(); refresh()" />
  <ManagementFolderDialog ref="managementFolderDialog" :connected="connected" :path-separator="visiblePreferences.pathSeparator"
    @changed="invalidateRefresh(); refresh(); refreshJobs(false,true)" />
  <BatchCoreDialog ref="batchCoreDialog" :instances="rows" :meta="presentation.instances" :source-rows="coreCheckRows" :query="manuallyCheckCoreBatch"
    :batches="coreBatches" :connected="connected" :blocked-names="new Set([...lifecycleBusy,...queuedStarts,...residentStarting])"
    :cancelling="cancellingCoreBatch" :cancel-failure="coreCancelFailure"
    @submitted="coreBatchSubmitted" @changed="refreshAfterCoreChange" @cancel="cancelCoreBatch" />
  <dialog ref="dialog" class="overlay-dialog" aria-labelledby="operation-dialog-title" :class="{compact:modal==='force'||modal==='remove'||modal==='console-window'}" @cancel.prevent="close">
    <OverlayScroll class="dialog-scroll" :label="t('app.dialog.content')">
    <template v-if="modal==='batch'"><h2 id="operation-dialog-title">{{ t('app.batch.title') }}</h2><p>{{ t('app.batch.body',{count:pendingBatch?.targets.length ?? 0,action:t(pendingBatch?.action==='start'?'app.batch.start':pendingBatch?.action==='stop'?'app.batch.stop':'app.batch.restart')}) }}</p><p v-if="pendingBatch?.action==='restart'" class="hint">{{ t('app.batch.restartHint') }}</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="submittingBatch" @click="close">{{ t('action.cancel') }}</button><OperationButton class="primary" :label="t('app.batch.confirm')" :busy-label="t('action.submitting')" :busy="submittingBatch" @click="confirmBatch"/></div></template>
    <template v-if="modal==='console-window'"><h2 id="operation-dialog-title">{{ t('console.confirm.title') }}</h2><p>{{ t('console.confirm.body',{name:target?.name ?? '',reason:consoleWindowStart?.reason ?? ''}) }}</p><p class="notice">{{ t('console.confirm.warning') }}</p><div class="dialog-actions"><button ref="cancelButton" autofocus @click="close">{{ t('action.cancel') }}</button><button @click="startFromConsoleDialog(false)">{{ t('console.confirm.plain') }}</button><button class="danger" @click="startFromConsoleDialog(true)">{{ t('console.confirm.window') }}</button></div></template>
    <template v-else-if="modal==='force'"><h2 id="operation-dialog-title">{{ t('app.force.title') }}</h2><p>{{ t('app.force.body',{name:target?.name ?? ''}) }}</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="modalBusy" @click="close">{{ t('action.cancel') }}</button><button class="danger" :disabled="modalBusy" @click="forceStop">{{ t('app.force.confirm') }}</button></div></template>
    <template v-else-if="modal==='remove'"><h2 id="operation-dialog-title">{{ t('app.remove.title') }}</h2><p>{{ t('app.remove.intro',{name:target?.name ?? ''}) }}</p>
      <fieldset class="remove-choices" :disabled="modalBusy">
        <label class="remove-choice"><input v-model="removeChoice" type="radio" name="remove-choice" value="retain" aria-describedby="remove-retain-hint"><span><strong>{{ t('app.remove.retain') }}</strong><span id="remove-retain-hint" class="hint">{{ t('app.remove.retainHint') }}</span></span></label>
        <label class="remove-choice"><input v-model="removeChoice" type="radio" name="remove-choice" value="delete" aria-describedby="remove-delete-hint"><span><strong>{{ t('app.remove.delete') }}</strong><span id="remove-delete-hint" class="hint">{{ t(removeMeta?.managementDiscovered?'app.remove.deleteDiscoveredHint':'app.remove.deleteHint') }}</span></span></label>
        <label class="remove-choice"><input v-model="removeChoice" type="radio" name="remove-choice" value="recycle" aria-describedby="remove-recycle-hint"><span><strong class="remove-danger">{{ t('app.remove.recycle') }}</strong><span id="remove-recycle-hint" class="hint">{{ t(removeMeta?.managementDiscovered?'app.remove.recycleDiscoveredHint':'app.remove.recycleHint') }}</span></span></label>
      </fieldset>
      <div class="remove-confirm" :class="{inactive:removeChoice!=='recycle'}">
        <p class="hint selectable" translate="no">{{ t('app.remove.folder',{path:removeMeta?.path?displayPath(removeMeta.path,visiblePreferences.pathSeparator):'--'}) }}</p>
        <p class="hint">{{ t('app.remove.nukeWarning') }}</p>
        <div class="remove-confirm-label"><span id="remove-confirm-label" class="selectable">{{ t('app.remove.typeName',{name:target?.name ?? ''}) }}</span><CopyButton :text="target?.name ?? ''" :label="t('app.remove.copyName')" :disabled="modalBusy||removeChoice!=='recycle'" /></div>
        <input v-model="removeConfirmName" type="text" name="remove-confirm-name" aria-labelledby="remove-confirm-label" autocomplete="off" spellcheck="false" :disabled="modalBusy||removeChoice!=='recycle'" @keydown.enter.prevent="removeManagement">
      </div>
      <p v-if="modalMessage" role="status">{{ modalMessage }}</p><p v-if="modalError" ref="modalErrorElement" tabindex="-1" class="form-error" role="alert">{{ modalError }}</p><div class="dialog-actions"><button ref="cancelButton" autofocus :disabled="modalBusy" @click="close">{{ t('action.cancel') }}</button><button class="danger" :disabled="modalBusy||!removeReady" @click="removeManagement">{{ t(removeChoice==='recycle'?'app.remove.confirmRecycle':'app.remove.confirm') }}</button></div></template>
    <template v-else-if="modal==='logs'"><h2 id="operation-dialog-title">{{ t('app.logs.title',{name:target?.name ?? ''}) }}</h2><label v-if="logs?.sources">{{ t('app.logs.source') }}
        <select v-model="logSource" name="log-source">
          <option v-for="source in logs.sources" :key="source.source" :value="source.source">{{ source.source==='terminal'&&logs.merged?t('app.logs.merged'):logLabels[source.source] }}</option>
        </select>
      </label>
      <p class="hint selectable">{{ selectedLog?.path ?? logs?.path ?? t('app.logs.noPath') }}</p>
      <p v-if="logs" class="hint">{{ t(logs.consoleReady?'app.logs.readyFound':'app.logs.readyUnverified') }}</p><p v-if="logsError" class="form-error" role="alert">{{ logsError }}</p><p v-if="logsMessage" role="status">{{ logsMessage }}</p><InitialQueryProgress v-if="logsReading && !logs && !logsError" :label="t('app.logs.reading')" /><OverlayScroll v-else-if="logs" ref="logsScroll" class="logs-scroll" :label="t('app.logs.content')"><pre>{{ selectedLog ? selectedLog.text ?? logAvailability[selectedLog.state] : logs?.text ?? t('app.logs.none') }}</pre></OverlayScroll><div class="dialog-actions"><button @click="openLogFolder">{{ t('app.logs.openFolder') }}</button><button ref="cancelButton" autofocus @click="close">{{ t('action.close') }}</button></div></template>
    <template v-else-if="modal==='create'">
      <h2 id="operation-dialog-title">{{ t(creationPreview ? 'app.create.confirmTitle' : 'app.create.title') }}</h2>
      <template v-if="creationPreview">
        <dl class="create-review">
          <dt>{{ t('app.create.path') }}</dt><dd class="selectable" translate="no">{{ createPath(creationPreview.target) }}</dd>
          <dt>{{ t('app.create.coreVersion') }}</dt><dd>{{ creationPreview.core.family }} {{ creationPreview.core.minecraft }}<span v-if="creationPreview.core.build"> · Build {{ creationPreview.core.build }}</span></dd>
          <dt>Port</dt><dd>{{ draft.port }}</dd>
          <template v-if="creationPreview.javaMajor"><dt>{{ t('app.create.suggestedJava') }}</dt><dd>Java {{ creationPreview.javaMajor }}</dd></template>
          <template v-if="creationTestAdmins"><dt>{{ t('app.create.testAdmins') }}</dt><dd>{{ creationTestAdmins }}</dd></template>
          <template v-if="creationPreview.whitelist">
            <dt>{{ t('app.create.allowedFolder') }}</dt><dd class="selectable" translate="no">{{ createPath(creationPreview.whitelist) }}</dd>
          </template>
        </dl>
        <p v-if="creationPreview.whitelist" class="hint">{{ t('app.create.onlyFolder') }}</p>
        <p v-if="creationPreview.initializationDefaults === false" class="hint">{{ t('app.create.builtinDefaults') }}</p>
        
        <div v-if="creationPhase" class="creation-progress">
          <div class="creation-progress-heading">
            <p id="creation-progress-label" role="status">{{ t(`creation.${creationPhase}`) }}</p>
            <span v-if="creationBytes" aria-hidden="true">{{ Math.floor(creationBytes.received / creationBytes.total * 100) }}%</span>
          </div>
          <progress aria-labelledby="creation-progress-label"
            :value.attr="creationBytes?.received" :max.attr="creationBytes?.total"></progress>
        </div>
        <p v-if="modalError" class="form-error" role="alert">{{ modalError }}</p>
        <p v-if="creationUnconfirmedWork" class="hint selectable">{{ t('app.create.workId') }} <bdi class="mono" translate="no">{{ creationUnconfirmedWork }}</bdi></p>
        <div class="dialog-actions">
          <button ref="createBackButton" type="button" :disabled="modalBusy" @click="cancelCreationPreview">{{ t('app.create.back') }}</button>
          <button type="button" class="primary" :disabled="modalBusy" @click="provision">{{ t('app.create.submit') }}</button>
        </div>
      </template>
      <form v-else @submit.prevent="previewCreation">
        <div class="form-row create-name-row">
          <div>
            <label for="create-instance-name">{{ t('app.create.name') }}
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
        <label for="create-parent">{{ t('app.create.parent') }}</label>
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
          <label for="create-core">{{ t('app.create.coreJar') }}</label>
          <div class="create-path-editor">
            <input id="create-core" v-model.trim="draft.core" name="core" required autocomplete="off"
              spellcheck="false" :readonly="modalBusy || createEditingPicker">
            <PathPickerButton :current="draft.core" kind="jar" :disabled="modalBusy || createEditingPicker"
              @picking="createEditingPicker=$event" @failed="pickerFailed" @selected="draft.core=createPath($event)" />
          </div>
        </template>
        <label class="create-test-admins">{{ t('app.create.testAdmins') }}
          <select v-model="draft.testAdministrators" name="test-administrators" :disabled="modalBusy || createEditingPicker">
            <option value="default">{{ t('app.create.testAdminsDefault') }}</option>
            <option value="grant">{{ t('app.create.testAdminsGrant') }}</option>
            <option value="skip">{{ t('app.create.testAdminsSkip') }}</option>
          </select>
        </label>
        <p v-if="modalMessage" role="status">{{ modalMessage }}</p>
        <p v-if="modalError" class="form-error" role="alert">{{ modalError }}</p>
        <div class="dialog-actions">
          <button ref="cancelButton" type="button" autofocus :disabled="modalBusy || createEditingPicker" @click="close">{{ t('action.cancel') }}</button>
          <button class="primary" :disabled="modalBusy || createEditingPicker || !canPreviewCreation">{{ t('app.create.preview') }}</button>
        </div>
      </form>
    </template>
  </OverlayScroll>
  </dialog>
  <dialog ref="routeDialog" class="compact" aria-labelledby="route-dialog-title" @cancel.prevent="routeDialog?.close()"><h2 id="route-dialog-title">{{ t('action.discard') }}</h2><p>{{ t('app.route.body') }}</p><div class="dialog-actions"><button autofocus @click="routeDialog?.close()">{{ t('action.cancel') }}</button><button class="danger" @click="confirmRoute">{{ t('action.discard') }}</button></div></dialog>
  <dialog ref="discardProvisionDialog" class="compact" aria-labelledby="discard-provision-title" @cancel.prevent="discardProvisionDialog?.close()"><h2 id="discard-provision-title">{{ t('app.draft.title') }}</h2><p>{{ t('app.draft.body') }}</p><div class="dialog-actions"><button autofocus @click="discardProvisionDialog?.close()">{{ t('action.cancel') }}</button><button class="danger" @click="discardProvision">{{ t('app.draft.confirm') }}</button></div></dialog>
  <dialog ref="failureDialog" @close="afterFailureClose" class="compact" role="alertdialog" aria-labelledby="failure-title" aria-describedby="failure-message"><h2 id="failure-title">{{ t('app.failure.title') }}</h2><p id="failure-message" class="selectable">{{ failureMessage }}</p><OverlayScroll v-if="failureItems.length" class="failure-items" :label="t('app.failure.items')"><ul class="selectable"><li v-for="item in failureItems" :key="item.name"><bdi translate="no">{{ item.name }}</bdi> · {{ t(batchFailureReasonId(item.reason,item.phase)) }}</li></ul></OverlayScroll><div class="dialog-actions"><button autofocus @click="failureDialog?.close()">{{ t('action.close') }}</button></div></dialog>
</template>


<style scoped>
.creation-progress { display: grid; gap: .5rem; margin-block: 1rem; }
.create-test-admins { margin-block-start: 1rem; }
.create-test-admins select { justify-self: start; }
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
