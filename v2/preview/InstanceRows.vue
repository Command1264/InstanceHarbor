<script setup lang="ts">
import StatusBadge from './StatusBadge.vue'

export interface InstanceRow {
  id: string
  name: string
  project: string
  core: string
  version: string
  port: number
  status: string
  build: string | number
  latest: string | number
  pin: number | null
  last?: string
}

defineProps<{ rows: InstanceRow[]; selected: Set<string>; anchor: string | null }>()
</script>

<template>
  <tr v-for="(row, index) in rows" :key="row.id"
      :data-row="row.id" :data-rowselect="row.id"
      :tabindex="anchor === row.id || (!anchor && index === 0) ? 0 : -1"
      :aria-selected="selected.has(row.id)" :class="{ selected: selected.has(row.id) }">
    <td>
      <span class="name-label">
        <svg v-if="row.pin !== null" class="pin" aria-hidden="true" viewBox="0 0 16 20">
          <path d="M5 2h6M6 2v6l-3 4h10l-3-4V2M8 12v7" fill="none" stroke="currentColor" stroke-width="1.8"/>
        </svg>{{ row.name }}
      </span>
      <span class="sub">{{ row.project }}<template v-if="String(row.build) !== String(row.latest)">　build {{ row.build }} → {{ row.latest }}</template></span>
    </td>
    <td class="core-column">{{ row.core }}<span class="sub">{{ row.version }}</span></td>
    <td class="center">{{ row.port }}</td>
    <td class="center"><StatusBadge :status="row.status"/></td>
    <td>{{ row.last || '尚無操作' }}</td>
  </tr>
</template>
