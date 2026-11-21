<script setup lang="ts">
defineProps<{
  choices: [string, string, string][]
  invalid: { label: string; path: string; exists: boolean }[]
  pending: string
  message: string
}>()
</script>

<template>
  <p class="hint">選擇只更新設定草稿；回到設定頁儲存後才保留。下列皆為示範資料。</p>
  <div class="actions">
    <button type="button" data-action="java-refresh">重新探索</button>
    <button type="button" data-action="java-browse">選擇 java.exe</button>
  </div>
  <div class="button-list" role="group" aria-label="可用 Java 安裝">
    <button v-for="[label, path, source] in choices" :key="path" type="button" :data-java="path" :aria-pressed="pending === path">
      {{ label }}<small>{{ source }}　{{ path }}</small>
    </button>
  </div>
  <details class="java-invalid">
    <summary>失效或無法使用的安裝（{{ invalid.length }}）</summary>
    <button v-for="item in invalid" :key="item.path" type="button" disabled>
      {{ item.label }}<small>{{ item.path }}</small><small>{{ item.exists ? '存在但無法執行' : '檔案已不存在' }}</small>
    </button>
  </details>
  <p id="java-pending-result" class="hint" role="status">{{ message }}</p>
</template>
