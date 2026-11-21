<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { computed, nextTick, ref, watch } from 'vue'
import { useAppLanguage, type MessageId } from '../i18n'
import { manage, rawError, type Instance } from '../manager'
import PathPickerButton from './PathPickerButton.vue'
import OperationButton from './OperationButton.vue'

type Key = { generation: number; publicKey: string }
type Status = { key: Key | null }
type Operation = 'rotate' | 'export' | 'inspect' | 'restore'
type BackupResult = { key?: Key; backup?: string }
type RecoveryInstance = Instance & { recoveryPinned?: boolean }
type RecoveryTarget = RecoveryInstance & { run: NonNullable<Instance['run']>; recoveryPinned: true }
type RecoveryResult = { state: string; run?: { id: string } }

const props = defineProps<{ connected: boolean }>()
const { t } = useAppLanguage()
const key = ref<Key | null>()
const dialog = ref<HTMLDialogElement>()
const passwordField = ref<HTMLInputElement>()
const mode = ref<Operation>()
const path = ref('')
const inspection = ref<{ path: string; key: Key }>()
const reading = ref(false)
const manualRead = ref(false)
const working = ref(false)
const notice = ref<MessageId>()
const statusError = ref<MessageId>()
const operationError = ref<MessageId>()
const exportedPath = ref('')
const targets = ref<RecoveryTarget[]>([])
const selectedName = ref('')
const targetReading = ref(false)
const targetsObserved = ref(false)
const manualTargetRead = ref(false)
const targetError = ref<MessageId>()
const recoverDialog = ref<HTMLDialogElement>()
const recoverCancel = ref<HTMLButtonElement>()
const pendingTarget = ref<{ name: string; runId: string }>()
const recoverBusy = ref(false)
const recoverError = ref<MessageId>()
const recoverNotice = ref<MessageId>()
const recoverNeedsRefresh = ref(false)
let targetReadEpoch = 0
let keyReadEpoch = 0

const selectedTarget = computed(() => targets.value.find(row => row.name === selectedName.value))
function isRecoveryTarget(row: RecoveryInstance): row is RecoveryTarget {
  return row.recoveryPinned === true && !!row.run?.id && ['Listening', 'Starting'].includes(row.state)
}

const choices: Record<Operation, MessageId> = {
  rotate: 'recovery.rotate',
  export: 'recovery.export',
  inspect: 'recovery.inspect',
  restore: 'recovery.restore',
}
const actionLabel = computed(() => {
  if (mode.value === 'rotate' && !key.value) return 'recovery.create'
  return mode.value ? choices[mode.value] : 'recovery.heading'
})
const restoreWouldReplace = computed(() => {
  const current = key.value
  const backup = inspection.value?.key
  return !!current && !!backup
    && (current.generation !== backup.generation || current.publicKey !== backup.publicKey)
})

function failureLabel(failure: unknown): MessageId {
  const value = rawError(failure)
  if (/RecoveryBackupInvalidPassword|RecoveryPasswordInvalidLength/.test(value)) return 'recovery.passwordInvalid'
  if (value.includes('RecoveryBackupWorkspaceMismatch')) return 'recovery.workspaceMismatch'
  if (value.includes('RecoveryKeyGenerationChanged')) return 'recovery.keyChanged'
  if (value.includes('RecoveryHostActive')) return 'recovery.hostActive'
  if (value.includes('RecoveryBackupIdentityChanged')) return 'recovery.backupChanged'
  if (/RecoveryBackupAuthenticationFailed|RecoveryBackupInvalidHeader|InvalidRecoveryBackupSize/.test(value)) {
    return 'recovery.authenticationFailed'
  }
  if (value.includes('create-recovery-backup') && /os error (80|183)\b/.test(value)) {
    return 'recovery.destinationExists'
  }
  if (/InvalidRecoveryBackupPath|RecoveryBackupPathRedirected|PathRedirected/.test(value)) {
    return 'recovery.invalidPath'
  }
  return 'recovery.operationFailed'
}

