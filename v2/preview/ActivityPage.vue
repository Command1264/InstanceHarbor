<script setup lang="ts">
defineProps<{
  core: boolean
  records: { id: string; action: string; targets: string; time: string; result: string; progress?: number }[]
}>()
</script>

<template>
  <header class="page-head">
    <div><h1>{{ core ? '近期核心更新' : '操作紀錄' }}</h1><p>離開頁面不取消工作；結果不寫入底部連線狀態列</p></div>
    <div class="actions">
      <button type="button" data-action="all-records">全部紀錄</button>
      <button type="button" data-action="recent-core">近期核心更新</button>
      <button type="button" data-action="cancel-remaining">取消全部剩餘工作</button>
      <button type="button" data-action="export-records" :disabled="!records.length">匯出目前紀錄</button>
      <a href="./activity-review.html" target="_blank" rel="noopener">保留期限與匯出示意</a>
    </div>
  </header>
  <section class="sheet">
    <div v-if="!records.length" class="empty">尚無示範操作。可回到實例頁啟動一台，或試用核心更新。</div>
    <article v-for="record in records" :key="record.id" class="operation-row">
      <div><strong>{{ record.action }}</strong><p class="hint">{{ record.targets }}</p><p class="hint">{{ record.time }}</p></div>
      <div><p>{{ record.result }}</p><progress v-if="record.progress !== undefined" max="100" :value="record.progress" :aria-label="record.action + '進度'"></progress></div>
      <button type="button" data-action="record" :data-id="record.id">查看結果</button>
    </article>
  </section>
</template>
