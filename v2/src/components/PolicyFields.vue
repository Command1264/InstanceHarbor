<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useAppLanguage, type MessageId } from '../i18n'
import { isWorldRule, validWorldRuleValue, type PolicyRow, type WorldLiveValue } from '../policy-settings'

const props = defineProps<{
  mode: 'initialization' | 'world'
  rows: PolicyRow[]
  values: Record<string, string>
  stopped?: boolean
  liveValues?: Record<string, WorldLiveValue>
}>()
const emit = defineEmits<{
  (event: 'change', key: string, value: string | null): void
}>()
const { t } = useAppLanguage()
const category = ref('伺服器')
const query = ref('')
const categories = ['伺服器', 'Spigot／Bukkit', 'Paper', '世界', '遊戲規則']
const categoryLabels: Record<string, MessageId> = {
  '伺服器': 'components.policy.category.server', 'Spigot／Bukkit': 'components.policy.category.spigot', Paper: 'components.policy.category.paper',
  '世界': 'components.policy.category.world', '遊戲規則': 'components.policy.category.gamerules'
}
const rows = computed(() => props.rows.filter(row => props.mode !== 'world' || row.key !== 'initialTime'))
const availableCategories = computed(() => categories.filter(item => rows.value.some(row => row.category === item)))
watch(availableCategories, choices => {
  if (!choices.includes(category.value)) category.value = choices[0] ?? '伺服器'
})
const visibleRows = computed(() => {
  const search = query.value.trim().toLowerCase()
  return rows.value.filter(row => search
    ? [fieldLabel(row), row.key, row.source, row.category].some(text => text.toLowerCase().includes(search))
    : row.category === category.value)
})
const choices: Record<string, MessageId> = {
  true: 'components.policy.value.true', false: 'components.policy.value.false', survival: 'components.policy.value.survival', creative: 'components.policy.value.creative', adventure: 'components.policy.value.adventure',
  spectator: 'components.policy.value.spectator', peaceful: 'components.policy.value.peaceful', easy: 'components.policy.value.easy', normal: 'components.policy.value.normal', hard: 'components.policy.value.hard',
  Flat: 'components.policy.value.Flat', Normal: 'components.policy.value.Normal'
}
function valueLabel(value: string | null | undefined) {
  return value == null ? t('components.policy.value.unchecked') : choices[value] ? t(choices[value]) : (value || t('components.policy.value.empty'))
}
function fieldLabel(row: PolicyRow) {
  return props.mode === 'initialization' && row.key === 'initialTime' ? t('components.policy.initialTime') : row.label
}
function editable(row: PolicyRow) {
  if (liveRow(row)) return props.liveValues?.[row.key] !== undefined && row.status !== 'unsupported'
  return props.mode === 'initialization' ? row.status === 'available' : !!row.editable
}
function liveRow(row: PolicyRow) {
  return props.mode === 'world' && isWorldRule(row)
}
function liveLabel(row: PolicyRow) {
  const observation = props.liveValues?.[row.key]
  if (!observation) return row.status === 'unsupported' ? row.reason || t('components.policy.unsupportedVersion') : t('components.policy.notChecked')
  const value = valueLabel(observation.target ?? observation.value)
  switch (observation.state) {
    case 'Queued': return t('components.policy.live.queued', { value })
    case 'Applying': return t('components.policy.live.applying', { value })
    case 'AwaitingNormalStop': return t('components.policy.live.awaitingNormalStop', { value })
    case 'Unconfirmed': return t('components.policy.live.unconfirmed')
    case 'LastSaved': return t('components.policy.live.lastSaved', { value })
    case 'Saved': return t('components.policy.live.saved', { value })
    case 'NoChange': return t('components.policy.live.noChange', { value })
    default: return t('components.policy.live.unchecked')
  }
}
function inputValue(row: PolicyRow) {
  const live = props.liveValues?.[row.key]
  return props.values[row.key] ?? (liveRow(row) ? live?.target ?? live?.value ?? '' : '')
}
function options(row: PolicyRow) {
  return liveRow(row) && row.kind === 'boolean' ? ['true', 'false'] : row.choices
}
function invalidRuleDraft(row: PolicyRow) {
  return liveRow(row) && Object.hasOwn(props.values, row.key) && !validWorldRuleValue(row, props.values[row.key])
}
function fieldDescription(row: PolicyRow) {
  const id = `${props.mode}-${row.key}`
  const descriptions = []
  if (row.source) descriptions.push(id + '-source')
  if (liveRow(row)) descriptions.push(id + '-result')
  if (invalidRuleDraft(row)) descriptions.push(id + '-error')
  return descriptions.length ? descriptions.join(' ') : undefined
}
function setOverride(row: PolicyRow, selected: boolean) {
  emit('change', row.key, selected ? row.inherited ?? row.choices[0] ?? String(row.min ?? '') : null)
}
function update(row: PolicyRow, event: Event) {
  emit('change', row.key, (event.target as HTMLInputElement | HTMLSelectElement).value)
}
function scrollNumber(event: WheelEvent) {
  if (document.activeElement !== event.target) return
  event.preventDefault()
  const element = event.target as HTMLElement
  const scroll = element.closest<HTMLElement>('.overlay-viewport')
  if (scroll) scroll.scrollTop += event.deltaY
}
</script>

