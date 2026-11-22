<script setup lang="ts">
import { computed, ref } from 'vue'
import OverlayScroll from './components/OverlayScroll.vue'
import OperationButton from './components/OperationButton.vue'
import InitialQueryProgress from './components/InitialQueryProgress.vue'
import { formatDisplayTime, isMessageId, useAppLanguage, type TimeFormat } from './i18n'
import type { Job } from './manager'
const props=defineProps<{jobs:Job[]; initialLoading?:boolean; loaded?:boolean; error:string; batchError:string; cancellingBatch?:string; cancellingCoreBatch?:string; coreCancelFailure?:string; feedback:''|'reading'|'read'; timeFormat?:TimeFormat}>()
const emit = defineEmits<{(e:'refresh'):void;(e:'cancel-batch',id:string):void;(e:'cancel-core-batch',id:string):void}>()
const { t, locale } = useAppLanguage()
const coreOnly = ref(false)
const includeOlder=ref(false)
const stateFilter=ref('')
const exportFormat=ref<'json'|'csv'>('json')
const exportRequested=ref(false)
const visible=computed(()=>props.jobs.filter(job=>{
  if(coreOnly.value && !job.action.startsWith('core-'))return false
  if(stateFilter.value && job.state!==stateFilter.value)return false
  return includeOlder.value || !['Completed','VerifiedLive','Cancelled','Failed','Skipped'].includes(job.state)
    || !job.createdAt || Date.now()-job.createdAt < 90*24*60*60*1000
}))
function exportJobs(){
  const fields=visible.value.map(job=>({
    id:job.id,action:job.action,instance:job.instance,state:job.state,
    createdAt:job.createdAt ? new Date(job.createdAt).toISOString() : null
  }))
  const escape=(value:string|null)=>{
    const cell=value??''
    return '"'+(/^[=+@\-\t\r]/.test(cell)?"'":'')+cell.replaceAll('"','""')+'"'
  }
  const csv=['"id","action","instance","state","createdAt"',...fields.map(row=>Object.values(row).map(escape).join(','))].join('\r\n')
  const file=new Blob([exportFormat.value==='json'?JSON.stringify(fields,null,2):csv],{type:exportFormat.value==='json'?'application/json':'text/csv;charset=utf-8'})
  const url=URL.createObjectURL(file)
  const anchor=document.createElement('a')
  anchor.href=url
  anchor.download='instanceharbor-operations.'+exportFormat.value
  anchor.click()
  window.setTimeout(()=>URL.revokeObjectURL(url),1000)
  exportRequested.value=true
}
const filterStates = ['Completed', 'VerifiedLive', 'Pending', 'Unconfirmed', 'Failed', 'Cancelled'] as const
function resultName(value: string) {
  const id = 'activity.state.' + value
  return t(isMessageId(id) ? id : 'activity.state.Unconfirmed')
}
function itemPhaseName(action: string, phase: string) {
  if (action === 'core-batch-install' && phase === 'Sending') return t('core.batch.Sending')
  return resultName(phase)
}
function reasonName(value: string) {
  const id = 'activity.reason.' + value
  return t(isMessageId(id) ? id : 'activity.checkDiagnostics')
}
function actionName(value: string) {
  const id = 'activity.action.' + value
  return isMessageId(id) ? t(id) : value
}
function timeName(value?: number | null) {
  return value != null && Number.isFinite(new Date(value).getTime())
    ? formatDisplayTime(value, locale.value, props.timeFormat) : t('activity.missingTime')
}
</script>
<template>
  <header class="page-head"><div><h1>{{ t('activity.heading') }}</h1></div><div class="actions"><button :aria-pressed="!coreOnly" @click="coreOnly=false">{{ t('activity.all') }}</button><button :aria-pressed="coreOnly" @click="coreOnly=true">{{ t('activity.core') }}</button><OperationButton class="refresh-control" :label="t(feedback==='read'?'activity.loaded':'activity.reload')" :busy-label="t('activity.loading')" :busy="feedback==='reading'" @click="emit('refresh')" /></div></header>
  <div class="actions activity-filter">
    <label>{{ t('activity.result') }} <select v-model="stateFilter" name="job-status">
      <option value="">{{ t('activity.allResults') }}</option>
      <option v-for="state in filterStates" :key="state" :value="state">{{ resultName(state) }}</option>
    </select></label>
    <label class="choice"><input v-model="includeOlder" type="checkbox" name="show-old">{{ t('activity.older') }}</label>
    <label>{{ t('activity.exportFormat') }} <select v-model="exportFormat" name="export-format" translate="no"><option value="json">JSON</option><option value="csv">CSV</option></select></label>
    <button :disabled="!visible.length" @click="exportJobs">{{ t('activity.export') }}</button>
    <span v-if="exportRequested" role="status" class="hint">{{ t('activity.exportRequested') }}</span>
  </div>
  <OverlayScroll class="activity-scroll" :label="t('activity.heading')"><section class="sheet activity"><p v-if="error" role="alert" class="form-error">{{ error }}</p><InitialQueryProgress v-else-if="initialLoading && !jobs.length" :label="t('activity.loading')" /><p v-else-if="loaded && !visible.length" class="empty">{{ t('activity.empty') }}</p>
    <p v-if="batchError" role="status" class="form-error">{{ batchError }}</p>
    <article v-for="job in visible" :key="job.action+':'+job.id" class="operation-row selectable">
      <div><strong>{{ actionName(job.action) }}</strong><p v-if="!job.items" class="hint" translate="no">{{ job.instance }}</p><p class="hint">{{ timeName(job.createdAt) }}</p></div>
      <div><p>{{ resultName(job.state) }}</p><p v-if="job.message" class="hint">{{ job.message }}</p>
        <ul v-if="job.items" class="batch-items">
          <li v-for="item in job.items" :key="item.name"><bdi translate="no">{{ item.name }}</bdi> · {{ itemPhaseName(job.action,item.phase) }}<span v-if="item.reason"> · {{ reasonName(item.reason) }}</span></li>
        </ul>
        <template v-if="job.action==='core-batch-install'">
          <OperationButton v-if="job.items?.some(item=>item.canCancel) || cancellingCoreBatch===job.id"
            :label="t('core.batch.cancel')" :busy-label="t('action.submitting')" :busy="cancellingCoreBatch===job.id"
            :disabled="!!cancellingCoreBatch && cancellingCoreBatch!==job.id" @click="emit('cancel-core-batch',job.id)" />
          <p v-if="coreCancelFailure===job.id" role="alert" class="form-error">{{ t('core.batch.cancelFailed') }}</p>
        </template>
        <OperationButton v-else-if="job.items?.some(item=>item.canCancel) || cancellingBatch===job.id"
          :label="t('activity.cancelRemaining')" :busy-label="t('action.submitting')" :busy="cancellingBatch===job.id"
          :disabled="!!cancellingBatch && cancellingBatch!==job.id" @click="emit('cancel-batch',job.id)" />
      </div>
      <code :title="job.id" translate="no">{{ job.id }}</code>
    </article>
  </section></OverlayScroll>
</template>
<style scoped>
.activity-filter { margin-block-end: .75rem; }
.activity-scroll { flex:1; min-height:0; }
.batch-items { padding-inline-start: 1.25rem; overflow-wrap: anywhere; }
</style>
