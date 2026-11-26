<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { errorText, manage, manageCoreWithProgress, type CoreSource } from '../manager'
import type { CoreUpdateProgress } from '../core-progress'
import CoreUpdateProgressBar from './CoreUpdateProgressBar.vue'
import VelocityVersionDialog from './VelocityVersionDialog.vue'
import { formatDisplayTime, isMessageId, useAppLanguage, type TimeFormat } from '../i18n'
import { coreBlockedMessage, coreDiagnosticText } from '../core-diagnostics'
import { coreBuildText, coreVersionText } from '../core-display'
import OperationButton from './OperationButton.vue'
import PathPickerButton from './PathPickerButton.vue'
const props = defineProps<{
  name: string
  cachedSource?: CoreSource
  timeFormat?: TimeFormat
  query: (name:string) => Promise<{rows:CoreSource[];checkedAt:number}>
}>()
const emit = defineEmits<{ close: []; changed: [] }>()
type Profile = { family: string; minecraft: string; build: string }
type State = { installed: Profile | null; identified?: Profile | null; currentSha256: string; prepared?: (Profile & { sha256: string }) | null; pending: { request: string } | null; backups: { id: string; profile: Profile; crossVersion?: boolean; renameTo?: string | null }[]; coreFile?: string; canWrite: boolean; reason?: string | null }
type Action = 'core-install-latest' | 'core-install' | 'core-restore' | 'core-reconcile'
const dialog = ref<HTMLDialogElement>()
const confirm = ref<HTMLDialogElement>()
const progress = ref<HTMLDialogElement>()
const diagnostics = ref<HTMLDialogElement>()
const state = ref<State>()
const manualSource = ref<CoreSource>()
const source = computed(() => {
  const snapshot = manualSource.value ?? props.cachedSource
  return snapshot?.sha256 && state.value && snapshot.sha256 !== state.value.currentSha256 ? undefined : snapshot
})
const candidate = computed(() => source.value?.candidate ?? state.value?.prepared)
const prepared = computed(() => !!candidate.value?.sha256 && candidate.value.sha256 === state.value?.prepared?.sha256)
// A Purpur candidate is named by the official MD5 of its build (#41).
const candidateMd5 = computed(() => {
  const value = candidate.value
  return value && 'md5' in value ? value.md5 : undefined
})
// A verified Velocity proxy can also move to another Velocity version (#41).
const velocityVersion = ref<InstanceType<typeof VelocityVersionDialog>>()
const velocity = computed(() => state.value?.installed?.family?.toUpperCase() === 'VELOCITY')
async function versionChanged() {
  manualSource.value = undefined
  await read()
  emit('changed')
}
const purpurCheck = computed(() => source.value?.verification === 'purpurMd5'
  || [state.value?.installed?.family, state.value?.identified?.family].some(family => family?.toUpperCase() === 'PURPUR'))
const path = ref('')
// An unverified core shows the version its JAR states, marked as an offline reading.
const currentVersion = computed(() => {
  const core = state.value?.installed ?? state.value?.identified
  if (!core) return t('components.coreMaintenance.notChecked')
  const text = coreVersionText(core)
  return state.value?.installed ? text : text + t('components.coreMaintenance.offline')
})
const sha = ref('')
const result = ref('')
const error = ref('')
const checking = ref(false)
const busy = ref(false)
const confirming = ref(false)
// The step the running install or restore reported last (#60).
const step = ref<CoreUpdateProgress>()
const reused = ref(false)
const { locale, t } = useAppLanguage()
const lastRequest = ref<string>()
const copyFeedback = ref<'copied'|'failed'>()
const copying = ref(false)
const diagnosticText = computed(() => coreDiagnosticText({
  instance: props.name,
  installed: state.value?.installed,
  installedSha256: state.value?.currentSha256,
  pendingRequest: state.value?.pending?.request,
  lastRequest: lastRequest.value,
  backupIds: state.value?.backups.map(backup => backup.id),
  candidate: source.value?.candidate,
  checkedAt: source.value?.checkedAt,
}))
async function copyDiagnostics() {
  if (copying.value || !state.value) return
  copying.value = true
  copyFeedback.value = undefined
  try {
    await navigator.clipboard.writeText(diagnosticText.value)
    copyFeedback.value = 'copied'
  } catch {
    copyFeedback.value = 'failed'
  } finally { copying.value = false }
}
const sourceTime = computed(() => source.value?.checkedAt
  ? formatDisplayTime(source.value.checkedAt, locale.value, props.timeFormat) : t('components.coreMaintenance.notChecked'))
