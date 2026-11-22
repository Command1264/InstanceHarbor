<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { errorText, manage, rawError } from '../manager'
import { useAppLanguage } from '../i18n'
import OperationButton from './OperationButton.vue'

type TimePlan = {
  run: string
  key: string
  before: string
  target: string
  flowing: boolean | null
  registration: string
  sha256: string
  createdAt: number
}
type TimeReceipt = {
  name: string
  request: string
  state: string
  observed?: string | null
  error?: string | null
}
const { t } = useAppLanguage()
const props = defineProps<{ name: string; run?: string; ready: boolean; supported: boolean; connected: boolean; saving: boolean }>()
const emit = defineEmits<{ (event: 'mutating'): void }>()
const target = ref('')
const edited = ref(false)
const readingClock = ref(false)
const phase = ref<'checking' | 'applying' | null>(null)
const working = computed(() => phase.value !== null)
const feedback = ref('')
const error = ref('')
const blocked = ref(false)
const plan = ref<TimePlan>()
const dialog = ref<HTMLDialogElement>()
const trigger = ref<InstanceType<typeof OperationButton>>()
let generation = 0
const valid = computed(() => /^\d+$/.test(target.value) && Number(target.value) <= 23999)
const readable = computed(() => props.connected && props.ready && props.supported && !!props.run)
const available = computed(() => readable.value && !props.saving && !blocked.value)

watch([() => props.name, () => props.run, readable], async ([name, run, canRead], [oldName, oldRun], cleanup) => {
  target.value = ''
  edited.value = false
  readingClock.value = false
  if (name !== oldName || run !== oldRun) {
    feedback.value = ''
    error.value = ''
    blocked.value = false
  }
  generation++
  phase.value = null
  plan.value = undefined
  dialog.value?.close()
  if (!canRead || !run) return
  let current = true
  cleanup(() => { current = false })
  for (let attempt = 0; attempt < 7 && current && !edited.value; attempt++) {
    readingClock.value = true
    try {
      const observed: { run: string; value: string } = await manage(['world-live-clock', name, run], 'background')
      if (current && !edited.value && observed.run === run && /^\d+$/.test(observed.value) && Number(observed.value) <= 23999)
        target.value = observed.value
      return
    } catch (failure) {
      const reason = rawError(failure)
      if (reason.includes('WorldConsoleInputNotVerified')) {
        if (current) blocked.value = true
        return
      }
      if (!/ManagerBusy|WorldRunNotReady/.test(reason)) return
    } finally {
      if (current) readingClock.value = false
    }
    if (attempt < 6 && current) await new Promise(resolve => setTimeout(resolve, 2000))
  }
}, { immediate: true })
onBeforeUnmount(() => { generation++ })

async function prepare() {
  if (!available.value || !valid.value || working.value || readingClock.value || !props.run) return
  const { name, run } = props
  const epoch = ++generation
  phase.value = 'checking'
  try {
    const preview = await manage<TimePlan>(['world-live-preview', name, run, 'initialTime', String(Number(target.value))])
    if (epoch !== generation) return
    if (preview.run !== run || preview.key !== 'initialTime' || preview.target !== String(Number(target.value)))
      throw new Error('WorldPreviewChanged')
    error.value = ''
    plan.value = preview
    await nextTick()
    if (epoch === generation) dialog.value?.showModal()
  } catch (failure) {
    if (epoch === generation) error.value = t('components.worldTime.readFailed', { reason: errorText(failure) })
  } finally {
    if (epoch === generation) phase.value = null
  }
}

