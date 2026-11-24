<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import OverlayScroll from './OverlayScroll.vue'
import OperationButton from './OperationButton.vue'
import { isMessageId, useAppLanguage } from '../i18n'
import { manage, rawError } from '../manager'
import { displayPath } from '../path-display'
import { pickManagementFolder } from '../management-folders'

interface Candidate {
  directory: string
  name: string
  kind: 'Minecraft' | 'Velocity' | 'Unknown'
  core?: string | null
  port: number | null
  listenAddress: string | null
  state: 'Ready' | 'Blocked'
  reason: string | null
  warning?: string | null
}

interface Preview {
  directory: string
  complete: boolean
  rows: Candidate[]
  policyRevision: string
  previewRevision: string
}

interface SaveResult {
  registration: { rows: { directory: string; name: string; state: 'Imported' | 'Rejected' | 'NeedsConfirmation'; reason: string | null }[] } | null
  registrationError?: string | null
}

const props = defineProps<{ connected: boolean; pathSeparator?: 'forward' | 'native' }>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const closeButton = ref<HTMLButtonElement>()
const folderButton = ref<HTMLButtonElement>()
const errorElement = ref<HTMLElement>()
const directory = ref('')
const preview = ref<Preview>()
const excluded = ref<string[]>([])
const pickerBusy = ref(false)
const pending = ref<'scanning' | 'retrying' | 'saving'>()
const error = ref('')
const result = ref<SaveResult>()
let request = 0

const readyRows = computed(() => preview.value?.rows.filter(row => row.state === 'Ready') ?? [])
const pendingRows = computed(() => preview.value?.rows.filter(row => row.state === 'Blocked' && row.reason !== 'AlreadyManaged') ?? [])
const selectedCount = computed(() => readyRows.value.length - excluded.value.length)
const canSave = computed(() => props.connected && !pending.value && !pickerBusy.value &&
  preview.value?.complete === true && !result.value && !error.value)
const resultRows = computed(() => [
  ...(result.value?.registration?.rows ?? []),
  ...pendingRows.value.map(row => ({ ...row, state: 'NeedsConfirmation' })),
])
const resultCounts = computed(() => {
  const rows = resultRows.value
  const registered = rows.filter(row => row.state === 'Imported').length
  return { registered, pending: rows.length - registered }
})

function path(value: string) {
  return displayPath(value, props.pathSeparator)
}

function coreWarning(value: string) {
  const key = 'core.warning.' + value
  return isMessageId(key) ? t(key) : t('core.warning.CoreUnverified')
}

function reason(value?: string | null) {
  if (!value) return ''
  const local = 'batchImport.reason.' + value
  const shared = 'activity.reason.' + value
  if (isMessageId(local)) return t(local)
  if (isMessageId(shared)) return t(shared)
  return t('batchImport.reason.unknown')
}

async function showError(message: string) {
  error.value = message
  await nextTick()
  errorElement.value?.focus()
}

function openDialog() {
  if (dialog.value?.open) return
  dialog.value?.showModal()
  closeButton.value?.focus()
}

async function chooseFolder() {
  if (!props.connected || pickerBusy.value || pending.value || result.value) return
  const token = ++request
  pickerBusy.value = true
  let chosen: string | null = null
  try {
    chosen = await pickManagementFolder(directory.value)
  } catch {
    if (token === request) {
      openDialog()
      await showError(t('management.pickFailed'))
    }
    return
  } finally {
    pickerBusy.value = false
  }
  if (token !== request || !chosen) return
  directory.value = chosen
  preview.value = undefined
  excluded.value = []
  openDialog()
  await check()
}

async function check(retry = false) {
  if (pending.value || !directory.value || !props.connected) return
  const token = ++request
  const selectedDirectory = directory.value
  if (!retry) error.value = ''
  pending.value = retry ? 'retrying' : 'scanning'
  let restoreFocus = false
  try {
    const { revision } = await manage<{ revision: string }>(['scan-policy'], 'background')
    if (token !== request) return
    const scanned = await manage<Preview>(['management-folder-preview', JSON.stringify({ directory: selectedDirectory, revision })])
    if (token !== request) return
    preview.value = scanned
    excluded.value = excluded.value.filter(exclusion =>
      scanned.rows.some(row => row.state === 'Ready' && row.directory === exclusion && row.directory !== scanned.directory))
    restoreFocus = retry && document.activeElement?.id === 'management-folder-retry'
    error.value = ''
  } catch (failure) {
    if (token !== request) return
    const message = /\b(?:BatchImportDirectoryOutsideScope|ImportOutsideInstanceRoot)\b/.test(rawError(failure))
      ? t('batchImport.reason.BatchImportDirectoryOutsideScope') : t('management.previewFailed')
    if (retry) error.value = message
    else await showError(message)
  } finally {
    if (token === request) pending.value = undefined
  }
  if (restoreFocus && token === request) {
    await nextTick()
    // A successful retry removes its button, so leave focus on the next available action.
    if (token === request) folderButton.value?.focus()
  }
}

