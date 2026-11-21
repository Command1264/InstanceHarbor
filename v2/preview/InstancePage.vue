<script setup lang="ts">
export interface InstancePageState {
  query: string
  filter: string
  connected: boolean
  scenario: string
  coreQueryLabel?: string
  sort: string
  direction: number
}

defineProps<{ state: InstancePageState; statusNames: Record<string, string> }>()
const columns = [
  ['name', '實例'], ['core', '核心版本'], ['port', 'Port'],
  ['status', '觀察狀態'], ['last', '最近操作'],
]
</script>

<template>
  <header class="page-head">
    <div><h1>實例</h1><p>管理妳的遊玩世界與固定測服</p></div>
    <div class="actions">
      <button type="button" class="primary" data-action="create">建立實例</button>
      <button type="button" data-action="import">匯入已有伺服器</button>
    </div>
  </header>
  <div class="toolbar">
    <label class="sr-only" for="search">搜尋實例</label>
    <input id="search" class="search" name="search" type="search" autocomplete="off"
           placeholder="搜尋名稱、專案、核心、Port 或路徑" :value="state.query">
    <label for="status-filter">狀態</label>
    <select id="status-filter">
      <option value="all">所有狀態</option>
      <option v-for="(name, value) in statusNames" :key="value" :value="value">{{ name }}</option>
    </select>
    <button type="button" data-action="refresh">刷新清單</button>
    <details class="action-menu core-tools">
      <summary aria-label="核心維護"><span data-core-query-label>{{ state.coreQueryLabel || '核心維護' }}</span><span aria-hidden="true"> ▾</span></summary>
      <div class="all-actions-panel" role="group" aria-label="全部實例核心維護">
        <button type="button" data-action="check-all">檢查全部核心更新</button>
        <button type="button" data-action="batch-core">更新可更新核心</button>
        <button type="button" data-action="recent-core">近期核心更新</button>
      </div>
    </details>
    <details class="action-menu all-actions">
      <summary>全部操作<span aria-hidden="true"> ▾</span></summary>
      <div class="all-actions-panel" role="group" aria-label="全部實例操作">
        <button type="button" data-action="all-start">全部啟動</button>
        <button type="button" data-action="all-stop">全部正常停止</button>
        <button type="button" data-action="all-restart">全部重啟</button>
      </div>
    </details>
  </div>
  <div v-if="!state.connected" class="notice error">
    連線中斷。下方保留最後核對資料；需要即時證據的操作暫停。
    <button type="button" data-action="reconnect">重新連線</button>
  </div>
  <div class="workspace">
    <section class="list-panel" aria-label="實例清單">
      <div class="selection-line"><span id="selection-count"></span><span>
        <button v-if="state.scenario === 'stale'" type="button" data-action="clear-stale" class="quiet">清理失效釘選</button>
        <button type="button" data-action="clear" class="quiet">清除選取</button>
      </span></div>
      <div class="table-wrap">
        <table class="instances" role="grid" aria-multiselectable="true" aria-label="伺服器清單，可整列選取">
          <thead><tr>
            <th v-for="[key, label] in columns" :key="key" scope="col"
                :class="{ 'core-column': key === 'core', center: ['port', 'status'].includes(key) }"
                :aria-sort="state.sort === key && state.direction ? (state.direction === 1 ? 'ascending' : 'descending') : 'none'">
              <button type="button" :data-sort="key">{{ label }}<span class="sort-mark" aria-hidden="true">{{ state.sort === key ? (state.direction === 1 ? '↑' : state.direction === -1 ? '↓' : '·') : '' }}</span></button>
            </th>
          </tr></thead>
          <tbody id="instance-rows"></tbody>
        </table>
      </div>
      <div id="list-empty" class="empty" hidden>沒有符合的實例。可以清除篩選，或建立／匯入實例。</div>
    </section>
    <aside id="detail" class="detail-panel" aria-label="所選實例"></aside>
  </div>
</template>
