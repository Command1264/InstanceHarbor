<script setup lang="ts">
import { computed, onUnmounted, reactive, ref, watch } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage, rawError } from '../manager'
import ManagementPathRows, { type PathDraft } from './ManagementPathRows.vue'
import OperationButton from './OperationButton.vue'

const sourceNames = {
  windows: 'management.javaSourceWindows',
  mise: 'management.javaSourceMise',
  javaHome: 'management.javaSourceHome',
  path: 'management.javaSourcePath',
} as const
type Source = keyof typeof sourceNames
type Sources = Record<Source, boolean>
type Scope = {
  sources: Sources
  directories: string[]
}
type Snapshot = Scope & { revision: string; unavailableDirectories: string[] }
type Validation = { directoryErrors: (string | null)[] }
const defaultSources: Sources = { windows: true, mise: true, javaHome: true, path: true }
const maxRows = 16
const props = defineProps<{
  connected: boolean
  disabled: boolean
  pathLabel: (value: string) => string
}>()
const emit = defineEmits<{ dirty: [boolean]; results: [] }>()
const { t } = useAppLanguage()
const panel = ref<HTMLElement>()
const saved = ref<Snapshot>()
const sources = reactive<Sources>({ ...defaultSources })
const rows = ref<PathDraft[]>([])
const issues = ref<Record<number, string>>({})
const reading = ref(false)
const picking = ref(false)
const error = ref('')
const conflict = ref(false)
const scope = computed<Scope>(() => ({
  sources: { ...sources },
  directories: rows.value.map(row => row.path.trim()),
}))
const dirty = computed(() => {
  const previous = saved.value
  if (!previous) return false
  return (Object.keys(sourceNames) as Source[]).some(source => sources[source] !== previous.sources[source])
    || JSON.stringify(scope.value.directories.map(props.pathLabel)) !== JSON.stringify(previous.directories.map(props.pathLabel))
})
const busy = computed(() => props.disabled || !props.connected || picking.value || reading.value)
const canSave = computed(() => !busy.value && !!saved.value)
const previewInput = computed(() => dirty.value ? JSON.stringify(scope.value) : undefined)
const unavailable = computed(() => saved.value?.unavailableDirectories.map(props.pathLabel) ?? [])
let nextRowId = 0
let loadEpoch = 0
let validationEpoch = 0
let validationTimer: ReturnType<typeof setTimeout> | undefined

function setDraft(value: Scope) {
  Object.assign(sources, value.sources)
  rows.value = value.directories.map(path => ({ id: nextRowId++, path: props.pathLabel(path) }))
  issues.value = {}
}

function restore() {
  if (saved.value) setDraft(saved.value)
  if (!conflict.value) error.value = ''
}

function restoreDefaults() {
  if (busy.value) return
  setDraft({ sources: defaultSources, directories: [] })
}

function add() {
  if (rows.value.length < maxRows) rows.value.push({ id: nextRowId++, path: '' })
}

function edit(id: number, path: string) {
  const row = rows.value.find(item => item.id === id)
  if (row) row.path = path
  delete issues.value[id]
}

function remove(id: number) {
  rows.value = rows.value.filter(row => row.id !== id)
  delete issues.value[id]
}

function pickerFailed(failure: unknown) {
  error.value = errorText(failure)
}

async function load(discardDraft = false, origin: 'manual' | 'background' = 'background') {
  if (!props.connected || props.disabled || reading.value || (!discardDraft && dirty.value)) return
  const epoch = ++loadEpoch
  if (origin === 'manual') reading.value = true
  try {
    const snapshot = await manage<Snapshot>(['java-discovery-settings'], origin)
    if (epoch !== loadEpoch || (!discardDraft && dirty.value)) return
    const replaceDraft = discardDraft || saved.value?.revision !== snapshot.revision
    saved.value = snapshot
    if (replaceDraft) setDraft(snapshot)
    error.value = ''
    conflict.value = false
  } catch (failure) {
    if (epoch === loadEpoch && (!saved.value || origin === 'manual')) error.value = errorText(failure)
  } finally {
    if (epoch === loadEpoch) reading.value = false
  }
}

function payload() {
  return JSON.stringify({ revision: saved.value?.revision, ...scope.value })
}

