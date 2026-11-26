<script setup lang="ts">
import { computed, useId } from 'vue'
import { useAppLanguage, type MessageId } from '../i18n'
import { coreProgressBytes, coreProgressStep, coreProgressValue, type CoreUpdateProgress } from '../core-progress'

// One step of a core update (#60). The bar has a value only while a download
// with an official size runs; every other step shows an indeterminate bar.
// `reused` keeps saying that the downloaded core is used after that short step ends.
const props = defineProps<{ progress?: CoreUpdateProgress | null; fallback: MessageId; prefix?: string; reused?: boolean }>()
const { locale, t } = useAppLanguage()
const label = useId()
const step = computed(() => t(coreProgressStep(props.progress) ?? props.fallback))
const text = computed(() => props.prefix ? t('core.batch.itemProgress', { name: props.prefix, step: step.value }) : step.value)
const bytes = computed(() => {
  const message = coreProgressBytes(props.progress, locale.value)
  if (message) return t(message.id, message.values)
  return props.reused && props.progress?.phase !== 'reusing' ? t('core.progress.reusing') : ''
})
const value = computed(() => coreProgressValue(props.progress))
</script>

<template>
  <div class="core-update-progress">
    <div class="core-update-progress-heading">
      <p :id="label" role="status">{{ text }}</p>
      <span v-if="bytes">{{ bytes }}</span>
    </div>
    <progress :aria-labelledby="label" :value.attr="value?.value" :max.attr="value?.max"></progress>
  </div>
</template>

<style scoped>
.core-update-progress { display: grid; gap: .5rem; margin-block: .75rem; }
.core-update-progress p { margin: 0; }
.core-update-progress-heading { display: flex; justify-content: space-between; gap: .5rem; font-variant-numeric: tabular-nums; }
.core-update-progress-heading span { flex: none; }
progress {
  appearance: none;
  width: 100%;
  height: .625rem;
  container-type: inline-size;
  overflow: clip;
  border: 0;
  border-radius: 999px;
}
progress::-webkit-progress-bar { background: var(--raised); }
progress::-webkit-progress-value { background: var(--primary); border-radius: 999px; }
progress[value]::-webkit-progress-value { transition: inline-size .12s linear; }
progress:indeterminate::-webkit-progress-value { animation: core-update-progress-slide 1.25s linear infinite; }
@keyframes core-update-progress-slide {
  from, to { inline-size: 28%; margin-right: auto; }
  from { translate: -100% 0; }
  to { translate: 100cqi 0; }
}
@media (prefers-reduced-motion: reduce) {
  progress:indeterminate::-webkit-progress-value { animation-delay: -.625s; animation-play-state: paused; }
  progress[value]::-webkit-progress-value { transition: none; }
}
</style>
