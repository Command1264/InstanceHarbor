<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useAppLanguage, type MessageId } from '../i18n'
import { errorText, manage, rawError } from '../manager'
import { displayPath } from '../path-display'
import { cacheReasonId, formatBytes, type CacheCategory, type CacheItem, type CachePreview, type CacheResult } from '../cache-cleanup'
import OperationButton from './OperationButton.vue'
import OverlayScroll from './OverlayScroll.vue'

const props = defineProps<{ connected: boolean; pathSeparator?: 'forward' | 'native' }>()
const { t, locale } = useAppLanguage()
const preview = ref<CachePreview>()
const readError = ref('')
const manualRead = ref(false)
const clearing = ref(false)
const clearError = ref('')
const results = ref<CacheResult[]>()
const dialog = ref<HTMLDialogElement>()
const cancelButton = ref<HTMLButtonElement>()
let readEpoch = 0

const categories: { id: CacheCategory; label: MessageId }[] = [
  { id: 'download', label: 'components.cache.download' },
  { id: 'legacyWebView', label: 'components.cache.legacyWebView' },
  { id: 'webView', label: 'components.cache.webView' },
  { id: 'sharedCore', label: 'components.cache.sharedCore' },
]
const groups = computed(() => categories.map(category => ({
  ...category,
  items: preview.value?.items.filter(item => item.category === category.id) ?? [],
})))
const ready = computed(() => preview.value?.items.filter(item => item.state === 'ready') ?? [])
const readyBytes = computed(() => ready.value.reduce((sum, item) => sum + item.bytes, 0))
const failures = computed(() => results.value?.filter(result => result.outcome === 'failed') ?? [])

function size(bytes: number, complete = true) {
  const text = formatBytes(bytes, locale.value)
  return complete ? text : t('components.cache.atLeast', { size: text })
}
function path(value: string) {
  return displayPath(value, props.pathSeparator)
}
function reasonText(reason = '') {
  const id = cacheReasonId(reason)
  return id ? t(id) : t('components.cache.reason.other', { reason })
}
function stateText(item: CacheItem) {
  if (item.state === 'inUse') return t('components.cache.inUse')
  if (item.state === 'scheduled') return item.reason ? t('components.cache.scheduledFailed', { reason: reasonText(item.reason) }) : t('components.cache.scheduled')
  if (item.state === 'unavailable') return reasonText(item.reason)
  return ''
}
function outcomeText(result: CacheResult) {
  if (result.outcome === 'deleted') return t('components.cache.deleted')
  if (result.outcome === 'scheduled') return t('components.cache.scheduled')
  return t('components.cache.failed', { reason: reasonText(result.reason) })
}

async function read(manual = false) {
  if (!props.connected || clearing.value) return
  const epoch = ++readEpoch
  if (manual) manualRead.value = true
  try {
    const next = await manage<CachePreview>(['cache-preview'], manual ? 'manual' : 'background')
    if (epoch !== readEpoch) return
    preview.value = next
    readError.value = ''
  } catch (failure) {
    if (epoch !== readEpoch) return
    // A background reread keeps the last listing; report when the user asked or nothing is shown.
    if (manual || !preview.value) readError.value = t('components.cache.readFailed', { reason: errorText(failure) })
    else console.warn('Background cache listing failed', errorText(failure))
  } finally {
    if (epoch === readEpoch) manualRead.value = false
  }
}

async function openConfirm() {
  clearError.value = ''
  dialog.value?.showModal()
  await nextTick()
  cancelButton.value?.focus()
}
function closeConfirm() {
  if (!clearing.value) dialog.value?.close()
}
async function confirmClear() {
  if (!preview.value || clearing.value || !props.connected) return
  clearing.value = true
  clearError.value = ''
  readEpoch++
  try {
    const outcome = await manage<{ results: CacheResult[]; preview: CachePreview }>(
      ['cache-clear', JSON.stringify({ revision: preview.value.revision })], 'manual')
    results.value = outcome.results
    preview.value = outcome.preview
    dialog.value?.close()
  } catch (failure) {
    clearError.value = rawError(failure).includes('CachePreviewChanged')
      ? t('components.cache.previewChanged') : t('components.cache.clearFailed', { reason: errorText(failure) })
    clearing.value = false
    if (rawError(failure).includes('CachePreviewChanged')) await read()
  } finally {
    clearing.value = false
  }
}

