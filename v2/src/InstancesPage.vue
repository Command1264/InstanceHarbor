<script setup lang="ts">
import { computed, nextTick, onBeforeUpdate, onUpdated, ref, watch } from 'vue'
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
import { useAppLanguage } from './i18n'
import { actionName, batchItemState, canOpenConsole, canRestart, canStop, stateName, type CoreSource, type Instance, type InstanceMeta, type Job, type Preferences } from './manager'

const props = defineProps<{coreCheckRows?:Record<string,CoreSource>;
  rows: Instance[]; initialLoading?: boolean; loaded?: boolean; meta: InstanceMeta[]; preferences: Preferences; busyNames: Set<string>; feedback:string; refreshPending?: boolean; refreshDone?: boolean;
  queuedNames?: Set<string>; startingNames?: Set<string>; pinBusy?: boolean; stopping: Record<string, string>; forceBusy: boolean; consoleBusy: boolean; connected: boolean; jobs: Job[]
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
    (filter.value === 'running' && row.state === 'Listening') ||
    (filter.value === 'stopped' && row.state === 'Stopped') ||
    (filter.value === 'attention' && !['Listening','Stopped'].includes(row.state)))
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
 const pins=index<0?[{id:'pin',label:'釘選',disabled:blocked}]:[{id:'up',label:'釘選上移',disabled:blocked||index===0},{id:'down',label:'釘選下移',disabled:blocked||index===props.preferences.pins.length-1},{id:'pin',label:'取消釘選',disabled:blocked}]
 return [...pins,{id:'remove',label:'移除管理',danger:true,disabled:!props.connected||!row||row.state!=='Stopped'||props.busyNames.has(name)||!!props.queuedNames?.has(name)}]
})
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
const consoleHint = '只開啟具備後台且正在啟動或監聽中的所選實例。'
const single = computed(() => chosen.value.length === 1 ? chosen.value[0] : undefined)
const currentMeta = computed(() => single.value && metaByName.value.get(single.value.name))
const diagnosticProxies = computed(() => props.meta.filter(item => item.core?.family?.toLowerCase() === 'velocity').map(item => item.name))
function isQueued(row:Instance){return row.state==='Stopped' && !!props.queuedNames?.has(row.name)}
function starting(row:Instance){return row.state==='Stopped' && !!props.startingNames?.has(row.name)}
function stateClass(row:Instance){return isQueued(row)?'Queued':starting(row)?'Starting':row.state}
function displayState(row:Instance){const state=stateClass(row);return state==='Queued'?'排隊中':stateName(state)}
const emergency = computed(() => !!single.value && canStop(single.value) &&
  !!(single.value.stopPhase || props.stopping[single.value.name] === single.value.run?.id))
