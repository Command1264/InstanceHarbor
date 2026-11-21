<script setup lang="ts">
import {nextTick,onMounted,onUnmounted,ref} from 'vue'
const props=defineProps<{x:number;y:number;label:string;items:{id:string;label:string;disabled?:boolean;danger?:boolean}[]}>()
const emit=defineEmits<{(e:'action',id:string):void;(e:'close',restore:boolean):void}>()
const menu=ref<HTMLElement>(),left=ref(props.x),top=ref(props.y)
let alive=true
function buttons(){return Array.from(menu.value?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')??[])}
function key(event:KeyboardEvent){
 if(event.key==='Escape'){event.preventDefault();event.stopPropagation();emit('close',true)}
 else if(event.key==='Tab')emit('close',false)
 else if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
  event.preventDefault();const choices=buttons(),index=choices.indexOf(document.activeElement as HTMLButtonElement)
  const next=event.key==='Home'?0:event.key==='End'?choices.length-1:(index+(event.key==='ArrowDown'?1:-1)+choices.length)%choices.length
  choices[next]?.focus()
 }
}
function outside(event:PointerEvent){if(event.target instanceof Node&&!menu.value?.contains(event.target))emit('close',false)}
function closeOnResize(){emit('close',true)}
onMounted(async()=>{await nextTick();if(!alive||!menu.value)return;const bounds=menu.value.getBoundingClientRect();left.value=Math.max(8,Math.min(props.x,window.innerWidth-bounds.width-8));top.value=Math.max(8,Math.min(props.y,window.innerHeight-bounds.height-8));(buttons()[0]??menu.value).focus();document.addEventListener('pointerdown',outside,true);window.addEventListener('resize',closeOnResize)})
onUnmounted(()=>{alive=false;document.removeEventListener('pointerdown',outside,true);window.removeEventListener('resize',closeOnResize)})
</script>
<template><Teleport to="body"><div ref="menu" class="instance-context-menu" role="menu" tabindex="-1" :aria-label="label" :style="{left:left+'px',top:top+'px'}" @keydown="key" @contextmenu.prevent><p class="hint">{{ label }}</p><button v-for="item in items" :key="item.id" type="button" role="menuitem" :disabled="item.disabled" :class="{danger:item.danger}" @click="emit('action',item.id)">{{ item.label }}</button></div></Teleport></template>
