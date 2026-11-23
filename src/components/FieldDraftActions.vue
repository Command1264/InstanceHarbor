<!-- Lucide Rotate CCW and Undo 2, ISC license; see public/licenses/lucide.txt and assets/icons/sources.json. -->
<script setup lang="ts">
import { useAppLanguage } from '../i18n'

withDefaults(defineProps<{
  field: string
  label: string
  canRestoreDefault?: boolean
  canUndo: boolean
}>(), { canRestoreDefault: undefined })
const emit = defineEmits<{ 'restore-default': []; undo: [] }>()
const { t } = useAppLanguage()
</script>

<template>
  <div class="field-draft-actions">
    <button v-if="canRestoreDefault !== undefined" type="button"
      data-field-action="restore-default" :aria-controls="field" :disabled="!canRestoreDefault"
      :aria-label="t('settings.javaRestoreField', { field: label })"
      :title="t('settings.javaRestoreFieldHelp')" @click="emit('restore-default')">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
        <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
        <path d="M3 3v5h5" />
      </svg>
    </button>
    <button type="button" data-field-action="undo-draft" :aria-controls="field" :disabled="!canUndo"
      :aria-label="t('settings.javaUndoField', { field: label })"
      :title="t('settings.javaUndoFieldHelp')" @click="emit('undo')">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
        <path d="M9 14 4 9l5-5" />
        <path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.field-draft-actions {
  display: grid;
  grid-template-columns: repeat(2, 2rem);
  gap: .375rem;
}
.field-draft-actions button {
  display: grid;
  place-items: center;
  min-height: 2.125rem;
  padding: 0;
}
.field-draft-actions [data-field-action="undo-draft"] {
  grid-column: 2;
}
</style>