function sourceStatus(value: CoreSource | undefined) {
  if (value?.status === 'BuildAvailable') {
    return value.localBuild && value.remoteBuild
      ? t('core.source.buildAvailableBuilds', { current: value.localBuild, latest: value.remoteBuild })
      : t('core.source.buildAvailable')
  }
  if (value?.status === 'UpdateAvailable') return t('core.source.updateAvailable')
  if (value?.status === 'Current') return t('core.source.current')
  if (value?.status === 'NoStableBuild') return t('core.source.noStableBuild')
}
const statusText = computed(() => (prepared.value ? t('core.source.prepared') : result.value) || sourceStatus(source.value) || t('components.coreMaintenance.notChecked'))
function reasonText(reason: string) {
  const id = `activity.reason.${reason}`
  return isMessageId(id) ? t(id) : errorText(reason)
}
// Spigot is built locally with BuildTools, so it is only checked, never updated here (#75).
const spigot = computed(() => (state.value?.installed ?? state.value?.identified)?.family.toUpperCase() === 'SPIGOT')
const latestMinecraftText = computed(() => {
  const latest = source.value?.latestMinecraft
  if (!latest) return ''
  if (latest.status === 'Same') return t('core.source.spigot.latestSame', { version: source.value?.minecraft ?? '' })
  if (latest.status === 'Newer') return t('core.source.spigot.latestNewer', { build: latest.build ?? '' })
  return t('core.source.spigot.latestUnknown', { reason: reasonText(latest.reason ?? '') })
})
const linkError = ref('')
async function openBuildTools() {
  linkError.value = ''
  try { await invoke('open_about_link', { link: 'buildtools' }) }
  catch (failure) { linkError.value = t('core.source.spigot.linkFailed', { reason: errorText(failure) }) }
}
const primaryEnabled = computed(() => !spigot.value && state.value?.canWrite === true
  && (!!candidate.value || !source.value))
// Why Update core is disabled, shown in the dialog itself and not only as a tooltip.
const blockedReasonText = computed(() => {
  const reason = state.value?.reason
  if (!reason) return ''
  const id = coreBlockedMessage(reason)
  return id ? t(id) : errorText(reason)
})
const blockedText = computed(() => spigot.value ? t('core.source.spigot.buildTools')
  : primaryEnabled.value || !blockedReasonText.value ? ''
  : t('components.coreMaintenance.blockedReason', { reason: blockedReasonText.value }))
