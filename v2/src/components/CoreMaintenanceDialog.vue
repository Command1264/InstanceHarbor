<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { errorText, manage, type CoreSource } from '../manager'
import { formatDisplayTime, useAppLanguage, type TimeFormat } from '../i18n'
import { coreDiagnosticText } from '../core-diagnostics'
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
type State = { installed: Profile | null; currentSha256: string; prepared?: (Profile & { sha256: string }) | null; pending: { request: string } | null; backups: { id: string; profile: Profile }[]; canWrite: boolean; reason?: string | null }
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
const prepared = computed(() => !!candidate.value && candidate.value.sha256 === state.value?.prepared?.sha256)
const path = ref('')
const sha = ref('')
const result = ref('')
const error = ref('')
const checking = ref(false)
const busy = ref(false)
const confirming = ref(false)
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
  ? formatDisplayTime(source.value.checkedAt, locale.value, props.timeFormat) : '尚未核對')
function sourceStatus(value: CoreSource | undefined) {
  if (value?.status === 'NativeBehind' && typeof value.nativeVersion?.behind === 'number') {
    const count = new Intl.NumberFormat(locale.value).format(value.nativeVersion.behind)
    return t('core.source.nativeBehind', { count })
  }
  if (value?.status === 'UpdateAvailable') return t('core.source.updateAvailable')
  if (value?.status === 'Current') return t('core.source.current')
  if (value?.status === 'NoStableBuild') return t('core.source.noStableBuild')
}
const statusText = computed(() => (prepared.value ? t('core.source.prepared') : result.value) || sourceStatus(source.value) || '尚未核對')
const primaryEnabled = computed(() => state.value?.canWrite === true
  && (!!candidate.value || !source.value || state.value.installed?.family==='SPIGOT'))
