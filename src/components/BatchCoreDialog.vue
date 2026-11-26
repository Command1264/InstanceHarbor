<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { isMessageId, useAppLanguage, type MessageId } from '../i18n'
import { canCancelCoreBatchItem, manage, rawError, type CoreBatch, type CoreSource, type Instance, type InstanceMeta } from '../manager'
import CoreUpdateProgressBar from './CoreUpdateProgressBar.vue'
import OperationButton from './OperationButton.vue'
import OverlayScroll from './OverlayScroll.vue'

const props = defineProps<{
  instances: Instance[]
  meta: InstanceMeta[]
  sourceRows: Record<string, CoreSource>
  batches: CoreBatch[]
  connected: boolean
  blockedNames: Set<string>
  cancelling?: string
  cancelFailure?: string
  query: (names: string[]) => Promise<{ rows: CoreSource[] }>
}>()
const emit = defineEmits<{
  changed: []
  submitted: [batch?: CoreBatch]
  cancel: [id: string]
}>()
type ReportRow = {
  name: string
  result: MessageId
  source?: CoreSource
}
const { locale, t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const closeButton = ref<HTMLButtonElement>()
const checkButton = ref<InstanceType<typeof OperationButton>>()
const phase = ref<'idle' | 'checking' | 'ready' | 'submitting' | 'running' | 'done'>('idle')
const requestId = ref('')
const selected = ref(false)
const scope = ref<string[]>([])
const report = ref<ReportRow[]>([])
const resultBatch = ref<CoreBatch>()
const error = ref<MessageId>()
const submittingLabel = ref<MessageId>('core.batch.confirm')
const submitLabel = computed(() => phase.value === 'submitting' ? submittingLabel.value
  : requestId.value ? 'core.batch.retrySubmit' : 'core.batch.confirm')
const busy = computed(() => phase.value === 'checking' || phase.value === 'submitting')
const current = computed(() => props.batches.find(batch => batch.id === requestId.value))
const instanceByName = computed(() => new Map(props.instances.map(instance => [instance.name, instance])))
const metaByName = computed(() => new Map(props.meta.map(instance => [instance.name, instance])))
const ready = computed(() => report.value.flatMap(row => row.source ? [{name: row.name, source: row.source}] : []))
const notSubmitted = computed(() => report.value.filter(row => !row.source))
const maximumTargets = 64
const scopeTooLarge = computed(() => scope.value.length > maximumTargets)
const total = computed(() => resultBatch.value?.items.length ?? scope.value.length)
const processed = computed(() => resultBatch.value?.items.filter(item => item.phase !== 'Sending' && !canCancelCoreBatchItem(item)).length ?? 0)
const canCancel = computed(() => resultBatch.value?.items.some(canCancelCoreBatchItem) ?? false)
// The item the Manager works on now, with the step it reported (#60).
const working = computed(() => phase.value === 'running'
  ? resultBatch.value?.items.find(item => item.phase === 'Preparing' || item.phase === 'Sending') : undefined)
const cancelling = computed(() => !!requestId.value && props.cancelling === requestId.value)
const formattedCount = (value: number) => new Intl.NumberFormat(locale.value).format(value)

function identityOf(name: string) {
  const instance = instanceByName.value.get(name)
  const meta = metaByName.value.get(name)
  return JSON.stringify([instance?.state, instance?.run?.id, instance?.port, meta?.path, meta?.java, meta?.core, props.blockedNames.has(name)])
}
function sourceIdentity(source?: CoreSource) {
  return JSON.stringify([source?.sha256, source?.candidate?.sha256, source?.registrationRevision])
}
function sourceMessage(source?: CoreSource, fallback: MessageId = 'core.batch.checkFailed'): MessageId {
  if (source?.status === 'NoStableBuild') return 'core.source.noStableBuild'
  if (!source?.reason) return 'core.batch.noCandidate'
  const message = `activity.reason.${source.reason}`
  return isMessageId(message) ? message : fallback
}
const observedIdentity = computed(() => JSON.stringify(scope.value.map(identityOf)))
const previewIdentity = ref('')
const knownSources = ref(new Map<string, string>())
const membershipChanged = computed(() => scope.value.some(name => !instanceByName.value.has(name))
  || (!selected.value && scope.value.length !== props.instances.length))
const stale = computed(() => phase.value === 'ready' && !requestId.value && (
  membershipChanged.value || previewIdentity.value !== observedIdentity.value || ready.value.some(({name, source}) => {
    const observed = sourceIdentity(props.sourceRows[name])
    return observed !== knownSources.value.get(name) && observed !== sourceIdentity(source)
  })
))

function showBatch(batch: CoreBatch) {
  resultBatch.value = batch
  phase.value = batch.state === 'Running' ? 'running' : 'done'
}

watch(current, (batch, previous) => {
  if (!batch || !dialog.value?.open) return
  showBatch(batch)
  if (previous?.state === 'Running' && batch.state !== 'Running') emit('changed')
})
watch(cancelling, async value => {
  if (value || !dialog.value?.open) return
  await nextTick()
  if (document.activeElement === document.body) closeButton.value?.focus()
})

async function check() {
  if (!props.connected || busy.value || phase.value === 'running' || requestId.value || scopeTooLarge.value) return
  if (!scope.value.length || membershipChanged.value) {
    error.value = 'core.batch.noTargets'
    return
  }
  phase.value = 'checking'
  error.value = undefined
  previewIdentity.value = observedIdentity.value
  knownSources.value = new Map(scope.value.map(name => [name, sourceIdentity(props.sourceRows[name])]))
  try {
    const checked = await props.query(scope.value)
    if (membershipChanged.value || previewIdentity.value !== observedIdentity.value) {
      error.value = 'core.batch.reviewChanged'
      phase.value = 'idle'
      return
    }
    const byName = new Map(checked.rows.map(row => [row.name, row]))
    report.value = scope.value.map(name => {
      const source = byName.get(name)
      if (!source) return { name, result: 'core.batch.checkFailed' }
      if (source.status === 'Current') return { name, result: 'core.source.current' }
      if (!source.candidate || !source.sha256) return { name, result: sourceMessage(source) }
      // A Purpur candidate has no SHA-256 before download, so it is updated one at a time (#41).
      if (!source.candidate.sha256) return { name, result: 'core.batch.singleOnly' }
      if (instanceByName.value.get(name)?.state !== 'Stopped') return { name, result: 'core.batch.notStopped' }
      if (props.blockedNames.has(name)) return { name, result: 'core.batch.operationPending' }
      return { name, source, result: 'core.batch.updateTo' }
    })
    phase.value = 'ready'
  } catch {
    error.value = 'core.batch.checkFailed'
    phase.value = 'idle'
  }
  await nextTick()
  if (dialog.value?.open && document.activeElement === document.body) checkButton.value?.focus()
}

async function apply() {
  if (!props.connected || phase.value !== 'ready' || !ready.value.length || stale.value || scopeTooLarge.value) return
  submittingLabel.value = submitLabel.value
  phase.value = 'submitting'
  error.value = undefined
  requestId.value ||= crypto.randomUUID()
  const targets = ready.value.map(({name,source}) => ({
    name, currentSha256: source.sha256, candidateSha256: source.candidate?.sha256,
    registrationRevision: source.registrationRevision,
  }))
  try {
    const batch = await manage<CoreBatch>(['core-batch-submit', requestId.value, JSON.stringify(targets)])
    const observed = resultBatch.value?.id === batch.id ? resultBatch.value : undefined
    showBatch(batch.state === 'Running' && observed ? observed : batch)
    emit('submitted', resultBatch.value)
    if (batch.state !== 'Running' && observed?.state !== batch.state) emit('changed')
  } catch (failure) {
    // A lost reply may mean the Manager already accepted this ID. Reuse the exact request.
    if (resultBatch.value?.id === requestId.value) return
    if (/\b(CoreBatchPreviewChanged|CoreArtifactChanged|CoreTargetUnavailable|RegistrationChanged|InstanceNotConfirmedStopped)\b/.test(rawError(failure))) {
      requestId.value = ''
      previewIdentity.value = ''
      error.value = 'core.batch.reviewChanged'
    } else error.value = 'core.batch.submitFailed'
    phase.value = 'ready'
    emit('submitted')
  }
  await nextTick()
  if (dialog.value?.open && document.activeElement === document.body) closeButton.value?.focus()
}

function close() {
  if (!busy.value && !cancelling.value) dialog.value?.close()
}
function open(names?: string[]) {
  if (dialog.value?.open) return
  phase.value = 'idle'
  report.value = []
  resultBatch.value = undefined
  error.value = undefined
  selected.value = names !== undefined
  scope.value = names?.slice() ?? props.instances.map(instance => instance.name)
  const active = !selected.value ? props.batches.find(batch => batch.state === 'Running') : undefined
  requestId.value = active?.id ?? ''
  if (active) showBatch(active)
  dialog.value?.showModal()
}
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="batch-core-dialog overlay-dialog" aria-labelledby="batch-core-title" @cancel.prevent="close">
    <OverlayScroll class="dialog-scroll" :label="t('core.batch.heading')">
    <h2 id="batch-core-title">{{ t(resultBatch ? 'core.batch.heading' : selected ? 'core.batch.selected' : 'core.batch.all') }}</h2>
    <p v-if="!resultBatch">{{ t('core.batch.scope', { count: formattedCount(total) }) }}</p>
    <template v-if="resultBatch">
      <p role="status">{{ t('core.batch.progress', { finished: formattedCount(processed), total: formattedCount(total) }) }}</p>
      <progress v-if="phase === 'running'" :value="processed" :max="total" :aria-label="t('core.batch.heading')"></progress>
      <CoreUpdateProgressBar v-if="working" :progress="working.progress" :reused="working.reused" :prefix="working.name"
        :fallback="working.phase === 'Sending' ? 'core.batch.Sending' : 'core.batch.Preparing'" />
      <p v-if="canCancel" id="batch-core-cancel-help" class="hint">{{ t('core.batch.cancelHelp') }}</p>
      <dl class="import-preview" :aria-label="t('core.batch.results')">
        <template v-for="item in resultBatch.items" :key="item.name">
          <dt><bdi translate="no">{{ item.name }}</bdi></dt>
          <dd>
            {{ t(`core.batch.${item.phase}`) }}
            <span v-if="item.reason && item.phase === 'Failed'" class="hint">{{ t(sourceMessage({name:item.name,status:'Unknown',reason:item.reason}, 'activity.checkDiagnostics')) }}</span>
          </dd>
        </template>
        <template v-for="row in notSubmitted" :key="row.name">
          <dt><bdi translate="no">{{ row.name }}</bdi></dt>
          <dd>{{ t(row.result) }}</dd>
        </template>
      </dl>
    </template>
    <dl v-else-if="report.length" class="import-preview" :aria-label="t('core.batch.results')">
      <template v-for="row in report" :key="row.name">
        <dt><bdi translate="no">{{ row.name }}</bdi></dt>
        <dd>{{ t(row.result, { build: row.source?.candidate?.build ?? '' }) }}</dd>
      </template>
    </dl>
    <p v-if="requestId && cancelFailure === requestId" class="form-error" role="alert">{{ t('core.batch.cancelFailed') }}</p>
    <p v-else-if="stale" class="form-error" role="alert">{{ t('core.batch.reviewChanged') }}</p>
    <p v-else-if="!resultBatch && scopeTooLarge" class="form-error" role="alert">{{ t('core.batch.limit', { count: formattedCount(maximumTargets) }) }}</p>
    <p v-else-if="error" class="form-error" role="alert">{{ t(error) }}</p>
    <div class="dialog-actions">
      <button ref="closeButton" autofocus type="button" :disabled="busy || cancelling" @click="close">{{ t('core.batch.close') }}</button>
      <OperationButton v-if="canCancel" :label="t('core.batch.cancel')" :busy-label="t('core.batch.cancelling')"
        :disabled="!connected || (!!props.cancelling && !cancelling)" :busy="cancelling" aria-describedby="batch-core-cancel-help"
        @click="emit('cancel', requestId)" />
      <template v-if="!resultBatch">
        <OperationButton ref="checkButton" :label="t('core.batch.check')" :busy-label="t('action.checkingUpdates')"
          :busy="phase === 'checking'" :disabled="phase === 'submitting' || !!requestId || scopeTooLarge || !connected" @click="check" />
        <OperationButton class="primary" :label="t(submitLabel)" :busy-label="t('action.submitting')"
          :busy="phase === 'submitting'" :disabled="(phase !== 'ready' && phase !== 'submitting') || !ready.length || scopeTooLarge || stale || !connected"
          @click="apply" />
      </template>
    </div>
  </OverlayScroll>
  </dialog>
</template>

<style scoped>
.batch-core-dialog { overscroll-behavior: contain; }
.import-preview { grid-template-columns: fit-content(45%) minmax(0, 1fr); }
.import-preview dt { overflow-wrap: anywhere; }
progress { width: 100%; accent-color: var(--primary); }
.dialog-actions button[aria-disabled="true"] { opacity: .55; cursor: not-allowed; }
</style>
