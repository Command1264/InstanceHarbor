<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

// Centered mode is for full-page lists. It fills the nearest positioned ancestor below
// whatever precedes it (a table header, for example). Inside a scroll area that ancestor is
// the non-scrolling shell, so the bar stays in the middle of the visible region while the
// content scrolls horizontally. Dialogs keep the default, left-aligned layout.
const props = defineProps<{ label: string; centered?: boolean }>()
const root = ref<HTMLDivElement>()
const top = ref(0)
let observer: ResizeObserver | undefined

function measure() {
  const element = root.value
  const container = element?.offsetParent
  const before = element?.previousElementSibling
  if (!element || !container || !before) { top.value = 0; return }
  top.value = Math.max(0, Math.round(before.getBoundingClientRect().bottom - container.getBoundingClientRect().top))
}
onMounted(() => {
  if (!props.centered || !root.value) return
  measure()
  observer = new ResizeObserver(measure)
  const before = root.value.previousElementSibling
  if (before) observer.observe(before)
  if (root.value.offsetParent) observer.observe(root.value.offsetParent)
})
onUnmounted(() => observer?.disconnect())
</script>

<template>
  <div ref="root" class="initial-query-progress" :class="{ centered }" :style="centered ? { top: `${top}px` } : undefined">
    <label>
      <span>{{ label }}</span>
      <progress></progress>
    </label>
  </div>
</template>

<style scoped>
.initial-query-progress { padding: .75rem; color: var(--muted); }
.initial-query-progress label { display: grid; gap: .35rem; max-width: 20rem; }
.initial-query-progress progress { width: 100%; height: .5rem; accent-color: var(--primary); }
.initial-query-progress.centered { position: absolute; right: 0; bottom: 0; left: 0; display: grid; place-items: center; pointer-events: none; }
/* The empty third row matches the text row, so the bar itself sits on the center line. */
.initial-query-progress.centered label { width: min(20rem, 100%); grid-template-rows: 1fr auto 1fr; text-align: center; }
.initial-query-progress.centered label::after { content: ''; }
</style>