const choice = ref<{ action: Action; backup?: string; current?: string; candidateHash?: string; candidateBuild?: string }>()
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
      result.value = '核心已變更，請重新核對更新'
      return
    }
    if (checked && checked.status !== 'Unknown') manualSource.value = {...checked,checkedAt:reply.checkedAt}
    result.value = sourceStatus(checked) ?? '未能核對更新來源'
    if (checked?.reason === 'CoreSourceSpigotLocalRefsUnverified') error.value = t('core.source.spigotRefsUnverified')
    else if (checked?.reason === 'CoreSourceSpigotNativeCheckFailed') error.value = t('core.source.spigotNativeFailed')
    else if (checked?.reason) error.value = errorText(checked.reason)
  } catch (failure) {
    if (!dialog.value?.open || name !== props.name) return
    result.value = '更新核對失敗'
    error.value = errorText(failure)
  } finally { checking.value = false }
}
function ask(action: Action, backup?: string) {
  if (busy.value) return
  choice.value = {action,backup,current:state.value?.currentSha256,
    candidateHash:candidate.value?.sha256,candidateBuild:candidate.value?.build}
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
  try {
    const outcome = await manage<{ outcome: string }>(args)
    result.value = outcome.outcome === 'applied' ? '已核對核心維護結果' : '已核對，未替換核心'
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
  }
}
function close() { if (!busy.value) { dialog.value?.close(); emit('close') } }
defineExpose({ open: () => { dialog.value?.showModal(); void read() } })
</script>
<template>
  <dialog ref="dialog" class="compact" aria-labelledby="core-dialog-title" @cancel.prevent="close">
    <h2 id="core-dialog-title">{{ name }} · 核心維護</h2>
    <dl class="import-preview">
      <dt>目前版本</dt><dd>{{ state?.installed ? [state.installed.family,state.installed.minecraft,'build',state.installed.build].join(' ') : '尚未核對' }}</dd>
      <dt>最新版本</dt><dd>{{ source?.remoteBuild ?? candidate?.build ?? '尚未核對' }}</dd>
      <dt>核對時間</dt><dd>{{ sourceTime }}</dd>
      <dt>更新狀態</dt><dd role="status">{{ statusText }}</dd>
    </dl>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <div class="core-dialog-actions">
      <button class="primary" :disabled="!primaryEnabled" :title="state?.reason ? errorText(state.reason) : undefined"
        @click="primaryAction">{{ candidate ? '更新核心' : '選擇本機核心' }}</button>
      <OperationButton label="檢查更新" :busy-label="t('action.checkingUpdates')" :busy="checking" :disabled="busy" @click="check" />
      <button @click="diagnostics?.showModal()">備份與診斷</button>
      <button autofocus @click="close">關閉</button>
    </div>
  </dialog>
  <dialog ref="diagnostics" aria-labelledby="core-diagnostics-title">
      <h2 id="core-diagnostics-title">備份與診斷</h2>
      <p v-if="state?.reason" class="hint">{{ errorText(state.reason) }}</p>
      <p v-if="result" role="status">{{ result }}</p>
      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div class="field">
        <label for="local-core-path">核心 JAR</label>
        <div class="core-path-editor">
          <input id="local-core-path" v-model.trim="path" name="local-core-path" autocomplete="off" spellcheck="false">
          <PathPickerButton kind="jar" :current="path" :disabled="busy"
            @selected="path = $event" @failed="error = '無法開啟檔案選取器。'" />
        </div>
      </div>
      <div class="field"><label for="local-core-sha256">SHA-256</label><input id="local-core-sha256" v-model.trim="sha" name="local-core-sha256" autocomplete="off" spellcheck="false"></div>
      <button :disabled="!state?.canWrite || !path || !/^[0-9a-f]{64}$/i.test(sha)" @click="ask('core-install')">更新核心</button>
      <ul v-if="state?.backups.length"><li v-for="backup in state.backups" :key="backup.id">Build {{ backup.profile.build }} <button :disabled="!state.canWrite" @click="ask('core-restore',backup.id)">還原</button></li></ul>
      <details v-if="state" class="detail-section">
        <summary>{{ t('core.diagnostics.show') }}</summary>
        <pre class="selectable" tabindex="0" :aria-label="t('core.diagnostics.show')" translate="no">{{ diagnosticText }}</pre>
        <button type="button" :aria-disabled="copying" @click="copyDiagnostics">{{ t('core.diagnostics.copy') }}</button>
        <p v-if="copyFeedback==='copied'" class="hint" role="status">{{ t('core.diagnostics.copied') }}</p>
        <p v-if="copyFeedback==='failed'" class="form-error" role="alert">{{ t('core.diagnostics.copyFailed') }}</p>
      </details>
      <div class="dialog-actions">
        <button v-if="state?.pending" @click="ask('core-reconcile')">核對未完成工作</button>
        <button autofocus @click="diagnostics?.close()">關閉</button>
      </div>
  </dialog>
  <dialog ref="confirm" class="compact" aria-labelledby="core-confirm-title" @cancel="busy && $event.preventDefault()">
    <h2 id="core-confirm-title">{{ choice?.action==='core-restore'?'還原核心':choice?.action==='core-reconcile'?'核對維護結果':'更新核心' }}</h2>
    <p v-if="choice?.action==='core-install-latest' && choice.candidateBuild">目標版本為 Build {{ choice.candidateBuild }}。</p>
    <p>管理服務會再次核對目標、 Port、已安裝核心及備份。請確認要繼續。</p>
    <div class="dialog-actions">
      <button autofocus :disabled="busy" @click="confirm?.close()">取消</button>
      <OperationButton class="primary" label="確認" :busy-label="t('action.submitting')" :busy="confirming" @click="apply" />
    </div>
  </dialog>
  <dialog ref="progress" class="compact" tabindex="-1" aria-labelledby="core-progress-title" @cancel.prevent @keydown.esc.stop.prevent>
    <h2 id="core-progress-title">核心維護進行中</h2><p>請等待管理服務核對檔案寫入結果。</p><progress aria-label="核心維護"></progress>
  </dialog>
</template>
<style scoped>
.core-path-editor { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .5rem; }
.core-dialog-actions { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: .5rem; }
</style>
