<script setup lang="ts">
import { ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  type?: 'button' | 'submit'
  label: string
  busyLabel: string
  busy?: boolean
  disabled?: boolean
  delay?: number
}>(), { type: 'button', delay: 220 })
const emit = defineEmits<{ click: [MouseEvent] }>()
const spinning = ref(false)
const button = ref<HTMLButtonElement>()
defineExpose({ focus: () => button.value?.focus() })

function activate(event: MouseEvent) {
  if (props.busy || props.disabled) event.preventDefault()
  else emit('click', event)
}

watch(() => props.busy, (busy, _previous, cleanup) => {
  spinning.value = false
  if (!busy) return
  const timer = setTimeout(() => { spinning.value = true }, props.delay)
  cleanup(() => clearTimeout(timer))
}, { immediate: true })
</script>

<template>
  <button ref="button" :type="type" class="operation-button" :disabled="disabled" :aria-disabled="busy || undefined"
    :data-idle-label="label" :data-busy-label="busyLabel"
    @click="activate">
    <span class="operation-button-label">{{ busy ? busyLabel : label }}<span v-if="busy && spinning" class="operation-spinner" aria-hidden="true"></span></span>
  </button>
</template>

<style scoped>
.operation-button { display: inline-grid; place-items: center; text-align: center; }
.operation-button[aria-disabled='true'] { cursor: wait; }
.operation-button::before, .operation-button::after, .operation-button-label { grid-area: 1 / 1; }
.operation-button::before, .operation-button::after { visibility: hidden; }
.operation-button::before { content: attr(data-idle-label); }
.operation-button::after { content: attr(data-busy-label); padding-inline-end: .95em; }
.operation-button-label { display: inline-flex; align-items: center; justify-content: center; gap: .3em; }
.operation-spinner {
  flex: none; inline-size: .65em; block-size: .65em;
  border: 1.5px solid currentColor; border-inline-end-color: transparent; border-radius: 50%;
  animation: operation-spin .8s linear infinite;
}
@keyframes operation-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  .operation-spinner { animation: none; }
}
</style>
