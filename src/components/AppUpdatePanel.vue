<script setup lang="ts">
// The update result rows shared by "App updates" and "About" (#65).
import { computed, onMounted, ref } from 'vue'
import { formatDisplayTime, useAppLanguage, type TimeFormat } from '../i18n'
import { nativeBridge } from '../manager'
import { updateAction, updateActionText, updateAssetText, updateFailureText, updateResultText } from '../app-update'
import {
  checkForUpdateNow, downloadUpdate, installUpdate, openUpdateRelease, readUpdateStatus, revealUpdate,
  updateActionError, updateBusy, updateChecking, updateFeedback, updateStatus,
} from '../app-update-store'
import OperationButton from './OperationButton.vue'

const props = withDefaults(defineProps<{ timeFormat?: TimeFormat, connected?: boolean, showVersion?: boolean }>(), { showVersion: true })
const { t, locale } = useAppLanguage()
const confirmDialog = ref<HTMLDialogElement>()
const cancelButton = ref<HTMLButtonElement>()

onMounted(() => {
  // The rail already read it at start; read again quietly in case it changed.
  if (nativeBridge) readUpdateStatus().catch(error => console.warn('Could not read the update status', error))
})

const result = computed(() => updateResultText(updateStatus.value, t))
const asset = computed(() => updateAssetText(updateStatus.value, t))
const action = computed(() => updateAction(updateStatus.value))
const actionLabel = computed(() => t(action.value === 'install' ? 'components.update.install'
  : action.value === 'reveal' ? 'components.update.reveal' : 'components.update.download'))
const busyLabel = computed(() => t(updateBusy.value === 'download' ? 'components.update.downloading' : 'action.working'))
const lastCheck = computed(() => {
  const at = updateStatus.value?.lastAttempt
  return at ? formatDisplayTime(at * 1000, locale.value, props.timeFormat) : '--'
})
const failure = computed(() => {
  const code = updateStatus.value?.lastError
  return code ? t('components.update.lastFailed', { reason: updateFailureText(code, t) }) : ''
})
const done = ref<'' | 'downloaded' | 'installing' | 'revealed'>('')
const feedback = computed(() => updateChecking.value ? t('components.update.checking')
  : updateBusy.value === 'download' ? t('components.update.downloading')
  : updateActionError.value ? t('components.update.actionFailed', { reason: updateActionText(updateActionError.value, t) })
  : done.value === 'downloaded' ? t('components.update.downloaded')
  : done.value === 'installing' ? t('components.update.installing')
  : done.value === 'revealed' ? t('components.update.revealed')
  : updateFeedback.value === 'done' ? t('components.update.checkDone') : '')

async function run() {
  const update = updateStatus.value?.update
  if (!update || !action.value) return
  done.value = ''
  if (action.value === 'install') {
    confirmDialog.value?.showModal()
    cancelButton.value?.focus()
  } else if (action.value === 'download') {
    if (await downloadUpdate(update.tag)) done.value = 'downloaded'
  } else if (await revealUpdate(update.tag)) done.value = 'revealed'
}

async function confirmInstall() {
  const update = updateStatus.value?.update
  if (!update) return
  if (await installUpdate(update.tag)) {
    done.value = 'installing'
    confirmDialog.value?.close()
  }
}
</script>

<template>
  <div v-if="showVersion" class="field"><span class="update-label">{{ t('components.update.currentVersion') }}</span><span class="update-value">{{ updateStatus?.currentVersion ?? '--' }}</span></div>
  <div class="field"><span class="update-label">{{ t('components.update.latest') }}</span><div class="editor">
    <span class="update-value" data-update-result>{{ result }}</span>
    <small v-if="asset" class="hint">{{ asset }}</small>
    <div class="actions">
      <OperationButton class="primary" data-update-action :label="actionLabel" :busy-label="busyLabel" :busy="!!updateBusy"
        :disabled="!action || !nativeBridge || connected === false || updateChecking" @click="run" />
      <button type="button" :disabled="!updateStatus?.update" @click="updateStatus?.update && openUpdateRelease(updateStatus.update.tag)">{{ t('components.update.openRelease') }}</button>
      <OperationButton :label="t('components.update.checkNow')" :busy-label="t('components.update.checking')" :busy="updateChecking" :disabled="!nativeBridge || connected === false || !!updateBusy" @click="checkForUpdateNow" />
    </div>
    <span class="hint" role="status">{{ feedback }}</span>
  </div></div>
  <div class="field"><span class="update-label">{{ t('components.update.lastCheck') }}</span><div class="editor">
    <span class="update-value">{{ lastCheck }}</span>
    <p v-if="failure" class="form-error" role="alert">{{ failure }}</p>
  </div></div>
  <dialog ref="confirmDialog" class="compact" aria-labelledby="update-install-title" @cancel.prevent="updateBusy || confirmDialog?.close()">
    <h2 id="update-install-title">{{ t('components.update.installTitle', { version: updateStatus?.update?.version ?? '--' }) }}</h2>
    <p>{{ t('components.update.installBody') }}</p>
    <p class="hint">{{ t('components.update.installServers') }}</p>
    <p v-if="updateActionError && updateBusy === ''" class="form-error" role="alert">{{ t('components.update.actionFailed', { reason: updateActionText(updateActionError, t) }) }}</p>
    <div class="dialog-actions">
      <button ref="cancelButton" type="button" :disabled="!!updateBusy" @click="confirmDialog?.close()">{{ t('action.cancel') }}</button>
      <OperationButton class="primary" :label="t('components.update.installConfirm')" :busy-label="t('action.working')" :busy="updateBusy === 'install'" @click="confirmInstall" />
    </div>
  </dialog>
</template>

<style scoped>
.update-label { font-size: .875rem; font-weight: 500; }
.update-value { overflow-wrap: anywhere; }
.editor .form-error { margin: 0; }
</style>
