<script setup lang="ts">
import { computed, nextTick, onBeforeUpdate, onMounted, onUnmounted, onUpdated, ref, watch } from 'vue'
import OverlayScroll from './components/OverlayScroll.vue'
import OperationButton from './components/OperationButton.vue'
import InitialQueryProgress from './components/InitialQueryProgress.vue'
import ActionMenu from './components/ActionMenu.vue'
import PinIcon from './components/PinIcon.vue'
import ContextMenu from './components/ContextMenu.vue'
import InstanceDiagnosticsPanel from './components/InstanceDiagnosticsPanel.vue'
import {usePinnedDrag} from './usePinnedDrag'
import {keepEditingContext} from './context-menu'
import { displayPath } from './path-display'
import { presentedState } from './instance-state'
import { coreReasonId } from './core-display'
import { useAppLanguage, type MessageId } from './i18n'
import { actionName, batchItemState, canOpenConsole, canRestart, canStop, consoleSummary, stateName, type CoreSource, type Instance, type InstanceMeta, type Job, type Preferences } from './manager'

const props = defineProps<{coreCheckRows?:Record<string,CoreSource>;
  rows: Instance[]; initialLoading?: boolean; loaded?: boolean; meta: InstanceMeta[]; preferences: Preferences; busyNames: Set<string>; feedback:string; refreshPending?: boolean; refreshDone?: boolean;
  queuedNames?: Set<string>; startingNames?: Set<string>; pinBusy?: boolean; stopping: Record<string, string>; forceBusy: boolean; consoleBusy: boolean; consoleAvailable?: boolean; connected: boolean; jobs: Job[]
}>()
const emit = defineEmits<{
  (e:'action', action:'start'|'stop'|'restart', rows:Instance[]):void
  (e:'batch-action', action:'start'|'stop'|'restart', rows:Instance[]):void
  (e:'force', row:Instance):void
  (e:'console', rows:Instance[]):void
  (e:'pin', name:string):void
  (e:'pin-move', name:string, direction:number):void
  (e:'pin-order', pins:string[], expected:string[]):void
  (e:'settings', name:string):void
  (e:'core', name:string):void
  (e:'batch-core', names?:string[]):void
  (e:'logs', name:string):void
  (e:'remove', name:string):void
  (e:'open-folder', name:string):void
  (e:'removed'):void
  (e:'create'):void
  (e:'add-management-folder'):void
  (e:'refresh'):void
}>()
const { t } = useAppLanguage()
const query = ref('')
const filter = ref('all')
const selected = ref<string[]>([])
const anchor = ref<string | null>(null)
const sort = ref<'name'|'core'|'port'|'state'|'recent'|null>('name')
const direction = ref(1)
const instanceTable = ref<HTMLTableElement>()
let focusedPin: HTMLElement | null = null
onBeforeUpdate(() => {
  const active = document.activeElement
  focusedPin = active instanceof HTMLElement && (active.classList.contains('pin-toggle') || active.matches('tr[data-instance]')) && instanceTable.value?.contains(active) ? active : null
})
onUpdated(() => {
  // Moving a keyed row can drop browser focus even though its button survives.
  if(focusedPin?.isConnected && document.activeElement===document.body)focusedPin.focus({preventScroll:true})
  focusedPin=null
})
const metaByName = computed(() => new Map(props.meta.map(item => [item.name, item])))
const rowOrder = computed(() => new Map(props.rows.map((row,index) => [row.name,index])))
const recentByName = computed(() => {
  const result = new Map<string,Job>()
  for (const job of props.jobs) {
    if (job.items) {
      for (const item of job.items) {
        if (!result.has(item.name)) result.set(item.name, {...job, instance:item.name, state:batchItemState(item.phase)})
      }
    } else if (!result.has(job.instance)) result.set(job.instance, job)
  }
  return result
})
const visible = computed(() => props.rows.filter(row => {
  const text = (row.name + ' ' + (metaByName.value.get(row.name)?.path ?? '')).toLocaleLowerCase()
  return text.includes(query.value.toLocaleLowerCase()) && (filter.value === 'all' ||
    // A server that is stopping is no longer listed as listening.
    (filter.value === 'running' && stateClass(row) === 'Listening') ||
    (filter.value === 'stopped' && row.state === 'Stopped') ||
    (filter.value === 'attention' && row.state !== 'Stopped' && stateClass(row) !== 'Listening'))
}).sort((a,b) => {
  const ap = props.preferences.pins.indexOf(a.name), bp = props.preferences.pins.indexOf(b.name)
  if (ap !== -1 || bp !== -1) return ap === -1 ? 1 : bp === -1 ? -1 : ap-bp
  if(sort.value===null)return (rowOrder.value.get(a.name)??0)-(rowOrder.value.get(b.name)??0)
  const value = (row:Instance) => sort.value === 'core' ? coreLabel(row.name) : sort.value === 'port' ? row.port : sort.value === 'state' ? displayState(row) : sort.value === 'recent' ? recentByName.value.get(row.name)?.createdAt ?? 0 : row.name
  const av = value(a), bv = value(b)
  return (typeof av === 'number' && typeof bv === 'number' ? av-bv : String(av).localeCompare(String(bv), 'zh-Hant',{numeric:true})) * direction.value
}))
const canDragPins=computed(()=>props.connected && !props.pinBusy && props.preferences.pins.every(name=>visible.value.some(row=>row.name===name)))
const pendingPins=ref<string[]>()
function commitPinOrder(pins:string[],expected:string[]){
 pendingPins.value=pins
 emit('pin-order',pins,expected)
 // Rejected requests never enter busy; accepted writes retain the preview until settled.
 void nextTick(()=>{if(!props.pinBusy)pendingPins.value=undefined})
}
watch(()=>props.pinBusy,busy=>{if(!busy)pendingPins.value=undefined})
const pinDrag=usePinnedDrag({pins:()=>props.preferences.pins,visible:()=>visible.value.map(row=>row.name),enabled:()=>canDragPins.value,commit:commitPinOrder})
const displayed=computed(()=>{
 const pins=pinDrag.dragging.value?pinDrag.preview.value:pendingPins.value
 if(!pins?.length)return visible.value
 const lookup=new Map(visible.value.map(row=>[row.name,row]))
 return [...pins.map(name=>lookup.get(name)).filter((row):row is Instance=>!!row),...visible.value.filter(row=>!pins.includes(row.name))]
})
const menu=ref<{name:string;x:number;y:number;trigger:HTMLElement}>()
const menuItems=computed(()=>{
 const name=menu.value?.name??'',row=props.rows.find(row=>row.name===name)
 const index=props.preferences.pins.indexOf(name),blocked=!!props.pinBusy||!props.connected
 const pins=index<0?[{id:'pin',label:t('instances.pin'),disabled:blocked}]:[{id:'up',label:t('instances.pinUp'),disabled:blocked||index===0},{id:'down',label:t('instances.pinDown'),disabled:blocked||index===props.preferences.pins.length-1},{id:'pin',label:t('instances.unpin'),disabled:blocked}]
 return [...pins,{id:'remove',label:t('instances.removeManagement'),danger:true,disabled:!removable(row)}]
})
function removable(row?:Instance){return !!row&&props.connected&&row.state==='Stopped'&&!props.busyNames.has(row.name)&&!props.queuedNames?.has(row.name)}
function showMenu(event:MouseEvent,name:string){
 if(keepEditingContext(event.target,event))return
 event.preventDefault();event.stopPropagation();pinDrag.cancel()
 const trigger=event.currentTarget as HTMLElement
 menu.value={name,x:event.clientX,y:event.clientY,trigger}
}
function showKeyboardMenu(event:KeyboardEvent,name:string){
 if(event.key!=='ContextMenu' && !(event.key==='F10'&&event.shiftKey))return false
 event.preventDefault();event.stopPropagation();const trigger=event.currentTarget as HTMLElement,rect=trigger.getBoundingClientRect()
 menu.value={name,x:rect.left,y:rect.bottom,trigger};return true
}
function closeMenu(restore:boolean){const trigger=menu.value?.trigger;menu.value=undefined;if(restore&&trigger?.isConnected)trigger.focus({preventScroll:true})}
function menuAction(id:string){
 const name=menu.value?.name;if(!name)return
 const allowed=menuItems.value.some(item=>item.id===id&&!item.disabled)
 closeMenu(true);if(!allowed)return
 if(id==='remove')emit('remove',name)
 else if(id==='pin')emit('pin',name)
 else emit('pin-move',name,id==='up'?-1:1)
}
watch(()=>visible.value.map(row=>row.name).join('\u0000'),()=>{if(menu.value&&!visible.value.some(row=>row.name===menu.value!.name))closeMenu(false)})
const chosen = computed(() => props.rows.filter(row => selected.value.includes(row.name)))
const consoleTargets = computed(() => chosen.value.filter(canOpenConsole))
// The row stays in place; a stopped instance or a run without a record shows a dash.
function consoleText(row: Instance) { return row.console && canStop(row) ? consoleSummary(row.console) : '--' }
const consoleHint = computed(() => t('instances.consoleHint'))
const single = computed(() => chosen.value.length === 1 ? chosen.value[0] : undefined)
const currentMeta = computed(() => single.value && metaByName.value.get(single.value.name))
const diagnosticProxies = computed(() => props.meta.filter(item => item.core?.family?.toLowerCase() === 'velocity').map(item => item.name))
function isQueued(row:Instance){return row.state==='Stopped' && !!props.queuedNames?.has(row.name)}
function starting(row:Instance){return row.state==='Stopped' && !!props.startingNames?.has(row.name)}
function stateClass(row:Instance){return presentedState(row,{queued:!!props.queuedNames?.has(row.name),starting:!!props.startingNames?.has(row.name),stopping:!!row.run && props.stopping[row.name]===row.run.id})}
function displayState(row:Instance){const state=stateClass(row);return state==='Queued'?t('activity.state.Queued'):stateName(state)}
const emergency = computed(() => !!single.value && canStop(single.value) &&
  !!(single.value.stopPhase || props.stopping[single.value.name] === single.value.run?.id))
