<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import OverlayScroll from './OverlayScroll.vue'
import OperationButton from './OperationButton.vue'
import InitialQueryProgress from './InitialQueryProgress.vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage, type JavaCandidate, type RemovedInstance } from '../manager'
import { displayPath, type PathSeparator } from '../path-display'
import { restoreRequestId, restoreJavaChoices } from '../removed-instances'

const props = defineProps<{
  connected: boolean
  javaCandidates: JavaCandidate[]
  javaChecked: boolean
  refreshJava: (origin?: 'manual' | 'background', fresh?: boolean) => Promise<void>
  pathSeparator?: PathSeparator
}>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const closeButton = ref<HTMLButtonElement>()
const errorElement = ref<HTMLElement>()
const rows = ref<RemovedInstance[]>()
const loading = ref(false)
const busy = ref<string>()
const error = ref('')
const message = ref('')
const chosenJava = ref<Record<string, string>>({})
let request = 0

const usableJava = computed(() => props.javaCandidates.filter(item => item.valid).map(item => item.path))

function path(value: string) { return displayPath(value, props.pathSeparator) }
function choices(row: RemovedInstance) { return restoreJavaChoices(row.java, usableJava.value) }

async function load() {
  const token = ++request
  loading.value = true
  error.value = ''
  try {
    const found = await manage<RemovedInstance[]>(['removed-instances'])
    if (token !== request) return
    rows.value = found
    const next: Record<string, string> = {}
    for (const row of found) next[row.name] = chosenJava.value[row.name] ?? choices(row)[0] ?? ''
    chosenJava.value = next
  } catch (e) {
    if (token !== request) return
    error.value = t('instances.removed.error', { error: errorText(e) })
    await nextTick()
    errorElement.value?.focus()
  } finally {
    if (token === request) loading.value = false
  }
}

async function restore(row: RemovedInstance) {
  const java = chosenJava.value[row.name]
  if (!props.connected || busy.value || !java || !row.folderPresent) return
  busy.value = row.name
  error.value = ''
  message.value = ''
  try {
    await manage(['adopt', row.name, restoreRequestId(), java])
    message.value = t('instances.removed.restored', { name: row.name })
    emit('changed')
    await load()
  } catch (e) {
    error.value = t('instances.removed.error', { error: errorText(e) })
    await nextTick()
    errorElement.value?.focus()
  } finally {
    busy.value = undefined
  }
}

async function openFolder(row: RemovedInstance) {
  error.value = ''
  try {
    await manage(['open-server-folder', row.name])
  } catch (e) {
    error.value = t('instances.openServerFolderError', { error: errorText(e) })
  }
}

function open() {
  if (dialog.value?.open) return
  rows.value = undefined
  message.value = ''
  error.value = ''
  dialog.value?.showModal()
  closeButton.value?.focus()
  if (!props.javaChecked) void props.refreshJava('background')
  void load()
}

function close() {
  if (busy.value) return
  request++
  dialog.value?.close()
}

onBeforeUnmount(() => { request++ })
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="removed-instances-dialog" aria-labelledby="removed-instances-title" @cancel.prevent="close">
    <h2 id="removed-instances-title">{{ t('instances.removedInstances') }}</h2>
    <OverlayScroll class="removed-instances-scroll" :label="t('instances.removed.list')">
      <InitialQueryProgress v-if="loading && !rows" :label="t('instances.removed.loading')" />
      <p v-else-if="rows && !rows.length" class="empty">{{ t('instances.removed.empty') }}</p>
      <ul v-else-if="rows" class="removed-instances-list">
        <li v-for="row in rows" :key="row.name" class="removed-instances-row">
          <p><strong>{{ row.name }}</strong> <span class="hint">Port {{ row.port }}</span></p>
          <p v-if="row.path" class="hint selectable" translate="no">{{ path(row.path) }}</p>
          <p v-if="row.managementDiscovered" class="hint">{{ t('instances.removed.discovered') }}</p>
          <label>{{ t('instances.removed.java') }}
            <select v-model="chosenJava[row.name]" :name="`restore-java-${row.name}`" :disabled="!!busy || !row.folderPresent">
              <option value="" disabled>{{ t('instances.removed.javaPlaceholder') }}</option>
              <option v-for="java in choices(row)" :key="java" :value="java">{{ java === row.java ? t('instances.removed.javaSaved', { path: path(java) }) : path(java) }}</option>
            </select>
          </label>
          <p v-if="!row.folderPresent" class="form-error" role="note">{{ t('instances.removed.folderMissing') }}</p>
          <p v-else-if="!choices(row).length && javaChecked" class="form-error" role="note">{{ t('instances.removed.noJava') }}</p>
          <div class="actions">
            <button type="button" :disabled="!connected || !row.folderPresent" @click="openFolder(row)">{{ t('instances.removed.openFolder') }}</button>
            <OperationButton class="primary" :label="t('instances.removed.restore')" :busy-label="t('instances.removed.restoring')"
              :busy="busy === row.name" :disabled="!connected || !row.folderPresent || (!!busy && busy !== row.name) || !chosenJava[row.name]" @click="restore(row)" />
          </div>
        </li>
      </ul>
      <p v-if="message" role="status">{{ message }}</p>
      <p v-if="error" ref="errorElement" tabindex="-1" class="form-error" role="alert">{{ error }}</p>
    </OverlayScroll>
    <div class="dialog-actions">
      <OperationButton :label="t('instances.removed.reload')" :busy-label="t('instances.removed.loading')" :busy="loading" :disabled="!connected || !!busy" @click="load" />
      <button ref="closeButton" type="button" :disabled="!!busy" @click="close">{{ t('action.close') }}</button>
    </div>
  </dialog>
</template>

<style scoped>
.removed-instances-dialog { width: min(640px, calc(100vw - 36px)); overflow: hidden; }
.removed-instances-dialog[open] { display: flex; flex-direction: column; }
.removed-instances-scroll { min-height: 0; }
.removed-instances-scroll :deep(.overlay-viewport) { padding: 4px 14px 8px 4px; }
.removed-instances-list { list-style: none; padding: 0; margin: 8px 0 0; }
.removed-instances-row { border-top: 1px solid var(--border); padding: 10px 0; font-size: .875rem; min-width: 0; overflow-wrap: anywhere; display: grid; gap: 6px; }
.removed-instances-row p { margin: 0; }
label { display: grid; gap: 4px; min-width: 0; font-size: .875rem; }
select { width: 100%; }
.dialog-actions { flex: none; flex-wrap: wrap; margin-top: 12px; }
</style>