watch(() => props.connected, connected => { if (connected) void read() })
void read()
</script>

<template>
  <section aria-labelledby="cache-heading">
    <h2 id="cache-heading">{{ t('components.cache.heading') }}</h2>
    <p v-if="readError" class="form-error" role="alert">{{ readError }}</p>
    <section v-for="group in groups" :key="group.id" class="cache-group" :aria-labelledby="`cache-${group.id}`">
      <h3 :id="`cache-${group.id}`">{{ t(group.label) }}</h3>
      <p v-if="group.id === 'webView'" class="hint">{{ t('components.cache.webViewHelp') }}</p>
      <p v-if="!preview">--</p>
      <p v-else-if="!group.items.length">{{ t('components.cache.none') }}</p>
      <ul v-else class="cache-items">
        <li v-for="item in group.items" :key="item.path || item.category">
          <span v-if="item.path" class="selectable cache-path" translate="no">{{ path(item.path) }}</span>
          <span class="cache-size">{{ item.path ? size(item.bytes, item.complete) : '--' }}</span>
          <small v-if="stateText(item)" :class="item.state === 'scheduled' && item.reason ? 'form-error' : 'hint'">{{ stateText(item) }}</small>
        </li>
      </ul>
    </section>
    <div class="actions cache-actions">
      <button type="button" :disabled="!connected || !preview || !ready.length || clearing || manualRead" @click="openConfirm">
        {{ t('components.cache.clear') }}
      </button>
      <OperationButton :label="t('action.refresh')" :busy-label="t('action.checking')" :busy="manualRead"
        :disabled="!connected || clearing" @click="read(true)" />
    </div>
    <section v-if="results" class="cache-results" aria-labelledby="cache-results-title" role="status">
      <h3 id="cache-results-title">{{ failures.length ? t('components.cache.resultPartial', { count: failures.length }) : t('components.cache.resultDone') }}</h3>
      <ul v-if="results.length" class="cache-items">
        <li v-for="result in results" :key="result.path">
          <span class="selectable cache-path" translate="no">{{ path(result.path) }}</span>
          <small :class="result.outcome === 'failed' ? 'form-error' : 'hint'">{{ outcomeText(result) }}</small>
        </li>
      </ul>
    </section>

    <dialog ref="dialog" aria-labelledby="cache-confirm-title" @cancel.prevent="closeConfirm" @close="clearError=''">
      <h3 id="cache-confirm-title">{{ t('components.cache.confirmTitle', { size: size(readyBytes, ready.every(item => item.complete)) }) }}</h3>
      <OverlayScroll class="cache-confirm-scroll" :label="t('components.cache.heading')">
      <ul class="cache-items">
        <li v-for="item in ready" :key="item.path">
          <span>{{ t(categories.find(category => category.id === item.category)!.label) }}</span>
          <span class="selectable cache-path" translate="no">{{ path(item.path) }}</span>
          <span class="cache-size">{{ size(item.bytes, item.complete) }}</span>
          <small v-if="item.category === 'webView'" class="hint">{{ t('components.cache.webViewNextStart') }}</small>
        </li>
      </ul>
      </OverlayScroll>
      <p v-if="clearError" class="form-error" role="alert">{{ clearError }}</p>
      <div class="dialog-actions">
        <button ref="cancelButton" type="button" :disabled="clearing" @click="closeConfirm">{{ t('action.cancel') }}</button>
        <OperationButton class="primary" :label="t('components.cache.confirm')" :busy-label="t('action.working')" :busy="clearing"
          :disabled="!connected" @click="confirmClear" />
      </div>
    </dialog>
  </section>
</template>

<style scoped>
.cache-group { margin-block-start: 1rem; }
.cache-group h3 { margin-block-end: .25rem; }
.cache-items { margin: 0; padding: 0; list-style: none; display: grid; gap: .375rem; }
.cache-items li { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .125rem .75rem; align-items: baseline; }
.cache-items li > small, .cache-items li > span:first-child:not(.cache-path) { grid-column: 1 / -1; }
.cache-path { overflow-wrap: anywhere; }
.cache-size { font-variant-numeric: tabular-nums; white-space: nowrap; }
.cache-actions, .cache-results { margin-block-start: 1rem; }
dialog .cache-confirm-scroll { max-block-size: 50vh; }
</style>
