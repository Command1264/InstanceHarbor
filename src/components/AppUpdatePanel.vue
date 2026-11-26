<script setup lang="ts">
// The update result rows shared by "App updates" and "About" (#65).
import { computed, onMounted } from 'vue'
import { formatDisplayTime, useAppLanguage, type TimeFormat } from '../i18n'
import { nativeBridge } from '../manager'
import { updateAssetText, updateFailureText, updateResultText } from '../app-update'
import {
  checkForUpdateNow, openUpdateRelease, readUpdateStatus, updateActionError, updateChecking, updateFeedback, updateStatus,
} from '../app-update-store'
import OperationButton from './OperationButton.vue'

const props = withDefaults(defineProps<{ timeFormat?: TimeFormat, connected?: boolean, showVersion?: boolean }>(), { showVersion: true })
const { t, locale } = useAppLanguage()

onMounted(() => {
  // The rail already read it at start; read again quietly in case it changed.
  if (nativeBridge) readUpdateStatus().catch(error => console.warn('Could not read the update status', error))
})

const result = computed(() => updateResultText(updateStatus.value, t))
const asset = computed(() => updateAssetText(updateStatus.value, t))
const lastCheck = computed(() => {
  const at = updateStatus.value?.lastAttempt
  return at ? formatDisplayTime(at * 1000, locale.value, props.timeFormat) : '--'
})
const failure = computed(() => {
  const code = updateStatus.value?.lastError
  return code ? t('components.update.lastFailed', { reason: updateFailureText(code, t) }) : ''
})
const feedback = computed(() => updateChecking.value ? t('components.update.checking')
  : updateActionError.value ? t('components.update.actionFailed', { reason: updateActionError.value })
  : updateFeedback.value === 'done' ? t('components.update.checkDone') : '')
</script>

<template>
  <div v-if="showVersion" class="field"><span class="update-label">{{ t('components.update.currentVersion') }}</span><span class="update-value">{{ updateStatus?.currentVersion ?? '--' }}</span></div>
  <div class="field"><span class="update-label">{{ t('components.update.latest') }}</span><div class="editor">
    <span class="update-value" data-update-result>{{ result }}</span>
    <small v-if="asset" class="hint">{{ asset }}</small>
    <div class="actions">
      <button type="button" :disabled="!updateStatus?.update" @click="updateStatus?.update && openUpdateRelease(updateStatus.update.tag)">{{ t('components.update.openRelease') }}</button>
      <OperationButton :label="t('components.update.checkNow')" :busy-label="t('components.update.checking')" :busy="updateChecking" :disabled="!nativeBridge || connected === false" @click="checkForUpdateNow" />
    </div>
    <span class="hint" role="status">{{ feedback }}</span>
  </div></div>
  <div class="field"><span class="update-label">{{ t('components.update.lastCheck') }}</span><div class="editor">
    <span class="update-value">{{ lastCheck }}</span>
    <p v-if="failure" class="form-error" role="alert">{{ failure }}</p>
  </div></div>
</template>

<style scoped>
.update-label { font-size: .875rem; font-weight: 500; }
.update-value { overflow-wrap: anywhere; }
.editor .form-error { margin: 0; }
</style>
