<script setup lang="ts">
import StatusBadge from './StatusBadge.vue'
import type { InstanceRow } from './InstanceRows.vue'

defineProps<{
  rows: InstanceRow[]
  detail?: {
    row: InstanceRow & { checked?: string; path: string }
    blocked: boolean
    emergency: boolean
    consoleDisabled: boolean
    connected: boolean
    phase: string
    power: string
    reason: string
    pinIndex: number
    pinCount: number
    expanded: boolean
    recent: { text: string; tooltip: string; recordId?: string }
  }
}>()
</script>

<template>
  <template v-if="!rows.length">
    <h2>選取實例</h2>
    <p class="hint">選一台查看資訊與操作，或按 Ctrl／Shift 選取多台。</p>
  </template>
  <template v-else-if="!detail">
    <div>
      <h2>已選取 {{ rows.length }} 台實例</h2>
      <p class="hint">{{ rows.filter(row => row.status === 'running').length }} 台正在監聽 · {{ rows.filter(row => row.status === 'stopped').length }} 台已停止<span v-if="rows.some(row => !['running', 'stopped'].includes(row.status))"> · {{ rows.filter(row => !['running', 'stopped'].includes(row.status)).length }} 台需查看狀態</span></p>
    </div>
    <div class="actions primary-detail"><button type="button" data-action="start" class="primary">啟動所選</button><button type="button" data-action="stop">正常停止所選</button></div>
    <div class="actions secondary-detail"><button type="button" data-action="restart">重新啟動所選</button><button type="button" data-action="selected-core">更新所選核心</button></div>
    <details class="selected-targets"><summary>查看所選目標</summary><p v-for="row in rows" :key="row.id" class="small">{{ row.name }} <StatusBadge :status="row.status"/></p></details>
    <p class="hint">只處理所選實例；每台依狀態執行，未知或衝突個別回報。</p>
  </template>
  <template v-else>
    <header class="detail-identity">
      <h2>{{ detail.row.name }}</h2>
      <button type="button" data-action="pin" class="quiet pin-control" :aria-label="detail.row.pin === null ? '釘選實例' : '取消釘選'"
              :title="detail.row.pin === null ? '釘選實例' : '取消釘選'" :aria-pressed="detail.row.pin !== null">
        <svg class="pin" aria-hidden="true" viewBox="0 0 16 20"><path d="M5 2h6M6 2v6l-3 4h10l-3-4V2M8 12v7" fill="none" stroke="currentColor" stroke-width="1.8"/></svg>
      </button>
    </header>
    <div class="detail-core-line">
      <span class="hint">{{ detail.row.core }} {{ detail.row.version }} · Port {{ detail.row.port }}</span>
      <button type="button" data-action="core" class="quiet">核心維護</button>
    </div>
    <div class="observation-line"><span class="observation-label hint">觀察狀態</span><StatusBadge :status="detail.row.status"/></div>
    <p class="hint checked-line">最後核對 <span id="detail-checked">{{ detail.row.checked || '2026/11/2 14:30:00' }}</span></p>
    <p v-if="String(detail.row.build) !== String(detail.row.latest)" class="update-inline">有更新 · build {{ detail.row.build }} → {{ detail.row.latest }}</p>
    <div class="actions primary-detail">
      <button :key="detail.row.id + ':' + detail.power" type="button" :data-action="detail.power" data-primary-power
              :class="detail.emergency ? 'danger' : 'primary'"
              :disabled="detail.blocked && !detail.emergency"
              :aria-describedby="detail.blocked && !detail.emergency ? 'detail-operation-reason' : undefined">{{ detail.phase }}</button>
      <button type="button" data-action="console" :disabled="detail.consoleDisabled" :title="detail.consoleDisabled ? '需要已核對且正在運作的實例' : undefined">開啟原生後台</button>
    </div>
    <div class="actions secondary-detail">
      <button type="button" data-action="restart" :disabled="detail.blocked">重新啟動</button>
      <button type="button" data-action="instance-settings">實例設定</button>
      <details class="action-menu management-actions">
        <summary>管理操作<span aria-hidden="true"> ▾</span></summary>
        <div class="all-actions-panel" role="group" aria-label="此實例管理操作">
          <template v-if="detail.row.pin !== null">
            <button type="button" data-action="pin-up" :disabled="detail.pinIndex <= 0">釘選上移</button>
            <button type="button" data-action="pin-down" :disabled="detail.pinIndex < 0 || detail.pinIndex === detail.pinCount - 1">釘選下移</button>
          </template>
          <button type="button" data-action="unmanage" class="danger">移除管理</button>
        </div>
      </details>
    </div>
    <p v-if="detail.reason" id="detail-operation-reason" class="hint">{{ detail.reason }}</p>
    <section class="recent-detail">
      <span class="hint">最近操作</span>
      <div>
        <p class="recent-text" :title="detail.recent.tooltip">{{ detail.recent.text }}</p>
        <button type="button" data-action="record" :data-id="detail.recent.recordId" class="quiet"
                :disabled="!detail.recent.recordId" :title="detail.recent.recordId ? undefined : '尚無本輪操作紀錄'">查看結果</button>
      </div>
    </section>
    <details class="detail-section diagnostics" :open="detail.expanded">
      <summary>工作階段與診斷資料</summary>
      <dl><dt>Port 查核</dt><dd>未發現重複（示範）</dd><dt>宿主身分</dt><dd>{{ detail.connected ? '已核對示範身分' : '尚未重新核對' }}</dd></dl>
      <p class="mono">實例：{{ detail.row.id }}<br>run：demo-{{ detail.row.id }}<br>程序：{{ detail.row.status === 'running' ? '示範 PID 12345' : '未啟動' }}<br>位置：{{ detail.row.path }}</p>
      <div class="actions">
        <button type="button" data-action="session">核對工作階段</button>
        <button type="button" data-action="logs">查看啟動紀錄</button>
        <button type="button" data-action="log-folder" title="離線預覽不開啟本機目錄">開啟紀錄資料夾</button>
      </div>
      <p class="hint">最近核對的示範事實，不是真實程序資料；正在監聽不代表玩家已確認能登入。</p>
    </details>
  </template>
</template>
