<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
defineProps<{label:string}>()
const menu=ref<HTMLDetailsElement>(), trigger=ref<HTMLElement>()
function closeOutside(event:PointerEvent){if(menu.value?.open && !menu.value.contains(event.target as Node))menu.value.open=false}
function choose(event:MouseEvent){if((event.target as HTMLElement).closest('button:not(:disabled)') && menu.value)menu.value.open=false}
function escape(event:KeyboardEvent){if(event.key==='Escape'&&menu.value?.open){menu.value.open=false;trigger.value?.focus();event.preventDefault();event.stopPropagation()}}
onMounted(()=>document.addEventListener('pointerdown',closeOutside))
onUnmounted(()=>document.removeEventListener('pointerdown',closeOutside))
</script>
<template><details ref="menu" class="action-menu" @click="choose" @keydown="escape"><summary ref="trigger">{{ label }}</summary><div class="all-actions-panel"><slot /></div></details></template>
