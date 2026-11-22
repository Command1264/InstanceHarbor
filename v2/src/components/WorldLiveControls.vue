<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { errorText, manage, rawError } from '../manager'
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
    if (epoch === generation) error.value = '無法核對目前的世界時間：' + errorText(failure)
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
    if (receipt.state === 'NoChange') feedback.value = `目前世界時間已是 ${receipt.observed ?? confirmed.target} 刻。`
    else if (receipt.state === 'Applied' || receipt.state === 'AwaitingNormalStop') {
      feedback.value = `世界時間已調整，讀回 ${receipt.observed ?? confirmed.target} 刻。`
      if (receipt.state === 'AwaitingNormalStop') feedback.value += '世界會在正常停止時存檔。'
    } else {
      blocked.value = true
      error.value = '世界時間調整的結果未確認。請到操作紀錄查看結果。'
    }
  } catch (failure) {
    if (epoch === generation) {
      feedback.value = ''
      const reason = errorText(failure)
      if (/WorldPreviewExpired|WorldPreviewChanged|WorldRunUnavailable|WorldRunChanged|WorldRunNotReady/.test(rawError(failure))) {
        error.value = '此確認已失效，請重新核對目前的世界時間。'
      } else {
        blocked.value = true
        error.value = `無法確認世界時間是否已調整：${reason}。請到操作紀錄查看結果。`
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
    <h3 id="world-time-title">目前世界時間</h3>
    <p id="world-time-help" class="hint">
      <template v-if="!supported">尚未核對目前核心與版本是否支援調整世界時間。</template>
      <template v-else-if="!ready">伺服器就緒後可即時調整世界時間。</template>
      <template v-else>確認後即時調整目前運行中的世界。請輸入 0 到 23999 刻。</template>
    </p>
    <div class="world-time-actions">
      <label for="current-world-time">目標刻數</label>
      <input id="current-world-time" v-model="target" name="current-world-time" inputmode="numeric"
        autocomplete="off" aria-describedby="world-time-help" :aria-invalid="!!target && !valid || undefined"
        :disabled="!available || working" @input="edited = true">
      <OperationButton ref="trigger" label="調整世界時間" :busy-label="phase === 'applying' ? '送出中' : '核對中'"
        :busy="working" :disabled="!available || !valid || readingClock" @click="prepare" />
    </div>
    <p class="world-time-status" :class="{ 'world-time-success': !phase && feedback }" role="status">
      {{ feedback }}
    </p>
    <p v-if="target && !valid" class="form-error" role="alert">請輸入 0 到 23999 的整數。</p>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
  </section>
  <dialog ref="dialog" class="compact" aria-labelledby="world-time-confirm" @close="dismiss">
    <h2 id="world-time-confirm">確認調整世界時間</h2>
    <p v-if="plan">核對時為 {{ plan.before }} 刻，目標為 {{ plan.target }} 刻。</p>
    
    <div class="dialog-actions">
      <button type="button" autofocus @click="dialog?.close()">{{ phase === 'applying' ? '關閉' : '取消' }}</button>
      <OperationButton class="primary" label="立即調整" busy-label="送出中" :busy="phase === 'applying'"
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