function setIssues(result: Validation) {
  const next: Record<number, string> = {}
  for (const [index, code] of result.directoryErrors.entries()) {
    const row = rows.value[index]
    if (code && row)
      next[row.id] = t(row.path.trim() ? 'management.javaSearchPathInvalid' : 'management.pathRequired')
  }
  issues.value = next
  return Object.keys(next).length > 0
}

function cancelValidation() {
  clearTimeout(validationTimer)
  return ++validationEpoch
}

function queueValidation() {
  const epoch = cancelValidation()
  if (!props.connected || !dirty.value || props.disabled || conflict.value) return
  const input = payload()
  validationTimer = setTimeout(async () => {
    try {
      const result = await manage<Validation>(['validate-java-discovery-settings', input], 'background')
      if (epoch === validationEpoch && payload() === input) setIssues(result)
    } catch (failure) {
      if (epoch === validationEpoch) console.warn('Java search scope validation failed', errorText(failure))
    }
  }, 250)
}

async function prepareSave() {
  if (!dirty.value) return undefined
  if (conflict.value) throw new Error(t('management.javaSearchConflict'))
  if (!props.connected || picking.value || reading.value || !saved.value)
    throw new Error(t('management.javaSearchReviewRows'))
  loadEpoch++
  cancelValidation()
  error.value = ''
  const input = payload()
  const validation = await manage<Validation>(['validate-java-discovery-settings', input])
  if (setIssues(validation))
    throw new Error(t('management.javaSearchReviewRows'))
  return input
}

function focusIssue() {
  panel.value?.querySelector<HTMLElement>('[aria-invalid=true]')?.focus()
}

async function persist(input: string) {
  try {
    const snapshot = await manage<Snapshot>(['save-java-discovery-settings', input])
    saved.value = snapshot
    setDraft(snapshot)
  } catch (failure) {
    conflict.value = rawError(failure).includes('JavaScanDirectoryConflict')
    if (conflict.value) error.value = t('management.javaSearchConflict')
    throw failure
  }
}

watch(dirty, value => emit('dirty', value))
watch(scope, queueValidation, { flush: 'sync' })
watch(() => props.connected, connected => {
  if (connected) void load()
  else {
    loadEpoch++
    reading.value = false
    cancelValidation()
  }
}, { immediate: true })
onUnmounted(() => {
  loadEpoch++
  cancelValidation()
})
defineExpose({ canSave, previewInput, restore, prepareSave, persist, focusIssue })
</script>

<template>
  <section ref="panel" aria-labelledby="java-search-heading" class="java-search-scope">
    <div class="java-search-heading">
      <h3 id="java-search-heading">{{ t('management.javaSearchScope') }}</h3>
      <button type="button" :disabled="busy || !saved" @click="emit('results')">{{ t('management.javaSearchResults') }}</button>
    </div>
    <p class="hint">{{ t('management.javaSearchHelp') }}</p>
    <template v-if="saved">
      <fieldset class="java-search-sources" :disabled="busy">
        <legend>{{ t('management.javaSearchSources') }}</legend>
        <div class="java-source-options">
          <label v-for="(name, source) in sourceNames" :key="source" class="check">
            <input v-model="sources[source]" type="checkbox">
            {{ t(name) }}
          </label>
        </div>
      </fieldset>
      <ManagementPathRows kind="allow" :rows="rows"
        context="java" :max-rows="maxRows" :busy="busy" :issues="issues" :unavailable="unavailable"
        @add="add" @edit="edit" @remove="remove"
        @picking="picking=$event" @failed="pickerFailed" />
      <div class="actions">
        <button type="button" :disabled="busy" @click="restoreDefaults">{{ t('management.javaSearchDefaults') }}</button>
      </div>
    </template>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <OperationButton v-if="conflict || (!saved && error)"
      :label="t(conflict ? 'management.javaSearchDiscardReload' : 'management.reload')" :busy-label="t('activity.loading')"
      :busy="reading" :disabled="disabled || !connected || picking" @click="load(conflict, 'manual')" />
  </section>
</template>

<style scoped>
.java-search-scope { margin-block-start: 1rem; }
.java-search-heading { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: .5rem 1rem; }
.java-search-scope h3 { margin-block: 0; }
.java-search-sources { min-inline-size: 0; margin: .875rem 0; padding: 0; border: 0; }
.java-search-sources legend { font-weight: 600; margin-block-end: .375rem; }
.java-source-options { display: flex; flex-wrap: wrap; gap: .5rem 1rem; }
.java-source-options label { margin: 0; }
</style>