async function readKey(manual = false) {
  if (reading.value || !props.connected) return
  const epoch = ++keyReadEpoch
  reading.value = true
  manualRead.value = manual
  try {
    const status = await manage<Status>(['recovery-key-status'], manual ? 'manual' : 'background')
    if (epoch !== keyReadEpoch || !props.connected) return
    key.value = status.key
    statusError.value = undefined
  } catch {
    if (epoch === keyReadEpoch && props.connected) statusError.value = 'recovery.readFailed'
  } finally {
    if (epoch === keyReadEpoch) {
      reading.value = false
      manualRead.value = false
    }
  }
}

async function readTargets(manual = false) {
  if (!props.connected || (targetReading.value && (!manual || manualTargetRead.value))) return
  const epoch = ++targetReadEpoch
  targetReading.value = true
  manualTargetRead.value = manual
  try {
    const rows = await manage<RecoveryInstance[]>(['list'], manual ? 'manual' : 'background')
    if (epoch !== targetReadEpoch || !props.connected) return
    const selectedRun = selectedTarget.value?.run.id
    targets.value = rows.filter(isRecoveryTarget)
    targetsObserved.value = true
    if (!targets.value.some(row => row.name === selectedName.value && row.run.id === selectedRun)) selectedName.value = ''
    targetError.value = undefined
    if (manual) recoverNeedsRefresh.value = false
  } catch {
    if (epoch === targetReadEpoch && props.connected) targetError.value = 'recovery.targetsReadFailed'
  } finally {
    if (epoch === targetReadEpoch) {
      targetReading.value = false
      manualTargetRead.value = false
    }
  }
}

watch(() => props.connected, connected => {
  if (connected) {
    void readKey()
    void readTargets()
  } else {
    keyReadEpoch++
    reading.value = false
    manualRead.value = false
    targetReadEpoch++
    targetReading.value = false
    manualTargetRead.value = false
    targets.value = []
    targetsObserved.value = false
    selectedName.value = ''
  }
}, { immediate: true })

function openRecover() {
  const target = selectedTarget.value
  if (!props.connected || !target || recoverBusy.value || manualTargetRead.value || targetError.value || recoverNeedsRefresh.value) return
  pendingTarget.value = { name: target.name, runId: target.run.id }
  recoverError.value = undefined
  recoverNotice.value = undefined
  recoverDialog.value?.showModal()
  void nextTick(() => recoverCancel.value?.focus())
}

function closeRecover() {
  if (!recoverBusy.value) recoverDialog.value?.close()
}

function recoverFailureLabel(failure: unknown): MessageId {
  const value = rawError(failure)
  if (value.includes('RecoveryManagerActive')) return 'recovery.managerActive'
  if (value.includes('RecoveryChallengeExpired')) return 'recovery.challengeExpired'
  if (/RunMismatch|RecoveryRunChanged|HostUnconfirmed|InstanceRemoved/.test(value)) return 'recovery.runChanged'
  if (/RecoveryCandidateUnconfirmed|RecoveryCandidateImageUnavailable|PeerMismatch|RecoveryChallengeUnconfirmed|HostManagerRequired/.test(value)) return 'recovery.candidateUnconfirmed'
  if (/RecoveryKeyGenerationChanged|RecoveryKeyNotFound|RecoveryProofRejected/.test(value)) return 'recovery.keyChanged'
  if (/RecoveryChallengePending|RecoveryBusy/.test(value)) return 'recovery.pending'
  if (/ResultUnconfirmed|HostRecoveryUnconfirmed|RecoveryChallengeUnknown|ManagerPipeUnavailable|ManagerUnavailable|HostPipeUnavailable|HostUnavailable/.test(value)) return 'recovery.resultUnconfirmed'
  return 'recovery.recoverFailed'
}