const recent = computed(() => single.value && recentByName.value.get(single.value.name))
const refreshBusy=computed(()=>!!props.refreshPending)
const refreshLabel=computed(()=>props.refreshDone?t('instances.refreshDone'):t('action.refresh'))
function coreLabel(name:string){const core=metaByName.value.get(name)?.core;return core?.state==='known' ? core.label ?? `${core.family} ${core.minecraft}${core.build ? ` build ${core.build}` : ''}` : t('instances.coreUnchecked')}
function coreReason(name:string){const core=metaByName.value.get(name)?.core;const id=core?.state==='known'?undefined:coreReasonId(core?.reason);return id?t(id):undefined}
const coreVersions = computed(() => new Map<string, {version:string;build?:string;update?:string}>(props.meta.map(({ name, core }) => {
  if (core?.state !== 'known') return [name, { version: coreLabel(name) }]
  const source = props.coreCheckRows?.[name]
  const build = core.build
  const update = build && source?.status === 'UpdateAvailable'
    && source.family?.toLowerCase() === core.family?.toLowerCase()
    && source.minecraft === core.minecraft && source.build === build
    && source.remoteBuild !== build ? source.remoteBuild : undefined
  return [name, {
    version: core.family && core.minecraft ? `${core.family} ${core.minecraft}` : coreLabel(name),
    build,
    update,
  }]
})))
function pathLabel(path:string){return displayPath(path,props.preferences.pathSeparator)}
const jobStateIds:Record<string,MessageId>={Completed:'activity.state.Completed',Unconfirmed:'activity.state.Unconfirmed',Failed:'activity.state.Failed',Pending:'activity.state.Pending',Cancelled:'activity.state.Cancelled',Skipped:'activity.state.Skipped'}
const jobStateName=(state:string)=>t(Object.hasOwn(jobStateIds,state)?jobStateIds[state]:'activity.state.Unconfirmed')
watch(() => props.rows.map(row => row.name).join('\u0000'), () => {
  const names = new Set(props.rows.map(row => row.name))
  selected.value = selected.value.filter(name => names.has(name))
  if (anchor.value && !names.has(anchor.value)) anchor.value = null
})
function choose(event: MouseEvent | KeyboardEvent, name: string) {
  if(event.type==='click' && pinDrag.consumeClick(name))return
  const names = visible.value.map(row => row.name)
  if (event.shiftKey && anchor.value && names.includes(anchor.value)) {
    const a = names.indexOf(anchor.value), b = names.indexOf(name)
    selected.value = names.slice(Math.min(a,b), Math.max(a,b)+1)
  } else if (event.ctrlKey || event.metaKey) {
    selected.value = selected.value.includes(name) ? selected.value.filter(item => item !== name) : [...selected.value,name]
    anchor.value = name
  } else { selected.value = [name]; anchor.value = name }
}
function clearSelection() { selected.value = []; anchor.value = null }
// Esc clears the selection anywhere on the page, unless a dialog, menu or text field handles it.
function escapeClears(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented || !selected.value.length) return
  if (document.querySelector('dialog[open]')) return
  const target = event.target
  if (target instanceof HTMLElement && target.closest('input, textarea, select, [contenteditable="true"], [role="menu"]')) return
  clearSelection()
  // Leave no focus frame on a row that is no longer selected; a click on empty space behaves the same way.
  if (document.activeElement instanceof HTMLElement && document.activeElement.matches('.instances tbody tr')) document.activeElement.blur()
}
// A click on empty list space clears the selection; rows, controls and scrollbars keep it.
function blankClickClears(event: MouseEvent) {
  const target = event.target
  if (!(target instanceof Element) || target.closest('tr, button, a, input, select, summary, [role="scrollbar"], [role="menu"]')) return
  clearSelection()
}
onMounted(() => window.addEventListener('keydown', escapeClears))
onUnmounted(() => window.removeEventListener('keydown', escapeClears))
function keySelect(event: KeyboardEvent, name: string) {
  if(showKeyboardMenu(event,name))return
  if(event.target !== event.currentTarget)return
  if (event.key === ' ' || event.key === 'Enter') { event.preventDefault(); choose(event, name) }
}
function setSort(next: Exclude<typeof sort.value,null>) {
  if(sort.value!==next){sort.value=next;direction.value=1}
  else if(direction.value===1)direction.value=-1
  else sort.value=null
}
function batchRows(action:'start'|'stop'|'restart', rows:Instance[]) {
  if (action === 'stop') return rows
  return rows.filter(row=>!props.busyNames.has(row.name) && !props.queuedNames?.has(row.name) && !props.startingNames?.has(row.name) &&
    (action === 'start' ? row.state === 'Stopped' && !row.outsideManagement : canRestart(row)))
}
function batchCount(action:'start'|'stop'|'restart', rows:Instance[]) {
  return action === 'stop'
    ? rows.filter(row=>canStop(row)||props.queuedNames?.has(row.name)).length
    : batchRows(action,rows).length
}
function canSubmitBatch(action:'start'|'stop'|'restart', rows:Instance[]) {
  return props.connected && batchCount(action,rows)>0
}