<template>
  <div class="field">
    <label :for="`${mode}-search`">{{ t('components.policy.search') }}</label>
    <input :id="`${mode}-search`" :name="`${mode}-search`" v-model="query" type="search" autocomplete="off" :placeholder="t('components.policy.searchPlaceholder')">
  </div>
  <div class="actions" role="group" :aria-label="t('components.policy.categories')" v-if="!query.trim()">
    <button v-for="name in availableCategories" :key="name" type="button" :aria-pressed="category === name" @click="category = name">{{ categoryLabels[name] ? t(categoryLabels[name]) : name }}</button>
  </div>
  <p v-if="!visibleRows.length" class="hint">{{ t('components.policy.noMatch') }}</p>
  <div v-for="row in visibleRows" :key="row.key" class="field">
    <label :for="`${mode}-${row.key}`">{{ fieldLabel(row) }}</label>
    <div class="editor">
      <template v-if="mode === 'initialization' && row.status === 'available'">
        <label>
          <input type="checkbox" :checked="row.key in values" @change="setOverride(row, ($event.target as HTMLInputElement).checked)">
          {{ t('components.policy.override') }}
        </label>
        <small v-if="!(row.key in values)" class="hint">
          {{ row.inherited == null ? t('components.policy.coreDefault') : t('components.policy.currentDefault', { value: valueLabel(row.inherited) }) }}
        </small>
      </template>
      <template v-if="mode === 'world' && !editable(row)">
        <output :id="`${mode}-${row.key}`">{{ valueLabel(row.value) }}</output>
      </template>
      <template v-else-if="editable(row) && (mode === 'world' || row.key in values)">
        <select v-if="options(row).length" :id="`${mode}-${row.key}`" :name="`${mode}-${row.key}`" :value="inputValue(row)"
          :aria-describedby="fieldDescription(row)" @change="update(row, $event)">
          <option v-if="liveRow(row)" value="" disabled>{{ t('components.policy.chooseState') }}</option>
          <option v-for="choice in options(row)" :key="choice" :value="choice">{{ valueLabel(choice) }}</option>
        </select>
        <input v-else :id="`${mode}-${row.key}`" :name="`${mode}-${row.key}`" :value="inputValue(row)" autocomplete="off"
          :aria-describedby="fieldDescription(row)" :aria-invalid="invalidRuleDraft(row) || undefined"
          :type="row.kind === 'integer' ? 'number' : 'text'" :min="row.min ?? undefined"
          :max="row.kind === 'integer' ? row.max ?? undefined : undefined"
          :maxlength="row.kind === 'text' ? row.max ?? undefined : undefined"
          @input="update(row, $event)" @wheel="row.kind === 'integer' && scrollNumber($event)">
      </template>
      <small v-if="row.source" :id="`${mode}-${row.key}-source`" class="hint">{{ row.source }}</small>
      <small v-if="liveRow(row)" :id="`${mode}-${row.key}-result`" class="hint">{{ liveLabel(row) }}</small>
      <small v-if="invalidRuleDraft(row)" :id="`${mode}-${row.key}-error`" class="form-error" role="alert">
        {{ t('components.policy.integerRange', { min: row.min ?? 0, max: row.max ?? 32767 }) }}
      </small>
      <small v-if="!liveRow(row) && row.status !== 'available'" class="hint">{{ row.status === 'unsupported' ? t('components.policy.unsupportedReason', { reason: row.reason ?? '' }) : t('components.policy.notCheckedReason', { reason: row.reason ?? '' }) }}</small>
      <small v-else-if="!liveRow(row) && mode === 'world' && values[row.key] !== row.value" class="hint">
        {{ valueLabel(row.value) }} → {{ valueLabel(values[row.key]) }}
      </small>
    </div>
  </div>
</template>