async function confirmRecover() {
  const target = pendingTarget.value
  if (!target || !props.connected || recoverBusy.value || recoverNeedsRefresh.value) return
  recoverBusy.value = true
  recoverError.value = undefined
  try {
    // Read back the exact current run before sending the one allowed takeover request.
    const rows = await manage<RecoveryInstance[]>(['list'], 'manual')
    if (!rows.some(row => isRecoveryTarget(row) && row.name === target.name && row.run.id === target.runId)) {
      throw new Error('RecoveryRunChanged')
    }
    const result = await manage<RecoveryResult>(['host-recover', target.name, target.runId])
    if (result.state !== 'Recovered' || result.run?.id !== target.runId) {
      throw new Error('RecoveryResultUnconfirmed')
    }
    recoverNotice.value = 'recovery.recovered'
    recoverDialog.value?.close()
    void readTargets()
  } catch (failure) {
    recoverError.value = recoverFailureLabel(failure)
    recoverNeedsRefresh.value = true
    await nextTick()
    recoverCancel.value?.focus()
  } finally {
    recoverBusy.value = false
  }
}

function clearPassword() {
  if (passwordField.value) passwordField.value.value = ''
}

function open(operation: 'rotate' | 'export' | 'inspect') {
  if (!props.connected || working.value || reading.value) return
  inspection.value = undefined
  operationError.value = undefined
  notice.value = undefined
  exportedPath.value = ''
  path.value = ''
  clearPassword()
  mode.value = operation
  dialog.value?.showModal()
}

function close() {
  if (working.value) return
  clearPassword()
  dialog.value?.close()
}

function resetDialog() {
  clearPassword()
  mode.value = undefined
  operationError.value = undefined
  inspection.value = undefined
}

function backupRequest(action: 'export' | 'inspect' | 'restore', backupPath: string, expected?: Key) {
  const field = passwordField.value
  if (!field?.value) return Promise.reject(new Error('RecoveryBackupInvalidPassword'))
  const result = invoke<BackupResult>('recovery_backup', {
    action,
    path: backupPath,
    password: field.value,
    generation: expected?.generation,
    publicKey: action === 'restore' ? expected?.publicKey : undefined,
  })
  clearPassword()
  return result
}

async function submit() {
  const operation = mode.value
  if (!operation || !props.connected || working.value || restoreWouldReplace.value) return
  working.value = true
  operationError.value = undefined
  try {
    if (operation === 'rotate') {
      const previous = key.value
      const rotated = await manage<Status>(['recovery-key-rotate', previous ? String(previous.generation) : 'none'])
      key.value = rotated.key
      notice.value = previous ? 'recovery.rotated' : 'recovery.created'
    } else if (operation === 'export' && key.value) {
      const exported = await backupRequest('export', path.value, key.value)
      exportedPath.value = exported.backup ?? path.value
      notice.value = 'recovery.exported'
    } else if (operation === 'inspect') {
      const checkedPath = path.value
      const checked = await backupRequest('inspect', checkedPath)
      if (!checked.key) throw new Error('RecoveryBackupInvalidHeader')
      inspection.value = { path: checkedPath, key: checked.key }
      mode.value = 'restore'
      return
    } else if (operation === 'restore' && inspection.value) {
      const expected = inspection.value
      const restored = await backupRequest('restore', expected.path, expected.key)
      if (restored.key?.generation !== expected.key.generation || restored.key.publicKey !== expected.key.publicKey) {
        throw new Error('RecoveryBackupIdentityChanged')
      }
      key.value = restored.key
      notice.value = 'recovery.restored'
    } else {
      return
    }
    dialog.value?.close()
  } catch (failure) {
    operationError.value = failureLabel(failure)
    if (operation === 'restore' && operationError.value === 'recovery.backupChanged') {
      mode.value = 'inspect'
      inspection.value = undefined
    }
  } finally {
    clearPassword()
    working.value = false
    if (operation !== 'rotate' && dialog.value?.open) {
      await nextTick()
      passwordField.value?.focus()
    }
  }
}
</script>

