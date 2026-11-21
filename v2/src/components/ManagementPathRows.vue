<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import PathPickerButton from './PathPickerButton.vue'

export interface PathDraft {
  id: number
  path: string
}

const props = defineProps<{
  rows: PathDraft[]
  kind: 'allow' | 'exclude'
  busy: boolean
  issues: Record<number, string>
  context?: 'java'
  unavailable?: string[]
  maxRows?: number
}>()
const emit = defineEmits<{
  add: []
  edit: [number, string]
  picking: [boolean]
  failed: [unknown]
  remove: [number]
}>()
const { t } = useAppLanguage()
const removeDialog = ref<HTMLDialogElement>()
const cancelRemove = ref<HTMLButtonElement>()
const toRemove = ref<PathDraft>()
const prefix = () => props.context === 'java' ? 'java-search' : 'management'

function inputId(id: number) {
  return `${prefix()}-${props.kind}-path-${id}`
}

async function addRow() {
  emit('add')
  await nextTick()
  const row = props.rows.at(-1)
  if (row) document.getElementById(inputId(row.id))?.focus()
}

function confirmRemoval(row: PathDraft) {
  toRemove.value = row
  removeDialog.value?.showModal()
  cancelRemove.value?.focus()
}

async function removeRow() {
  const id = toRemove.value?.id
  const index = props.rows.findIndex(row => row.id === id)
  removeDialog.value?.close()
  if (index < 0 || id === undefined) return
  const nextRow = props.rows[index + 1] ?? props.rows[index - 1]
  emit('remove', id)
  await nextTick()
  document.getElementById(nextRow ? inputId(nextRow.id) : `${prefix()}-${props.kind}-add`)?.focus()
}
</script>

<template>
  <fieldset class="path-rules">
    <legend>{{ t(context === 'java' ? 'management.javaSearchDirectories'
      : (kind === 'allow' ? 'management.allow' : 'management.exclude')) }}</legend>
    <div v-for="(row, index) in rows" :key="row.id" class="path-rule">
      <input :id="inputId(row.id)" :name="`${prefix()}-${kind}-path`" :value="row.path"
        :aria-label="t(context === 'java' ? 'management.javaSearchDirectoryRow'
          : (kind === 'allow' ? 'management.allowRow' : 'management.excludeRow'), { index: index + 1 })"
        :aria-invalid="!!issues[row.id] || undefined"
        :aria-describedby="issues[row.id] || unavailable?.includes(row.path) ? `${inputId(row.id)}-status` : undefined"
        autocomplete="off" spellcheck="false" maxlength="1024" :readonly="busy"
        @input="emit('edit', row.id, ($event.target as HTMLInputElement).value)">
      <PathPickerButton :current="row.path" kind="folder" :disabled="busy"
        @picking="emit('picking', $event)" @failed="emit('failed', $event)" @selected="emit('edit', row.id, $event)" />
      <button type="button" class="remove-path" :disabled="busy" :title="t('management.removePath')"
        :aria-label="t('management.removeRow', { index: index + 1 })" @click="confirmRemoval(row)">
        <!-- Lucide Trash 2, ISC license. See assets/icons/sources.json. -->
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
          <path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2M10 11v6m4-6v6"/>
        </svg>
      </button>
      <small v-if="issues[row.id]" :id="`${inputId(row.id)}-status`" class="form-error">{{ issues[row.id] }}</small>
      <small v-else-if="unavailable?.includes(row.path)" :id="`${inputId(row.id)}-status`" class="hint">{{ t('management.javaScanFolderUnavailable') }}</small>
    </div>
    <button :id="`${prefix()}-${kind}-add`" type="button" :disabled="busy || (maxRows !== undefined && rows.length >= maxRows)" @click="addRow">{{ t('management.addPath') }}</button>
    <p v-if="maxRows !== undefined && rows.length >= maxRows" class="hint">{{ t('management.javaSearchPathLimit', { count: maxRows }) }}</p>
  </fieldset>
  <dialog ref="removeDialog" class="compact" :aria-labelledby="`${prefix()}-${kind}-remove-title`"
    @close="toRemove=undefined" @cancel.prevent="removeDialog?.close()">
    <h2 :id="`${prefix()}-${kind}-remove-title`">{{ t('management.removePath') }}</h2>
    <p class="removed-path selectable">{{ toRemove?.path || t('management.emptyPath') }}</p>
    <p>{{ t(context === 'java' ? 'management.javaSearchRemoveDetail' : 'management.removePathDetail') }}</p>
    <div class="dialog-actions">
      <button ref="cancelRemove" type="button" autofocus @click="removeDialog?.close()">{{ t('action.cancel') }}</button>
      <button class="danger" type="button" @click="removeRow">{{ t('management.removePath') }}</button>
    </div>
  </dialog>
</template>

<style scoped>
.path-rules { min-inline-size: 0; margin: .875rem 0; padding: 0; border: 0; }
.path-rules legend { font-weight: 600; margin-block-end: .375rem; }
.path-rule { display: grid; grid-template-columns: minmax(0, 1fr) auto auto; align-items: stretch; gap: .375rem; margin-block-end: .5rem; }
.path-rule input { min-width: 0; width: 100%; }
.path-rule small { grid-column: 1 / -1; }
.remove-path { display: grid; place-items: center; min-inline-size: 2.375rem; padding-inline: .5rem; color: var(--error); }
.remove-path:hover:not(:disabled), .remove-path:focus-visible { color: var(--error); border-color: var(--error); background: var(--danger-hover); }
.removed-path { overflow-wrap: anywhere; }
</style>
