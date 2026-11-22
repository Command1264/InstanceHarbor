<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, useId } from 'vue'
import { useAppLanguage } from '../i18n'

const props = defineProps<{ label: string }>()
const { t } = useAppLanguage()
const viewportId = useId()
const viewport = ref<HTMLDivElement>()
const inner = ref<HTMLDivElement>()
const track = ref<HTMLDivElement>()
const horizontalTrack = ref<HTMLDivElement>()
const needed = ref(false)
const thumbHeight = ref(28)
const thumbTop = ref(0)
const maximum = ref(0)
const current = ref(0)
const maximumX = ref(0)
const currentX = ref(0)
const thumbWidth = ref(28)
const thumbLeft = ref(0)
let observer: ResizeObserver | undefined
// Lets a parent keep a live view pinned to its end across content updates.
defineExpose({ viewport })
let frame = 0
let drag: { pointer: number; startPosition: number; startScroll: number; horizontal: boolean } | null = null

function measure() {
  frame = 0
  const view = viewport.value
  if (!view) return
  maximum.value = Math.max(0, view.scrollHeight - view.clientHeight)
  current.value = view.scrollTop
  maximumX.value = Math.max(0, view.scrollWidth - view.clientWidth)
  currentX.value = view.scrollLeft
  needed.value = maximum.value > 1
  const height = Math.max(0, (view.parentElement?.clientHeight ?? view.clientHeight) - 8)
  thumbHeight.value = Math.min(height, Math.max(28, height * view.clientHeight / Math.max(1, view.scrollHeight)))
  thumbTop.value = maximum.value ? (height - thumbHeight.value) * view.scrollTop / maximum.value : 0
  const width = Math.max(0, (view.parentElement?.clientWidth ?? view.clientWidth) - 18)
  thumbWidth.value = Math.min(width, Math.max(28, width * view.clientWidth / Math.max(1, view.scrollWidth)))
  thumbLeft.value = maximumX.value ? (width - thumbWidth.value) * view.scrollLeft / maximumX.value : 0
}
function update() {
  if (!frame) frame = requestAnimationFrame(measure)
}
function pointerDown(event: PointerEvent, horizontal = false) {
  const view = viewport.value
  const rail = horizontal ? horizontalTrack.value : track.value
  if (!view || !rail) return
  event.preventDefault()
  rail.focus({ preventScroll: true })
  if (!(event.target as HTMLElement).classList.contains('overlay-thumb')) {
    const rect = rail.getBoundingClientRect()
    const position = horizontal ? event.clientX - rect.left : event.clientY - rect.top
    const size = horizontal ? thumbWidth.value : thumbHeight.value
    const range = Math.max(1, (horizontal ? rail.clientWidth : rail.clientHeight) - size)
    const value = Math.max(0, Math.min(1, (position - size / 2) / range)) * (horizontal ? maximumX.value : maximum.value)
    if (horizontal) view.scrollLeft = value
    else view.scrollTop = value
  }
  drag = { pointer: event.pointerId, startPosition: horizontal ? event.clientX : event.clientY, startScroll: horizontal ? view.scrollLeft : view.scrollTop, horizontal }
  rail.setPointerCapture(event.pointerId)
  update()
}
function pointerMove(event: PointerEvent) {
  if (!drag || event.pointerId !== drag.pointer || !viewport.value) return
  const rail = drag.horizontal ? horizontalTrack.value : track.value
  if (!rail) return
  const range = Math.max(1, (drag.horizontal ? rail.clientWidth - thumbWidth.value : rail.clientHeight - thumbHeight.value))
  const delta = (drag.horizontal ? event.clientX : event.clientY) - drag.startPosition
  const value = drag.startScroll + delta * (drag.horizontal ? maximumX.value : maximum.value) / range
  if (drag.horizontal) viewport.value.scrollLeft = value
  else viewport.value.scrollTop = value
}
// The track sits beside the viewport, so forward wheel input that lands on it.
function wheel(event: WheelEvent) {
  if (!viewport.value) return
  viewport.value.scrollBy({ top: event.deltaY, left: event.deltaX })
  event.preventDefault()
}
function pointerUp(event: PointerEvent) {
  if (drag?.pointer === event.pointerId) drag = null
}
function keydown(event: KeyboardEvent, horizontal = false) {
  const view = viewport.value
  if (!view) return
  const extent = horizontal ? view.clientWidth : view.clientHeight
  const moves: Record<string, number> = { [horizontal ? 'ArrowRight' : 'ArrowDown']: 40, [horizontal ? 'ArrowLeft' : 'ArrowUp']: -40, PageDown: extent * .9, PageUp: -extent * .9 }
  let value = horizontal ? view.scrollLeft : view.scrollTop
  if (event.key in moves) value += moves[event.key]!
  else if (event.key === 'Home') value = 0
  else if (event.key === 'End') value = horizontal ? maximumX.value : maximum.value
  else return
  if (horizontal) view.scrollLeft = value
  else view.scrollTop = value
  event.preventDefault()
}
onMounted(async () => {
  await nextTick()
  observer = new ResizeObserver(update)
  if (viewport.value) observer.observe(viewport.value)
  if (inner.value) observer.observe(inner.value)
  measure()
})
onUnmounted(() => {
  observer?.disconnect()
  if (frame) cancelAnimationFrame(frame)
})
</script>

