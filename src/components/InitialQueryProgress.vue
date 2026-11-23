<script setup lang="ts">
defineProps<{ label: string }>()
</script>

<template>
  <div class="initial-query-progress">
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
</style>