<template>
  <section aria-labelledby="recovery-heading">
    <h2 id="recovery-heading">{{ t('recovery.heading') }}</h2>
    <p>{{ t('recovery.scope') }}</p>
    <p v-if="!connected" class="hint">{{ t('recovery.noConnection') }}</p>
    <p v-else-if="key === null">{{ t('recovery.none') }}</p>
    <template v-else-if="key">
      <p>{{ t('recovery.generation') }} · {{ key.generation }}</p>
      <details>
        <summary>{{ t('recovery.details') }}</summary>
        <p>{{ t('recovery.publicKey') }} <code class="selectable" translate="no">{{ key.publicKey }}</code></p>
      </details>
    </template>
    <p v-if="connected && statusError" class="form-error" role="alert">{{ t(statusError) }}</p>
    <div class="actions recovery-actions">
      <button type="button" :disabled="!connected || reading || key === undefined || !!statusError"
        @click="open('rotate')">{{ t(key ? 'recovery.rotate' : 'recovery.create') }}</button>
      <button type="button" :disabled="!connected || reading || !key || !!statusError"
        @click="open('export')">{{ t('recovery.export') }}</button>
      <button type="button" :disabled="!connected || reading"
        @click="open('inspect')">{{ t('recovery.inspect') }}</button>
      <OperationButton :label="t('action.refresh')" :busy-label="t('action.checking')" :busy="manualRead"
        :disabled="!connected || (reading && !manualRead)" @click="readKey(true)" />
    </div>
    <p v-if="notice" role="status">{{ t(notice) }} <code v-if="exportedPath" translate="no">{{ exportedPath }}</code></p>

    <section class="recovery-targets" aria-labelledby="recovery-targets-title">
      <h3 id="recovery-targets-title">{{ t('recovery.targetsTitle') }}</h3>
      <p class="hint">{{ t('recovery.targetsHelp') }}</p>
      <p v-if="targetError" class="form-error" role="alert">{{ t(targetError) }}</p>
      <p v-else-if="connected && targetsObserved && !targets.length">{{ t('recovery.targetsEmpty') }}</p>
      <label v-if="targets.length" for="recovery-target-select">
        {{ t('recovery.target') }}
        <select id="recovery-target-select" v-model="selectedName" name="recovery-target"
          :disabled="!connected || manualTargetRead || recoverBusy" autocomplete="off">
          <option value="">{{ t('recovery.chooseTarget') }}</option>
          <option v-for="row in targets" :key="row.name" :value="row.name">{{ row.name }}</option>
        </select>
      </label>
      <p v-if="selectedTarget" class="recovery-run">{{ t('recovery.runId') }} <code translate="no">{{ selectedTarget.run.id }}</code></p>
      <div class="actions recovery-actions">
        <OperationButton :label="t('recovery.refreshTargets')" :busy-label="t('action.checking')"
          :busy="manualTargetRead" :disabled="!connected" @click="readTargets(true)" />
        <button type="button" :disabled="!connected || !selectedTarget || manualTargetRead || !!targetError || recoverBusy || recoverNeedsRefresh"
          @click="openRecover">{{ t('recovery.openRecover') }}</button>
      </div>
      <p v-if="recoverNotice" role="status">{{ t(recoverNotice) }}</p>
    </section>

    <dialog ref="dialog" aria-labelledby="recovery-dialog-title" @cancel.prevent="close" @close="resetDialog">
      <h3 id="recovery-dialog-title">{{ t(actionLabel) }}</h3>
      <form autocomplete="off" @submit.prevent="submit">
        <p v-if="mode === 'rotate'">{{ t(key ? 'recovery.rotateConfirm' : 'recovery.createConfirm') }}</p>
        <template v-else-if="mode">
          <p>{{ t(mode === 'restore' ? 'recovery.restoreHelp' : mode === 'export' ? 'recovery.exportHelp' : 'recovery.inspectHelp') }}</p>
          <dl v-if="inspection" class="recovery-identity">
            <dt>{{ t('recovery.backupPath') }}</dt><dd><code translate="no">{{ inspection.path }}</code></dd>
            <dt>{{ t('recovery.generation') }}</dt><dd>{{ inspection.key.generation }}</dd>
            <dt>{{ t('recovery.publicKey') }}</dt><dd><code translate="no">{{ inspection.key.publicKey }}</code></dd>
          </dl>
          <div v-else>
            <label for="recovery-file">{{ t('recovery.backupPath') }}</label>
            <div class="recovery-path-editor">
              <input id="recovery-file" v-model.trim="path" name="recovery-file" type="text" required
                autocomplete="off" :spellcheck="false" :disabled="working" aria-describedby="recovery-path-help">
              <PathPickerButton :kind="mode === 'export' ? 'save-recovery' : 'recovery'" :current="path"
                :disabled="working" @selected="path = $event" @failed="operationError = 'recovery.invalidPath'" />
            </div>
            <small id="recovery-path-help">{{ t('recovery.pathHelp') }}</small>
          </div>
          <label v-if="!restoreWouldReplace" for="recovery-password">
            {{ t('recovery.password') }}
            <input id="recovery-password" ref="passwordField" name="recovery-password" type="password"
              autocomplete="off" required minlength="12" maxlength="1024" :disabled="working"
              aria-describedby="recovery-password-help">
            <small id="recovery-password-help">{{ t('recovery.passwordHelp') }}</small>
          </label>
        </template>
        <p v-if="restoreWouldReplace" class="form-error" role="status">{{ t('recovery.otherKey') }}</p>
        <p v-if="working" class="hint" role="status">{{ t('recovery.working') }}</p>
        <p v-if="operationError" class="form-error" role="alert">{{ t(operationError) }}</p>
        <div class="dialog-actions">
          <button autofocus type="button" :disabled="working" @click="close">{{ t('action.cancel') }}</button>
          <button class="primary" type="submit" :disabled="working || !connected || restoreWouldReplace">{{ t(actionLabel) }}</button>
        </div>
      </form>
    </dialog>

    <dialog ref="recoverDialog" class="compact" aria-labelledby="recover-dialog-title"
      aria-describedby="recover-dialog-description" @cancel.prevent="closeRecover"
      @close="pendingTarget=undefined;recoverError=undefined">
      <h3 id="recover-dialog-title">{{ t('recovery.confirmTitle') }}</h3>
      <p id="recover-dialog-description">{{ t('recovery.confirmHelp') }}</p>
      <p v-if="pendingTarget" class="recovery-run">{{ t('recovery.target') }} <strong>{{ pendingTarget.name }}</strong><br>
        {{ t('recovery.runId') }} <code translate="no">{{ pendingTarget.runId }}</code></p>
      <p v-if="recoverError" class="form-error" role="alert">{{ t(recoverError) }}</p>
      <p v-if="recoverBusy" role="status" class="hint">{{ t('recovery.recovering') }}</p>
      <div class="dialog-actions">
        <button ref="recoverCancel" type="button" autofocus :disabled="recoverBusy" @click="closeRecover">{{ t('action.cancel') }}</button>
        <button type="button" class="primary" :disabled="recoverBusy || !connected || recoverNeedsRefresh" @click="confirmRecover">{{ t('recovery.confirmRecover') }}</button>
      </div>
    </dialog>
  </section>
</template>

<style scoped>
.recovery-path-editor { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .5rem; }
.recovery-actions { margin-block-start: .75rem; }
.recovery-targets { margin-block-start: 1.5rem; }
.recovery-targets select { display: block; inline-size: min(100%, 26rem); margin-block-start: .375rem; }
.recovery-run { overflow-wrap: anywhere; }
.recovery-identity { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: .375rem .75rem; }
.recovery-identity dd { margin-inline-start: 0; overflow-wrap: anywhere; }
dialog form > label { margin-block-start: .75rem; }
</style>