const choice = ref<{ action: Action; backup?: string; current?: string; candidateHash?: string; candidateBuild?: string; md5?: boolean; crossVersion?: boolean; renameTo?: string | null }>()
async function read(quiet = false) {
  try { state.value = await manage<State>(['core-state', props.name], 'background') }
  catch (failure) { if (!quiet && !error.value) error.value = errorText(failure) }
}
let refresh: number | undefined
let progressTimer: number | undefined
onMounted(() => {
  refresh = window.setInterval(() => {
    if (dialog.value?.open && !busy.value && !checking.value && !confirm.value?.open) void read(true)
  }, 5000)
})
onUnmounted(() => {
  window.clearInterval(refresh)
  window.clearTimeout(progressTimer)
})
async function check() {
  if (checking.value || busy.value) return
  const name = props.name
  checking.value = true
  error.value = ''
  try {
    const reply = await props.query(name)
    if (!dialog.value?.open || name !== props.name) return
    const checked = reply.rows.find(row=>row.name===name)
    if (checked?.sha256 && state.value && checked.sha256 !== state.value.currentSha256) {
      result.value = t('components.coreMaintenance.changed')
      return
    }
    if (checked && checked.status !== 'Unknown') manualSource.value = {...checked,checkedAt:reply.checkedAt}
    result.value = sourceStatus(checked) ?? t('components.coreMaintenance.sourceUnavailable')
    if (checked?.reason) error.value = reasonText(checked.reason)
  } catch (failure) {
    if (!dialog.value?.open || name !== props.name) return
    result.value = t('components.coreMaintenance.checkFailed')
    error.value = errorText(failure)
  } finally { checking.value = false }
}
function ask(action: Action, backup?: string) {
  if (busy.value) return
  const saved = state.value?.backups.find(item=>item.id===backup)
  choice.value = {action,backup,current:state.value?.currentSha256,
    crossVersion:saved?.crossVersion===true,
    // The new file name of an official name (#112).
    renameTo:action==='core-restore' ? saved?.renameTo : action==='core-install-latest' ? source.value?.renameTo : undefined,
    candidateHash:candidate.value?.sha256 ?? candidateMd5.value,candidateBuild:candidate.value?.build,
    md5:!candidate.value?.sha256 && !!candidateMd5.value}
  confirm.value?.showModal()
}
function primaryAction() {
  if (candidate.value) ask('core-install-latest')
  else diagnostics.value?.showModal()
}
async function apply() {
  const selected = choice.value
  const snapshot = state.value
  if (!selected || !snapshot || busy.value) return
  const request = selected.action === 'core-reconcile' ? snapshot.pending?.request : crypto.randomUUID()
  if (!request) return
  const args = [selected.action, props.name, request]
  if (selected.action === 'core-install-latest') {
    if (!selected.current || !selected.candidateHash) return
    args.push(selected.current, selected.candidateHash)
  }
  if (selected.action === 'core-install') args.push(snapshot.currentSha256, path.value, sha.value.toLowerCase())
  if (selected.action === 'core-restore' && selected.backup) args.push(snapshot.currentSha256, selected.backup)
  lastRequest.value = request
  error.value = ''
  result.value = ''
  busy.value = true
  confirming.value = true
  // This IPC returns the completed operation, with no separate receipt.
  // Keep quick replies in the confirmation and show the progress dialog for longer work.
  progressTimer = window.setTimeout(() => {
    confirm.value?.close()
    confirming.value = false
    progress.value?.showModal()
    progress.value?.focus()
  }, 250)
  step.value = undefined
  reused.value = false
  try {
    const outcome = selected.action === 'core-reconcile'
      ? await manage<{ outcome: string }>(args)
      : await manageCoreWithProgress<{ outcome: string }>(args, update => {
        step.value = update
        if (update.phase === 'reusing') reused.value = true
      })
    result.value = outcome.outcome === 'applied' ? t('components.coreMaintenance.verified') : t('components.coreMaintenance.notReplaced')
    manualSource.value = undefined
    emit('changed')
  } catch (failure) { error.value = errorText(failure) }
  finally {
    await read()
    window.clearTimeout(progressTimer)
    confirm.value?.close()
    progress.value?.close()
    confirming.value = false
    busy.value = false
    step.value = undefined
  }
}
function close() { if (!busy.value) { dialog.value?.close(); emit('close') } }
defineExpose({ open: () => { dialog.value?.showModal(); void read() } })
</script>
<template>
  <dialog ref="dialog" class="compact" aria-labelledby="core-dialog-title" @cancel.prevent="close">
    <h2 id="core-dialog-title">{{ t('components.coreMaintenance.title', { name }) }}</h2>
    <dl class="import-preview">
      <dt>{{ t('components.coreMaintenance.currentVersion') }}</dt><dd>{{ currentVersion }}</dd>
      <dt>{{ t('components.coreMaintenance.latestVersion') }}</dt><dd>{{ (source?.remoteBuild ?? candidate?.build) ? coreBuildText(source?.remoteBuild ?? candidate?.build ?? '') : t('components.coreMaintenance.notChecked') }}</dd>
      <dt>{{ t('components.coreMaintenance.checkedAt') }}</dt><dd>{{ sourceTime }}</dd>
      <dt>{{ t('components.coreMaintenance.updateStatus') }}</dt><dd role="status">{{ statusText }}</dd>
      <template v-if="spigot && latestMinecraftText">
        <dt>{{ t('core.source.spigot.latestMinecraft') }}</dt><dd>{{ latestMinecraftText }}</dd>
      </template>
      <template v-if="purpurCheck"><dt>{{ t('components.coreMaintenance.verification') }}</dt><dd>{{ t('components.coreMaintenance.purpurMd5') }}</dd></template>
    </dl>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <p v-if="blockedText" id="core-update-blocked" class="hint" role="status">{{ blockedText }}</p>
    <p v-if="spigot"><button type="button" @click="openBuildTools">{{ t('core.source.spigot.buildToolsLink') }}</button></p>
    <p v-if="linkError" class="form-error" role="alert">{{ linkError }}</p>
    <div class="core-dialog-actions">
      <button class="primary" :disabled="!primaryEnabled" :title="blockedText || undefined" :aria-describedby="blockedText ? 'core-update-blocked' : undefined"
        @click="primaryAction">{{ candidate || spigot ? t('components.coreMaintenance.update') : t('components.coreMaintenance.chooseLocal') }}</button>
      <OperationButton :label="t('components.coreMaintenance.check')" :busy-label="t('action.checkingUpdates')" :busy="checking" :disabled="busy" @click="check" />
      <button @click="diagnostics?.showModal()">{{ t('components.coreMaintenance.diagnostics') }}</button>
      <button autofocus @click="close">{{ t('action.close') }}</button>
      <button v-if="velocity" class="core-dialog-wide" :disabled="!state?.canWrite || busy" @click="velocityVersion?.open()">{{ t('components.velocityVersion.open') }}</button>
    </div>
  </dialog>
  <VelocityVersionDialog ref="velocityVersion" :name="name" @changed="versionChanged" />
  <dialog ref="diagnostics" aria-labelledby="core-diagnostics-title">
      <h2 id="core-diagnostics-title">{{ t('components.coreMaintenance.diagnostics') }}</h2>
      <p v-if="blockedReasonText" class="hint">{{ blockedReasonText }}</p>
      <p v-if="result" role="status">{{ result }}</p>
      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div class="field">
        <label for="local-core-path">{{ t('components.coreMaintenance.coreJar') }}</label>
        <div class="core-path-editor">
          <input id="local-core-path" v-model.trim="path" name="local-core-path" autocomplete="off" spellcheck="false">
          <PathPickerButton kind="jar" :current="path" :disabled="busy"
            @selected="path = $event" @failed="error = t('components.coreMaintenance.pickerFailed')" />
        </div>
      </div>
      <div class="field"><label for="local-core-sha256">SHA-256</label><input id="local-core-sha256" v-model.trim="sha" name="local-core-sha256" autocomplete="off" spellcheck="false"></div>
      <button :disabled="!state?.canWrite || !path || !/^[0-9a-f]{64}$/i.test(sha)" @click="ask('core-install')">{{ t('components.coreMaintenance.update') }}</button>
      <ul v-if="state?.backups.length"><li v-for="backup in state.backups" :key="backup.id">{{ backup.crossVersion ? coreVersionText(backup.profile) : coreBuildText(backup.profile.build) }} <button :disabled="!state.canWrite" @click="ask('core-restore',backup.id)">{{ t('components.coreMaintenance.restore') }}</button></li></ul>
      <details v-if="state" class="detail-section">
        <summary>{{ t('core.diagnostics.show') }}</summary>
        <pre class="selectable" tabindex="0" :aria-label="t('core.diagnostics.show')" translate="no">{{ diagnosticText }}</pre>
        <button type="button" :aria-disabled="copying" @click="copyDiagnostics">{{ t('core.diagnostics.copy') }}</button>
        <p v-if="copyFeedback==='copied'" class="hint" role="status">{{ t('core.diagnostics.copied') }}</p>
        <p v-if="copyFeedback==='failed'" class="form-error" role="alert">{{ t('core.diagnostics.copyFailed') }}</p>
      </details>
      <div class="dialog-actions">
        <button v-if="state?.pending" @click="ask('core-reconcile')">{{ t('components.coreMaintenance.reconcile') }}</button>
        <button autofocus @click="diagnostics?.close()">{{ t('action.close') }}</button>
      </div>
  </dialog>
  <dialog ref="confirm" class="compact" aria-labelledby="core-confirm-title" @cancel="busy && $event.preventDefault()">
    <h2 id="core-confirm-title">{{ choice?.action==='core-restore'?t('components.coreMaintenance.restoreTitle'):choice?.action==='core-reconcile'?t('components.coreMaintenance.reconcileTitle'):t('components.coreMaintenance.update') }}</h2>
    <p v-if="choice?.action==='core-install-latest' && choice.candidateBuild">{{ t('components.coreMaintenance.targetBuild', { build: choice.candidateBuild }) }}</p>
    <p v-if="choice?.action==='core-install-latest' && choice.md5">{{ t('components.coreMaintenance.purpurMd5Note') }}</p>
    <p v-if="choice?.renameTo">{{ t(choice.action==='core-restore' ? 'components.coreMaintenance.restoreName' : 'components.coreMaintenance.renameTo', { name: choice.renameTo }) }}</p>
    <p v-if="choice?.action==='core-restore' && choice.crossVersion">{{ t('components.velocityVersion.restoreNote') }}</p>
    <p>{{ t('components.coreMaintenance.backupNote') }}</p>
    <div class="dialog-actions">
      <button autofocus :disabled="busy" @click="confirm?.close()">{{ t('action.cancel') }}</button>
      <OperationButton class="primary" :label="t('action.confirm')" :busy-label="t('action.submitting')" :busy="confirming" @click="apply" />
    </div>
  </dialog>
  <dialog ref="progress" class="compact" tabindex="-1" aria-labelledby="core-progress-title" @cancel.prevent @keydown.esc.stop.prevent>
    <h2 id="core-progress-title">{{ t('components.coreMaintenance.progressTitle') }}</h2><p>{{ t('components.coreMaintenance.progressBody') }}</p>
    <CoreUpdateProgressBar :progress="step" :reused="reused" fallback="components.coreMaintenance.progressLabel" />
  </dialog>
</template>
<style scoped>
.core-path-editor { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .5rem; }
.core-dialog-actions { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: .5rem; }
.core-dialog-wide { grid-column: 1 / -1; }
</style>