function toggle(row: Candidate, checked: boolean) {
  if (!canSave.value || row.state !== 'Ready' || row.directory === preview.value?.directory) return
  excluded.value = checked
    ? excluded.value.filter(directory => directory !== row.directory)
    : [...excluded.value, row.directory]
}

async function save() {
  const scanned = preview.value
  if (!canSave.value || !scanned) return
  const token = ++request
  pending.value = 'saving'
  try {
    result.value = await manage<SaveResult>(['management-folder-save', JSON.stringify({
      directory: scanned.directory,
      revision: scanned.policyRevision,
      previewRevision: scanned.previewRevision,
      excludedDirectories: excluded.value,
    })])
    emit('changed')
  } catch (failure) {
    // The policy update may have succeeded even if the response was lost.
    emit('changed')
    if (token === request) await showError(/PreviewChanged|SettingsConflict/.test(rawError(failure))
      ? t('batchImport.reason.BatchImportPreviewChanged') : t('management.saveFailed'))
  } finally {
    if (token === request) pending.value = undefined
  }
  if (result.value) {
    await nextTick()
    closeButton.value?.focus()
  }
}

function open() {
  if (dialog.value?.open || pickerBusy.value) return
  directory.value = ''
  preview.value = undefined
  result.value = undefined
  error.value = ''
  excluded.value = []
  void chooseFolder()
}

function close() {
  if (pending.value === 'saving' || pickerBusy.value) return
  request++
  pending.value = undefined
  dialog.value?.close()
}

