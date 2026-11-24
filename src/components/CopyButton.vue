<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import { useAppLanguage } from '../i18n'

const props = defineProps<{ text: string; label: string; disabled?: boolean }>()
const { t } = useAppLanguage()
const copied = ref(false)
const failed = ref(false)
let timer: ReturnType<typeof setTimeout> | undefined

async function copy() {
  if (props.disabled || !props.text) return
  clearTimeout(timer)
  failed.value = false
  try {
    await navigator.clipboard.writeText(props.text)
    copied.value = true
  } catch {
    copied.value = false
    failed.value = true
  }
  timer = setTimeout(() => { copied.value = false; failed.value = false }, 2000)
}

onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <button type="button" class="copy-button" :disabled="disabled || !text"
    :aria-label="copied ? t('components.copy.done') : failed ? t('components.copy.failed') : label"
    :title="copied ? t('components.copy.done') : failed ? t('components.copy.failed') : label" @click="copy">
    <!-- Lucide Copy and Check, ISC license. See assets/icons/sources.json. -->
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
      stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
      <path v-if="copied" d="M20 6 9 17l-5-5"/>
      <template v-else>
        <rect width="14" height="14" x="8" y="8" rx="2" ry="2"/>
        <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>
      </template>
    </svg>
  </button>
  <span class="sr-only" role="status">{{ copied ? t('components.copy.done') : failed ? t('components.copy.failed') : '' }}</span>
</template>

<style scoped>
.copy-button { display: inline-flex; align-items: center; justify-content: center; inline-size: 1.75rem; block-size: 1.75rem; padding: 0; flex: none; }
.copy-button svg { flex: none; }
</style>
