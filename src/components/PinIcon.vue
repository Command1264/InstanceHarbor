<!-- Lucide Pin, ISC license; see public/licenses/lucide.txt and assets/icons/sources.json. -->
<script setup lang="ts">
defineProps<{ filled?: boolean }>()
</script>
<template>
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="16" height="16" fill="none"
       stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
       aria-hidden="true" focusable="false">
    <g transform="rotate(45 12 12)">
      <path d="M12 17v5" />
      <path :fill="filled ? 'currentColor' : 'none'" d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
    </g>
  </svg>
</template>