onBeforeUnmount(() => { request++ })
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="management-folder-dialog" aria-labelledby="management-folder-title" @cancel.prevent="close">
    <h2 id="management-folder-title">{{ t('management.add') }}</h2>
    <OverlayScroll class="management-folder-scroll" :label="t('management.paths')">
      
      <dl v-if="directory" class="management-folder-source">
        <dt>{{ t('management.selectedFolder') }}</dt>
        <dd class="hint selectable" translate="no">{{ path(directory) }}</dd>
      </dl>

      <section v-if="preview && !result" class="management-folder-preview" aria-labelledby="management-folder-preview-title">
        <h3 id="management-folder-preview-title">{{ t('management.preview') }}</h3>
        <p class="hint" role="status">{{ t('management.summary', {
          selected: selectedCount, excluded: excluded.length, blocked: pendingRows.length,
        }) }}</p>
        <p v-if="!preview.complete" class="form-error" role="alert">{{ t('batchImport.incomplete') }}</p>
        <p v-else-if="!preview.rows.length" class="hint">{{ t('management.noCandidates') }}</p>
        <ul class="management-folder-list">
          <li v-for="row in preview.rows" :key="row.directory" class="management-folder-row" :data-directory="row.directory">
            <label class="management-folder-row-heading">
              <input type="checkbox" :checked="row.state === 'Ready' && !excluded.includes(row.directory)"
                :disabled="!canSave || row.state !== 'Ready' || row.directory === preview.directory"
                @change="toggle(row, ($event.target as HTMLInputElement).checked)">
              <strong translate="no">{{ row.name }}</strong>
            </label>
            <div class="management-folder-row-content">
              <p class="hint selectable" translate="no">{{ path(row.directory) }}</p>
              <p>
                <span v-if="row.reason === 'AlreadyManaged'" class="badge">{{ t('management.alreadyManaged') }}</span>
                <span v-else-if="row.state === 'Blocked'" class="badge RunUnconfirmed">{{ t('management.blocked') }}</span>
                <span v-else-if="excluded.includes(row.directory)" class="badge">{{ t('management.exclude') }}</span>
                <span class="management-folder-core">{{ row.core || t(`batchImport.${row.kind}`) }}</span>
                <span v-if="row.port !== null" class="management-folder-port">{{ t('batchImport.port', { port: row.port }) }}</span>
                <bdi v-if="row.listenAddress" class="management-folder-port selectable" translate="no">{{ row.listenAddress }}</bdi>
              </p>
              <p v-if="row.reason && row.reason !== 'AlreadyManaged'" class="hint">{{ reason(row.reason) }}</p>
              <p v-if="row.state === 'Ready' && row.warning" class="form-error" role="note">{{ coreWarning(row.warning) }}</p>
              <p v-else-if="row.state === 'Ready' && row.directory === preview.directory" class="hint">{{ t('management.rootLocked') }}</p>
            </div>
          </li>
        </ul>
      </section>

      <section v-if="result" class="management-folder-result" aria-labelledby="management-folder-result-title">
        <h3 id="management-folder-result-title">{{ t('management.result') }}</h3>
        <p role="status">{{ t('management.saveDone') }} · {{ t('management.resultCounts', resultCounts) }}</p>
        <p v-if="result.registrationError" class="form-error" role="alert">{{ t('management.registrationPending') }}</p>
        <ul v-if="resultRows.length" class="management-folder-list">
          <li v-for="row in resultRows" :key="row.directory" class="management-folder-receipt-row" :data-directory="row.directory">
            <strong translate="no">{{ row.name }}</strong>
            <span class="badge" :class="row.state === 'Imported' ? 'Listening' : 'RunUnconfirmed'">{{ t(row.state === 'Imported' ? 'management.registered' : 'management.blocked') }}</span>
            <p class="hint selectable" translate="no">{{ path(row.directory) }}</p>
            <p v-if="row.reason" class="hint">{{ reason(row.reason) }}</p>
          </li>
        </ul>
      </section>
      <p v-if="pending === 'scanning' || pending === 'saving'" class="hint" role="status">{{ t(`management.${pending}`) }}</p>
      <p v-if="error" ref="errorElement" tabindex="-1" class="form-error" role="alert">{{ error }}</p>
    </OverlayScroll>
    <div class="dialog-actions">
      <button ref="closeButton" type="button" :disabled="pending === 'saving' || pickerBusy" @click="close">{{ t(result ? 'batchImport.close' : 'action.cancel') }}</button>
      <template v-if="!result">
        <button ref="folderButton" type="button" :disabled="!connected || !!pending || pickerBusy" @click="chooseFolder">{{ t(directory ? 'management.changeFolder' : 'management.browse') }}</button>
        <OperationButton v-if="error && directory" id="management-folder-retry"
          :label="t('management.reload')" :busy-label="t('management.scanBusy')" :busy="pending === 'retrying'"
          :disabled="!connected || pending === 'saving' || pickerBusy" @click="check(true)" />
        <button v-if="preview" class="primary management-folder-save" type="button" :disabled="!canSave" @click="save">{{ t('action.saveShort') }}</button>
      </template>
    </div>
  </dialog>
</template>

<style scoped>
.management-folder-dialog { width: min(760px, calc(100vw - 36px)); overflow: hidden; }
.management-folder-dialog[open] { display: flex; flex-direction: column; }
.management-folder-scroll { min-height: 0; }
.management-folder-scroll :deep(.overlay-viewport) { padding: calc(4px + var(--focus-ring-extent)) 14px 8px calc(4px + var(--focus-ring-extent)); }
.management-folder-source { margin: 0; }
.management-folder-source dd { margin: 0; overflow-wrap: anywhere; }
label { display: grid; gap: 4px; min-width: 0; font-size: .875rem; }
.management-folder-preview, .management-folder-result { margin-top: 16px; }
h3 { margin: 0; font-size: 1rem; }
.management-folder-list { list-style: none; padding: 0; margin: 8px 0 0; }
.management-folder-row, .management-folder-receipt-row {
  border-top: 1px solid var(--border);
  padding: 10px 0;
  font-size: .875rem;
  min-width: 0;
  overflow-wrap: anywhere;
}
.management-folder-row-heading { grid-template-columns: 1.25rem minmax(0, 1fr); align-items: center; gap: 8px; }
.management-folder-row-heading input { margin: 0 auto; accent-color: var(--primary); }
.management-folder-row-content { margin-inline-start: 1.75rem; }
.management-folder-row p, .management-folder-receipt-row p { margin: 4px 0; }
.management-folder-core, .management-folder-port, .management-folder-receipt-row .badge { margin-inline-start: 8px; }
.dialog-actions { flex: none; flex-wrap: wrap; margin-top: 12px; }
</style>
