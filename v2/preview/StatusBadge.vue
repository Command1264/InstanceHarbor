<script setup lang="ts">
import { computed } from 'vue'
import circle from '../../docs/design/v2-wireframe/assets/lucide/circle.svg?raw'
import dot from '../../docs/design/v2-wireframe/assets/lucide/circle-dot.svg?raw'
import dashed from '../../docs/design/v2-wireframe/assets/lucide/circle-dashed.svg?raw'
import failure from '../../docs/design/v2-wireframe/assets/lucide/circle-x.svg?raw'
import unknown from '../../docs/design/v2-wireframe/assets/lucide/circle-help.svg?raw'

const props = defineProps<{ status: string }>()
const states: Record<string, [string, string]> = {
  running: ['正在監聽', dot], stopped: ['已停止', circle],
  starting: ['啟動中', dashed], stopping: ['停止中', dashed],
  failed: ['啟動失敗', failure], unknown: ['尚未核對', unknown],
}
const state = computed(() => states[props.status] ?? ['未知', unknown])
const icon = computed(() => state.value[1].replace('<svg', '<svg class="status-icon" aria-hidden="true" focusable="false"'))
</script>

<template>
  <span :class="['badge', status]">
    <span class="preview-icon" aria-hidden="true" v-html="icon"></span>
    <span class="status-text">{{ state[0] }}</span>
  </span>
</template>

<style scoped>
.preview-icon { display: contents; }
</style>