const recent = computed(() => single.value && recentByName.value.get(single.value.name))
const refreshBusy=computed(()=>!!props.refreshPending)
const refreshLabel=computed(()=>props.refreshDone?'已核對':'重新核對')
function coreLabel(name:string){const core=metaByName.value.get(name)?.core;return core?.state==='known' ? core.label ?? `${core.family} ${core.minecraft}${core.build ? ` build ${core.build}` : ''}` : '尚未核對'}
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
const jobStateName=(state:string)=>({Completed:'已完成',Unconfirmed:'結果未確認',Failed:'失敗',Pending:'處理中',Cancelled:'已取消',Skipped:'已略過'} as Record<string,string>)[state]??'結果未確認'
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
function keySelect(event: KeyboardEvent, name: string) {
  if(showKeyboardMenu(event,name))return
  if(event.target !== event.currentTarget)return
  if (event.key === 'Escape') { selected.value=[];anchor.value=null;return }
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
function canSubmitBatch(action:'start'|'stop'|'restart', rows:Instance[]) {
  return props.connected && (action === 'stop'
    ? rows.some(row=>canStop(row)||props.queuedNames?.has(row.name))
    : batchRows(action,rows).length>0)
}

</script>
<template>
  <header class="page-head"><div><h1>實例</h1></div>
    <div class="actions"><button class="management-folder-trigger" type="button" :disabled="!connected" @click="emit('add-management-folder')">{{ t('management.add') }}</button><button class="primary" type="button" :disabled="!connected" @click="emit('create')">建立實例</button></div>
  </header>
  <div class="toolbar"><label class="search-label">搜尋實例<input v-model="query" type="search" name="instance-search" autocomplete="off"></label>
    <label>狀態<select v-model="filter" name="status-filter"><option value="all">全部</option><option value="running">正在監聽</option><option value="stopped">已停止</option><option value="attention">需查看</option></select></label>
    <div class="actions toolbar-controls"><OperationButton class="refresh-control" :label="refreshLabel" :busy-label="t('action.checking')" :busy="refreshBusy" @click="emit('refresh')" />
    <ActionMenu label="全部操作">
      <button :disabled="!canSubmitBatch('start',rows)" @click="emit('batch-action','start',batchRows('start',rows))">全部啟動</button>
      <button :disabled="!canSubmitBatch('stop',rows)" @click="emit('batch-action','stop',batchRows('stop',rows))">全部正常停止</button>
      <button :disabled="!canSubmitBatch('restart',rows)" @click="emit('batch-action','restart',batchRows('restart',rows))">全部重新啟動</button>
      <button type="button" :disabled="!connected" @click="emit('batch-core')">{{ t('core.batch.all') }}</button>
    </ActionMenu></div>
  </div>
  <p class="operation-status selectable" role="status">{{ feedback }}</p>
  <div class="workspace">
    <section class="list-panel" aria-label="實例清單"><div v-if="!loaded || selected.length" class="selection-line"><span>{{ !loaded ? '尚未取得清單' : selected.length ? `已選取 ${selected.length} 台` : '' }}</span><span><button v-if="selected.length" class="quiet clear-selection" @click="selected=[];anchor=null">清除選取</button></span></div>
      <OverlayScroll v-if="initialLoading || visible.length" class="table-wrap" label="實例清單"><table ref="instanceTable" class="instances"><thead><tr><th :aria-sort="sort==='name' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('name')">實例 {{ sort==='name' ? (direction===1?'↑':'↓') : '' }}</button></th><th class="core-column" :aria-sort="sort==='core' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('core')">核心版本 {{ sort==='core' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='port' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('port')">Port {{ sort==='port' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='state' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('state')">觀察狀態 {{ sort==='state' ? (direction===1?'↑':'↓') : '' }}</button></th><th :aria-sort="sort==='recent' ? (direction===1?'ascending':'descending') : 'none'"><button type="button" @click="setSort('recent')">最近操作 {{ sort==='recent' ? (direction===1?'↑':'↓') : '' }}</button></th></tr></thead>
        <tbody><tr v-for="row in displayed" :key="row.name" :class="{selected:selected.includes(row.name),'pin-drag-source':pinDrag.dragging.value===row.name,'pinned-draggable':preferences.pins.includes(row.name)&&canDragPins}" :data-instance="row.name" tabindex="0" :aria-selected="selected.includes(row.name)" @pointerdown.capture="pinDrag.nextPointerDown" @pointerdown="pinDrag.begin($event,row.name)" :title="preferences.pins.includes(row.name)?(canDragPins?'按住此列拖曳，調整釘選順序':'清除篩選並等待儲存完成後，可拖曳調整釘選順序'):undefined" @click="choose($event,row.name)" @keydown="keySelect($event,row.name)" @contextmenu="showMenu($event,row.name)">
          <td class="instance-name-cell"><button class="pin-toggle" type="button" :class="{pinned:preferences.pins.includes(row.name)}" :aria-pressed="preferences.pins.includes(row.name)" :aria-label="`${preferences.pins.includes(row.name)?'取消釘選':'釘選'} ${row.name}`" :title="preferences.pins.includes(row.name)?'取消釘選實例':'釘選實例'" :aria-disabled="pinBusy || !connected" @click.stop="!pinBusy && connected && emit('pin',row.name)" @dblclick.stop @keydown.stop="showKeyboardMenu($event,row.name)"><PinIcon :filled="preferences.pins.includes(row.name)" /></button><span class="instance-name">{{ row.name }}</span></td>
          <td class="core-column">
            <div class="core-version">
              <span>{{ coreVersions.get(row.name)?.version ?? '尚未核對' }}</span>
              <small v-if="coreVersions.get(row.name)?.build" class="core-build" translate="no">
                Build {{ coreVersions.get(row.name)?.build }}
                <template v-if="coreVersions.get(row.name)?.update">
                  <span aria-hidden="true"> → {{ coreVersions.get(row.name)?.update }}</span>
                  <span class="sr-only" translate="yes"> {{ t('core.batch.updateTo', { build: coreVersions.get(row.name)?.update ?? '' }) }}</span>
                </template>
              </small>
            </div>
          </td><td class="number">{{ row.port }}</td><td><span class="badge" :class="stateClass(row)">{{ displayState(row) }}</span></td><td>{{ recentByName.get(row.name) ? `${actionName(recentByName.get(row.name)!.action)} · ${jobStateName(recentByName.get(row.name)!.state)}` : '尚無操作' }}</td>
        </tr></tbody></table><InitialQueryProgress v-if="initialLoading" label="正在讀取實例清單" /></OverlayScroll><p v-else class="empty list-empty">{{ !loaded ? '尚未取得實例清單，請按重新核對。' : rows.length ? '沒有符合條件的實例。' : '沒有可顯示的實例。' }}</p>
    </section>
    <aside class="detail-panel" aria-label="實例詳細資料"><OverlayScroll class="detail-scroll" label="實例詳細資料內容"><div class="detail-content">
      <template v-if="chosen.length > 1"><h2>已選取 {{ chosen.length }} 台實例</h2>
        <div class="detail-actions">
          <button :class="{primary:canSubmitBatch('start',chosen)}" :disabled="!canSubmitBatch('start',chosen)" @click="emit('batch-action','start',batchRows('start',chosen))">啟動所選</button>
          <span class="action-tooltip" :title="consoleHint">
            <button :class="{primary:connected && consoleTargets.length && !canSubmitBatch('start',chosen)}" :disabled="!connected || consoleBusy || !consoleTargets.length" aria-describedby="selected-console-hint" @click="emit('console',consoleTargets)">開啟所選後台</button>
            <span id="selected-console-hint" class="sr-only">{{ consoleHint }}</span>
          </span>
          <button :disabled="!canSubmitBatch('stop',chosen)" @click="emit('batch-action','stop',batchRows('stop',chosen))">正常停止所選</button>
          <button :disabled="!canSubmitBatch('restart',chosen)" @click="emit('batch-action','restart',batchRows('restart',chosen))">重新啟動所選</button>
        </div>
        <button type="button" :disabled="!connected" @click="emit('batch-core',chosen.map(row=>row.name))">{{ t('core.batch.selected') }}</button>
        <details><summary>查看所選目標</summary><p v-for="row in chosen" :key="row.name" class="small">{{ row.name }} · {{ displayState(row) }}</p></details>
      </template>
      <template v-else-if="single"><h2>{{ single.name }}</h2>
        <p class="hint">{{ coreLabel(single.name) }}<br>Port {{ single.port }}<span v-if="single.listenAddress"> · <bdi class="selectable" :title="t('velocity.listenAddress')" translate="no">{{ single.listenAddress }}</bdi></span></p><button type="button" :disabled="!connected" @click="emit('core',single.name)">核心維護</button>
        <dl><dt>觀察狀態</dt><dd><span class="badge" :class="stateClass(single)">{{ displayState(single) }}</span></dd><dt v-if="single.outsideManagement">管理範圍</dt><dd v-if="single.outsideManagement">已排除，仍可正常停止</dd><dt>監聽程序</dt><dd>{{ single.ownerPid ? `${single.ownerName ?? '名稱未確認'} · PID ${single.ownerPid}` : '未確認' }}</dd></dl>
        <div class="detail-actions">
          <button v-if="emergency" class="danger" :disabled="forceBusy" @click="emit('force',single)">強制停止</button>
          <button v-else-if="single.state==='Stopped'" class="primary" :disabled="!connected || single.outsideManagement || busyNames.has(single.name) || isQueued(single) || starting(single)" @click="emit('action','start',[single])">{{ starting(single)?'啟動中':isQueued(single)?'排隊中':'啟動' }}</button>
          <button v-else :disabled="!connected || busyNames.has(single.name) || !canStop(single)" @click="emit('action','stop',[single])">正常停止</button>
          <button :class="{primary:connected && canOpenConsole(single) && !emergency}" :disabled="!connected || consoleBusy || !canOpenConsole(single)" @click="emit('console',[single])">開啟後台</button>
          <button :disabled="!connected || busyNames.has(single.name) || isQueued(single) || startingNames?.has(single.name) || !canRestart(single)" @click="emit('action','restart',[single])">重新啟動</button>
          <button @click="emit('settings',single.name)">實例設定</button>
        </div>

        <div class="recent-operation detail-section"><p v-if="recent" class="hint">最近操作：{{ actionName(recent.action) }} · {{ jobStateName(recent.state) }}</p></div>
        <details class="detail-section diagnostics"><summary>工作階段與診斷資料</summary><p class="mono">實際程序狀態：{{ stateName(single.state) }}<br>Run：{{ single.run?.id ?? '無' }}<br>Java：{{ pathLabel(currentMeta?.java ?? '尚未提供') }}<br>位置：{{ pathLabel(currentMeta?.path ?? '尚未提供') }}</p><button @click="emit('logs',single.name)">查看啟動紀錄</button>
          <InstanceDiagnosticsPanel :key="single.name" :instance="single.name" :rows="rows" :proxies="diagnosticProxies" :connected="connected" :time-format="preferences.timeFormat" />
        </details>
      </template>
      <template v-else><h2>選取實例</h2><p class="hint">選一台查看資訊與操作，或按 Ctrl／Shift 選取多台。</p></template>
    </div></OverlayScroll></aside>
  </div>
  <ContextMenu v-if="menu" :key="menu.name+':'+menu.x+':'+menu.y" :x="menu.x" :y="menu.y" :label="menu.name" :items="menuItems" @action="menuAction" @close="closeMenu" />
  <span class="sr-only" role="status">{{ pinDrag.dragging.value?'正在調整 '+pinDrag.dragging.value+' 的釘選順序':'' }}</span>
</template>