</script>
<template>
  <header class="page-head"><div><h1>{{ t('nav.instances') }}</h1></div>
    <div class="actions"><button class="management-folder-trigger" type="button" :disabled="!connected" @click="emit('add-management-folder')">{{ t('management.add') }}</button><button class="primary" type="button" :disabled="!connected" @click="emit('create')">{{ t('instances.createInstance') }}</button></div>
  </header>
  <div class="toolbar"><label class="search-label">{{ t('instances.searchLabel') }}<input v-model="query" type="search" name="instance-search" autocomplete="off"></label>
    <label>{{ t('instances.statusLabel') }}<select v-model="filter" name="status-filter"><option value="all">{{ t('activity.allResults') }}</option><option value="running">{{ t('instance.state.listening') }}</option><option value="stopped">{{ t('instance.state.stopped') }}</option><option value="attention">{{ t('instances.filterAttention') }}</option></select></label>
    <div class="actions toolbar-controls"><OperationButton class="refresh-control" :label="refreshLabel" :busy-label="t('action.checking')" :busy="refreshBusy" @click="emit('refresh')" />
    <ActionMenu :label="t('instances.allActions')">
      <button :disabled="!canSubmitBatch('start',rows)" @click="emit('batch-action','start',batchRows('start',rows))">{{ t('instances.startAll') }}</button>
      <button :disabled="!canSubmitBatch('stop',rows)" @click="emit('batch-action','stop',batchRows('stop',rows))">{{ t('instances.stopAll') }}</button>
      <button :disabled="!canSubmitBatch('restart',rows)" @click="emit('batch-action','restart',batchRows('restart',rows))">{{ t('instances.restartAll') }}</button>
      <button type="button" :disabled="!connected" @click="emit('batch-core')">{{ t('core.batch.all') }}</button>
      <button type="button" :disabled="!connected" @click="emit('removed')">{{ t('instances.removedInstances') }}</button>
    </ActionMenu></div>
  </div>
  <p class="operation-status selectable" role="status">{{ feedback }}</p>
  <div class="workspace">
    <section class="list-panel" :aria-label="t('instances.listLabel')" @click="blankClickClears"><div v-if="!loaded" class="selection-line"><span>{{ t('instances.listNotLoaded') }}</span></div>
      <OverlayScroll v-if="initialLoading || visible.length" class="table-wrap" :label="t('instances.listLabel')"><table ref="instanceTable" class="instances"><thead><tr><th :aria-sort="sort==='name' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('name')">{{ t('nav.instances') }} {{ sort==='name' ? (direction===1?'↑':'↓') : '' }}</button></th><th class="core-column" :aria-sort="sort==='core' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('core')">{{ t('instances.colCore') }} {{ sort==='core' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='port' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('port')">Port {{ sort==='port' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='state' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('state')">{{ t('instances.observedState') }} {{ sort==='state' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='recent' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('recent')">{{ t('instances.recentAction') }} {{ sort==='recent' ? (direction===1?'↑':'↓') : '' }}</button></th></tr></thead>
        <tbody><tr v-for="row in displayed" :key="row.name" :class="{selected:selected.includes(row.name),'pin-drag-source':pinDrag.dragging.value===row.name,'pinned-draggable':preferences.pins.includes(row.name)&&canDragPins}" :data-instance="row.name" tabindex="0" :aria-selected="selected.includes(row.name)" @pointerdown.capture="pinDrag.nextPointerDown" @pointerdown="pinDrag.begin($event,row.name)" :title="preferences.pins.includes(row.name)?(canDragPins?t('instances.pinDragHint'):t('instances.pinDragBlocked')):undefined" @click="choose($event,row.name)" @keydown="keySelect($event,row.name)" @contextmenu="showMenu($event,row.name)">
          <td class="instance-name-cell"><button class="pin-toggle" type="button" :class="{pinned:preferences.pins.includes(row.name)}" :aria-pressed="preferences.pins.includes(row.name)" :aria-label="preferences.pins.includes(row.name)?t('instances.unpinNamed',{name:row.name}):t('instances.pinNamed',{name:row.name})" :title="preferences.pins.includes(row.name)?t('instances.unpinInstance'):t('instances.pinInstance')" :aria-disabled="pinBusy || !connected" @click.stop="!pinBusy && connected && emit('pin',row.name)" @dblclick.stop @keydown.stop="showKeyboardMenu($event,row.name)"><PinIcon :filled="preferences.pins.includes(row.name)" /></button><span class="instance-name">{{ row.name }}</span></td>
          <td class="core-column">
            <div class="core-version">
              <span :title="coreReason(row.name)">{{ coreVersions.get(row.name)?.version ?? t('instances.coreUnchecked') }}</span>
              <small v-if="coreVersions.get(row.name)?.build" class="core-build" translate="no">
                Build {{ coreVersions.get(row.name)?.build }}
                <template v-if="coreVersions.get(row.name)?.update">
                  <span aria-hidden="true"> → {{ coreVersions.get(row.name)?.update }}</span>
                  <span class="sr-only" translate="yes"> {{ t('core.batch.updateTo', { build: coreVersions.get(row.name)?.update ?? '' }) }}</span>
                </template>
              </small>
            </div>
          </td><td class="number">{{ row.port }}</td><td><span class="badge" :class="stateClass(row)">{{ displayState(row) }}</span></td><td>{{ recentByName.get(row.name) ? `${actionName(recentByName.get(row.name)!.action)} · ${jobStateName(recentByName.get(row.name)!.state)}` : t('instances.noActions') }}</td>
        </tr></tbody></table><InitialQueryProgress v-if="initialLoading" :label="t('instances.loadingList')" /></OverlayScroll><div v-else class="empty list-empty"><p>{{ !loaded ? t('instances.emptyNotLoaded') : rows.length ? t('instances.emptyNoMatch') : t('instances.emptyNone') }}</p><div v-if="loaded && !rows.length" class="actions"><button class="primary" type="button" :disabled="!connected" @click="emit('create')">{{ t('instances.createInstance') }}</button><button type="button" :disabled="!connected" @click="emit('add-management-folder')">{{ t('management.add') }}</button></div></div>
    </section>
    <aside class="detail-panel" :aria-label="t('instances.detailLabel')"><OverlayScroll class="detail-scroll" :label="t('instances.detailContentLabel')"><div class="detail-content">
      <template v-if="chosen.length > 1"><h2>{{ t('instances.selectedHeading', { count: chosen.length }) }}</h2>
        <div class="detail-actions">
          <button :class="{primary:canSubmitBatch('start',chosen)}" :disabled="!canSubmitBatch('start',chosen)" @click="emit('batch-action','start',batchRows('start',chosen))">{{ t('instances.startSelected',{count:batchCount('start',chosen)}) }}</button>
          <span v-if="consoleAvailable" class="action-tooltip" :title="consoleHint">
            <button :class="{primary:connected && consoleTargets.length && !canSubmitBatch('start',chosen)}" :disabled="!connected || consoleBusy || !consoleTargets.length" aria-describedby="selected-console-hint" @click="emit('console',consoleTargets)">{{ t('instances.consoleSelected') }}</button>
            <span id="selected-console-hint" class="sr-only">{{ consoleHint }}</span>
          </span>
          <button :disabled="!canSubmitBatch('stop',chosen)" @click="emit('batch-action','stop',batchRows('stop',chosen))">{{ t('instances.stopSelected',{count:batchCount('stop',chosen)}) }}</button>
          <button :disabled="!canSubmitBatch('restart',chosen)" @click="emit('batch-action','restart',batchRows('restart',chosen))">{{ t('instances.restartSelected',{count:batchCount('restart',chosen)}) }}</button>
        </div>
        <button type="button" :disabled="!connected" @click="emit('batch-core',chosen.map(row=>row.name))">{{ t('core.batch.selected') }}</button>
        <details><summary>{{ t('instances.viewTargets') }}</summary><p v-for="row in chosen" :key="row.name" class="small">{{ row.name }} · {{ displayState(row) }}</p></details>
      </template>
      <template v-else-if="single"><h2>{{ single.name }}</h2>
        <p class="hint">{{ coreLabel(single.name) }}<template v-if="coreReason(single.name)"> · {{ coreReason(single.name) }}</template><br>Port {{ single.port }}<span v-if="single.listenAddress"> · <bdi class="selectable" :title="t('velocity.listenAddress')" translate="no">{{ single.listenAddress }}</bdi></span></p><p v-if="single.listenAddress==='0.0.0.0'" class="form-error" role="note">{{ t('velocity.allInterfaces') }}</p><p v-if="metaByName.get(single.name)?.core?.unverified" class="form-error" role="note">{{ t('core.warning.unverifiedInstance') }}</p><button type="button" :disabled="!connected" @click="emit('core',single.name)">{{ t('settings.core') }}</button>
        <dl><dt>{{ t('instances.observedState') }}</dt><dd><span class="badge" :class="stateClass(single)">{{ displayState(single) }}</span></dd><dt v-if="single.outsideManagement">{{ t('instances.managementScope') }}</dt><dd v-if="single.outsideManagement">{{ t('instances.excludedCanStop') }}</dd><dt>{{ t('instances.listener') }}</dt><dd>{{ single.ownerPid ? t('instances.listenerOwner', { name: single.ownerName ?? t('instances.nameUnconfirmed'), pid: single.ownerPid }) : t('instances.unconfirmed') }}</dd><dt>{{ t('console.tier.label') }}</dt><dd>{{ consoleText(single) }}<template v-if="single.console?.tier==='window' && canStop(single)"><br><span class="form-error">{{ t('console.windowWarning') }}</span></template></dd></dl>
        <div class="detail-actions">
          <button v-if="emergency" class="danger" :disabled="forceBusy" @click="emit('force',single)">{{ t('instances.forceStop') }}</button>
          <button v-else-if="single.state==='Stopped'" class="primary" :disabled="!connected || single.outsideManagement || busyNames.has(single.name) || isQueued(single) || starting(single)" @click="emit('action','start',[single])">{{ starting(single)?t('instance.state.starting'):isQueued(single)?t('activity.state.Queued'):t('activity.action.start') }}</button>
          <button v-else :disabled="!connected || busyNames.has(single.name) || !canStop(single)" @click="emit('action','stop',[single])">{{ t('instances.stop') }}</button>
          <button v-if="consoleAvailable" :class="{primary:connected && canOpenConsole(single) && !emergency}" :disabled="!connected || consoleBusy || !canOpenConsole(single)" @click="emit('console',[single])">{{ t('instances.openConsole') }}</button>
          <button :disabled="!connected || busyNames.has(single.name) || isQueued(single) || startingNames?.has(single.name) || !canRestart(single)" @click="emit('action','restart',[single])">{{ t('instances.restart') }}</button>
          <button @click="emit('settings',single.name)">{{ t('instances.settings') }}</button>
        </div>
        <div class="detail-actions">
          <button type="button" :disabled="!connected" @click="emit('open-folder',single.name)">{{ t('instances.openServerFolder') }}</button>
          <button type="button" class="danger" :disabled="!removable(single)" @click="emit('remove',single.name)">{{ t('instances.removeManagement') }}</button>
        </div>

        <div class="recent-operation detail-section"><p v-if="recent" class="hint">{{ t('instances.recentLine', { action: actionName(recent.action), state: jobStateName(recent.state) }) }}</p></div>
        <details class="detail-section diagnostics"><summary>{{ t('instances.diagnosticsSummary') }}</summary><p class="mono">{{ t('instances.processState', { state: stateName(single.state) }) }}<br>{{ t('instances.run', { id: single.run?.id ?? t('instances.none') }) }}<br>{{ t('instances.java', { path: pathLabel(currentMeta?.java ?? t('instances.notProvided')) }) }}<br>{{ t('instances.location', { path: pathLabel(currentMeta?.path ?? t('instances.notProvided')) }) }}</p><button @click="emit('logs',single.name)">{{ t('instances.viewStartupLog') }}</button>
          <InstanceDiagnosticsPanel :key="single.name" :instance="single.name" :rows="rows" :proxies="diagnosticProxies" :connected="connected" :time-format="preferences.timeFormat" />
        </details>
      </template>
      <template v-else><h2>{{ t('instances.selectInstance') }}</h2><p class="hint">{{ t('instances.selectHint') }}</p></template>
    </div></OverlayScroll></aside>
  </div>
  <ContextMenu v-if="menu" :key="menu.name+':'+menu.x+':'+menu.y" :x="menu.x" :y="menu.y" :label="menu.name" :items="menuItems" @action="menuAction" @close="closeMenu" />
  <span class="sr-only" role="status">{{ pinDrag.dragging.value?t('instances.reordering',{name:pinDrag.dragging.value}):'' }}</span>
</template>
