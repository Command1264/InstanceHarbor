<script setup lang="ts">
import { ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { pickManagementFolder, pickFile, type FilePickerKind } from '../management-folders'

const props = defineProps<{
  current: string
  kind: 'folder' | FilePickerKind
  disabled?: boolean
}>()
const emit = defineEmits<{
  selected: [string]
  failed: [unknown]
  picking: [boolean]
}>()
const { t } = useAppLanguage()
const busy = ref(false)

async function browse() {
  if (busy.value || props.disabled) return
  busy.value = true
  emit('picking', true)
  try {
    const current = props.current.trim()
    const path = await (props.kind === 'folder' ? pickManagementFolder(current) : pickFile(current, props.kind))
    if (path) emit('selected', path)
  } catch (error) {
    emit('failed', error)
  } finally {
    busy.value = false
    emit('picking', false)
  }
}
</script>

<template>
  <button type="button" class="path-picker" :disabled="disabled || busy"
    :aria-label="t(kind === 'folder' ? 'management.pickFolder' : 'management.pickFile')"
    :title="t(kind === 'folder' ? 'management.pickFolder' : 'management.pickFile')" @click="browse">
    <!-- Lucide Folder Open and File Search, ISC license. See assets/icons/sources.json. -->
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
      stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
      <template v-if="kind === 'folder'">
        <path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>
      </template>
      <template v-else>
        <path d="M14 2v4a2 2 0 0 0 2 2h4M4.268 21a2 2 0 0 0 1.727 1H18a2 2 0 0 0 2-2V7l-5-5H6a2 2 0 0 0-2 2v3M9 18l-1.5-1.5"/>
        <circle cx="5" cy="14" r="3"/>
      </template>
    </svg>
  </button>
</template>

<style scoped>
.path-picker { display: inline-flex; align-items: center; justify-content: center; min-inline-size: 2.375rem; min-block-size: 2.25rem; padding-inline: .5rem; }
.path-picker svg { flex: none; }
</style>
