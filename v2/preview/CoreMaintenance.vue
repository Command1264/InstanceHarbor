<script setup lang="ts">
import type { InstanceRow } from './InstanceRows.vue'

defineProps<{
  row: InstanceRow & { kind: string; checked?: string }
  connected: boolean
  update: boolean
}>()
</script>

<template>
  <p class="hint">Port {{ row.port }}　／　{{ row.kind }}</p>
  <dl class="core-facts">
    <dt>目前</dt><dd>{{ row.core }} {{ row.version }} · build {{ row.build }}</dd>
    <dt>最新</dt><dd id="core-latest">{{ row.core }} {{ row.version }} · build {{ row.latest }}</dd>
    <dt>核對時間</dt><dd id="core-time">{{ row.checked || '2026/11/2 14:30:00' }}</dd>
    <dt>更新狀態</dt><dd id="core-status">{{ update ? '有更新可用' : '已是已知最新 build' }}</dd>
  </dl>
  <div class="pairs">
    <button type="button" data-action="core-update" :data-id="row.id" class="primary" :disabled="!connected || !update || row.status === 'unknown'">{{ row.core === 'Spigot' ? '選擇本機核心' : '更新核心' }}</button>
    <button type="button" data-action="core-check" :data-id="row.id" :disabled="!connected">檢查更新</button>
    <button type="button" data-action="backups" :data-id="row.id">備份與診斷</button>
    <button type="button" data-action="close-dialog">關閉</button>
  </div>
</template>
