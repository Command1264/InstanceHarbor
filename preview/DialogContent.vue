<script setup lang="ts">
defineProps<{ titleId: string; title: string; body: string; actions: string }>()
</script>

<template>
  <h2 :id="titleId" v-html="title"></h2>
  <div class="dialog-body" v-html="body"></div>
  <div class="dialog-actions" v-html="actions"></div>
</template>