<template>
  <div class="scroll-shell">
    <div :id="viewportId" ref="viewport" class="overlay-viewport" tabindex="0" :aria-label="props.label" @scroll.passive="update">
      <div ref="inner" class="overlay-inner"><slot /></div>
    </div>
    <div v-if="maximumX > 1" ref="horizontalTrack" class="overlay-track horizontal-track" role="scrollbar" tabindex="0"
         :aria-label="t('components.overlayScroll.horizontal', { label: props.label })" aria-orientation="horizontal" :aria-valuemin="0" :aria-controls="viewportId"
         :aria-valuemax="Math.round(maximumX)" :aria-valuenow="Math.round(currentX)"
         @pointerdown="pointerDown($event, true)" @pointermove="pointerMove" @pointerup="pointerUp"
         @lostpointercapture="drag = null" @keydown="keydown($event, true)" @wheel="wheel">
      <div class="overlay-thumb horizontal-thumb" :style="{width: `${thumbWidth}px`, transform: `translateX(${thumbLeft}px)`}" />
    </div>
    <div v-if="needed" ref="track" class="overlay-track" role="scrollbar" tabindex="0"
         :aria-label="t('components.overlayScroll.vertical', { label: props.label })" aria-orientation="vertical" :aria-valuemin="0"
         :aria-controls="viewportId"
         :aria-valuemax="Math.round(maximum)" :aria-valuenow="Math.round(current)"
         @pointerdown="pointerDown" @pointermove="pointerMove" @pointerup="pointerUp"
         @lostpointercapture="drag = null" @keydown="keydown" @wheel="wheel">
      <div class="overlay-thumb" :style="{height: `${thumbHeight}px`, transform: `translateY(${thumbTop}px)`}" />
    </div>
  </div>
</template>

<style scoped>
.scroll-shell { position:relative; display:flex; flex-direction:column; min-width:0; min-height:0; overflow:hidden; }
.overlay-viewport { flex:1; min-height:0; min-width:0; overflow:auto; scrollbar-width:none; overscroll-behavior:contain; padding-right:14px; padding-bottom:14px; }
.overlay-viewport::-webkit-scrollbar { display:none; }
.overlay-inner { min-width:0; display:flow-root; }
.overlay-track { position:absolute; top:4px; right:2px; bottom:4px; width:10px; touch-action:none; border-radius:5px; cursor:default; }
.overlay-thumb { position:absolute; top:0; right:2px; width:6px; border-radius:4px; background:var(--muted,#64748b); opacity:.65; pointer-events:auto; }
.overlay-track:hover .overlay-thumb,.overlay-track:focus-visible .overlay-thumb { opacity:1; }
.overlay-track:focus-visible { outline:2px solid var(--accent,#2563eb); outline-offset:-1px; }
.horizontal-track { top:auto; left:4px; right:14px; bottom:2px; width:auto; height:10px; }
.horizontal-thumb { top:2px; left:0; right:auto; height:6px; }
</style>