async function restoreFocus() {
  await nextTick()
  const button = trigger.value?.$el as HTMLButtonElement | undefined
  if (document.activeElement === document.body && !button?.disabled) button?.focus()
}
function dismiss() {
  if (dialog.value?.open) return
  plan.value = undefined
  if (!working.value) void restoreFocus()
}
async function applyTime() {
  const confirmed = plan.value
  if (!confirmed || !available.value || working.value || confirmed.run !== props.run || !dialog.value?.open) return
  const { name } = props
  const epoch = ++generation
  const request = crypto.randomUUID()
  phase.value = 'applying'
  emit('mutating')
  try {
    const receipt = await manage<TimeReceipt>(['world-live-apply', name, request, JSON.stringify(confirmed), 'time'])
    if (epoch !== generation) return
    if (receipt.name !== name || receipt.request !== request) throw new Error('WorldReceiptMismatch')
    feedback.value = ''
    if (receipt.state === 'NoChange') feedback.value = t('components.worldTime.unchanged', { value: receipt.observed ?? confirmed.target })
    else if (receipt.state === 'Applied' || receipt.state === 'AwaitingNormalStop') {
      feedback.value = t(receipt.state === 'AwaitingNormalStop' ? 'components.worldTime.adjustedSavedOnStop' : 'components.worldTime.adjusted', { value: receipt.observed ?? confirmed.target })
    } else {
      blocked.value = true
      error.value = t('components.worldTime.unconfirmed')
    }
  } catch (failure) {
    if (epoch === generation) {
      feedback.value = ''
      const reason = errorText(failure)
      if (/WorldPreviewExpired|WorldPreviewChanged|WorldRunUnavailable|WorldRunChanged|WorldRunNotReady/.test(rawError(failure))) {
        error.value = t('components.worldTime.expired')
      } else {
        blocked.value = true
        error.value = t('components.worldTime.applyFailed', { reason })
      }
    }
  } finally {
    if (epoch === generation) {
      phase.value = null
      dialog.value?.close()
      void restoreFocus()
    }
  }
}
</script>

<template>
  <section class="world-time" aria-labelledby="world-time-title">
    <h3 id="world-time-title">{{ t('components.worldTime.title') }}</h3>
    <p id="world-time-help" class="hint">
      <template v-if="!supported">{{ t('components.worldTime.unsupportedHelp') }}</template>
      <template v-else-if="!ready">{{ t('components.worldTime.notReadyHelp') }}</template>
      <template v-else>{{ t('components.worldTime.readyHelp') }}</template>
    </p>
    <div class="world-time-actions">
      <label for="current-world-time">{{ t('components.worldTime.target') }}</label>
      <input id="current-world-time" v-model="target" name="current-world-time" inputmode="numeric"
        autocomplete="off" aria-describedby="world-time-help" :aria-invalid="!!target && !valid || undefined"
        :disabled="!available || working" @input="edited = true">
      <OperationButton ref="trigger" :label="t('components.worldTime.adjust')" :busy-label="phase === 'applying' ? t('action.submitting') : t('action.checking')"
        :busy="working" :disabled="!available || !valid || readingClock" @click="prepare" />
    </div>
    <p class="world-time-status" :class="{ 'world-time-success': !phase && feedback }" role="status">
      {{ feedback }}
    </p>
    <p v-if="target && !valid" class="form-error" role="alert">{{ t('components.worldTime.invalid') }}</p>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
  </section>
  <dialog ref="dialog" class="compact" aria-labelledby="world-time-confirm" @close="dismiss">
    <h2 id="world-time-confirm">{{ t('components.worldTime.confirmTitle') }}</h2>
    <p v-if="plan">{{ t('components.worldTime.plan', { before: plan.before, target: plan.target }) }}</p>
    
    <div class="dialog-actions">
      <button type="button" autofocus @click="dialog?.close()">{{ phase === 'applying' ? t('action.close') : t('action.cancel') }}</button>
      <OperationButton class="primary" :label="t('components.worldTime.applyNow')" :busy-label="t('action.submitting')" :busy="phase === 'applying'"
        :disabled="!available" @click="applyTime" />
    </div>
  </dialog>
</template>

<style scoped>
.world-time { margin-block: 1.25rem; padding-block: 1rem; border-block: 1px solid var(--border); }
.world-time h3 { margin: 0 0 .5rem; }
.world-time-actions { display: flex; flex-wrap: wrap; gap: .5rem; align-items: center; }
.world-time-actions input { width: 8rem; }
.world-time-status:empty { margin: 0; }
.world-time-success { color: var(--ok); }
.world-time + dialog { overscroll-behavior: contain; }
</style>
