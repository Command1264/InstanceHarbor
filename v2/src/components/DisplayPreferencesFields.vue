<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  consoleTitleTokens,
  formatDisplayTime,
  renderConsoleTitle,
  useAppLanguage,
  validateConsoleTitle,
  type TimeFormat,
} from '../i18n'

const props = defineProps<{ showTime?: boolean; titleValues?: Record<string, string>; inheritedTitle?: string }>()
const timeFormat = defineModel<TimeFormat>('timeFormat', { default: 'local24' })
const titleTemplate = defineModel<string>('consoleTitleTemplate', { required: true })
const { t, locale } = useAppLanguage()
const sampleTime = Date.now()
const titleField = ref<HTMLInputElement>()
const touchedTitle = ref(false)
const placeholderKey = ref('instance')
const effectiveTitle = computed(() => !titleTemplate.value.trim() && props.inheritedTitle
  ? props.inheritedTitle : titleTemplate.value)
const preview = computed(() => renderConsoleTitle(effectiveTitle.value,
  props.titleValues ?? { instance: t('appearance.titleSample') }))
const titleError = computed(() => validateConsoleTitle(effectiveTitle.value))

function insertPlaceholder() {
  if (!titleField.value) return
  const field = titleField.value
  const token = '{' + placeholderKey.value + '}'
  const start = field.selectionStart ?? field.value.length
  const end = field.selectionEnd ?? start
  if (field.value.length - (end - start) + token.length > field.maxLength) return
  field.setRangeText(token, start, end, 'end')
  titleTemplate.value = field.value
  field.focus()
}
</script>

<template>
  <div v-if="showTime !== false" class="field">
    <label for="app-time-format">{{ t('appearance.timeFormat') }}</label>
    <div class="editor">
      <select id="app-time-format" v-model="timeFormat" name="time-format" aria-describedby="app-time-preview">
        <option value="local24">{{ t('appearance.local24') }}</option>
        <option value="local12">{{ t('appearance.local12') }}</option>
        <option value="utc">{{ t('appearance.utc') }}</option>
      </select>
      <small id="app-time-preview" class="hint">{{ t('appearance.preview') }} · {{ formatDisplayTime(sampleTime, locale, timeFormat) }}</small>
    </div>
  </div>
  <div class="field">
    <label for="app-console-title">{{ t(inheritedTitle ? 'appearance.instanceConsoleTitle' : 'appearance.consoleTitle') }}</label>
    <div class="editor">
      <input id="app-console-title" ref="titleField" v-model="titleTemplate" name="console-title"
        type="text" maxlength="256" autocomplete="off" :spellcheck="false" translate="no"
        :aria-invalid="touchedTitle && !!titleError"
        :aria-describedby="touchedTitle && titleError ? 'app-console-title-help app-console-title-error' : preview ? 'app-console-title-help app-console-title-preview' : 'app-console-title-help'"
        @blur="touchedTitle=true">
      <small id="app-console-title-help" class="hint">{{ t(inheritedTitle ? 'appearance.inheritedTitleHelp' : 'appearance.consoleTitleHelp') }}</small>
      <small v-if="touchedTitle && titleError" id="app-console-title-error" class="form-error" role="status">
        {{ t(titleError === 'invalid' ? 'appearance.titleLength' : 'appearance.titlePlaceholder') }}
      </small>
      <small v-if="preview" id="app-console-title-preview" class="hint title-preview">
        {{ t('appearance.preview') }} · <bdi translate="no">{{ preview }}</bdi>
      </small>
      <div class="title-insert">
        <label for="app-console-token">{{ t('appearance.titleVariables') }}</label>
        <select id="app-console-token" v-model="placeholderKey" name="console-title-token">
          <option v-for="[key, chinese, english] of consoleTitleTokens" :key="key" :value="key">
            {{ '{' + key + '}' }} · {{ locale === 'en' ? english : chinese }}
          </option>
        </select>
        <button type="button" @click="insertPlaceholder">{{ t('appearance.insertTitleToken') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.title-preview { overflow-wrap: anywhere; }
.title-insert { display: grid; gap: .375rem; min-width: 0; }
.title-insert select { max-width: 100%; }
.title-insert button { justify-self: start; }
</style>
