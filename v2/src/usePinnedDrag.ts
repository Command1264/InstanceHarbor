import {onUnmounted,ref,watch} from 'vue'
import {PinDragIntent,pinInsertionSlot,reorderPinsAtSlot} from './pin-order'

export function usePinnedDrag(options:{pins:()=>string[];visible:()=>string[];enabled:()=>boolean;commit:(pins:string[],expected:string[])=>void}){
 const dragging=ref(''),preview=ref<string[]>([])
 let gesture:{intent:PinDragIntent;name:string;pointer:number;source:HTMLElement;table:HTMLTableElement;viewport:HTMLElement;pins:string[];rows:{name:string;top:number;bottom:number}[];scrollTop:number;left:number;width:number;offsetX:number;offsetY:number;slot:number|null;ghost?:HTMLElement;shield?:HTMLElement}|undefined
 let frame=0,suppressName='',x=0,y=0
 function clear(){
  const old=gesture;gesture=undefined;dragging.value='';preview.value=[]
  cancelAnimationFrame(frame);frame=0;old?.ghost?.remove();old?.shield?.remove()
  document.documentElement.classList.remove('pin-drag-active')
  if(old?.source.hasPointerCapture(old.pointer))old.source.releasePointerCapture(old.pointer)
 }
 function target(){
  const g=gesture;if(!g)return
  const bounds=g.viewport.getBoundingClientRect(),shift=g.viewport.scrollTop-g.scrollTop
  if(!g.rows.length || x<Math.max(bounds.left,g.left) || x>Math.min(bounds.right,g.left+g.width) || y<bounds.top || y>bounds.bottom){
   // Leaving the list keeps the last valid slot; only an explicit cancel resets it.
   return
  }
  g.slot=pinInsertionSlot(g.rows,g.name,y-g.offsetY+shift,g.slot??g.pins.indexOf(g.name))
  const next=reorderPinsAtSlot(g.pins,g.name,g.slot)
  if(JSON.stringify(next)!==JSON.stringify(preview.value))preview.value=next
 }
 function positionGhost(){
  const g=gesture;if(!g?.ghost)return
  g.ghost.style.left=Math.max(0,Math.min(window.innerWidth-g.width,x-g.offsetX))+'px'
  g.ghost.style.top=y-g.offsetY+'px'
 }
 function makeGhost(){
  const g=gesture;if(!g)return
  const row=g.source.closest('tr')!,bounds=row.getBoundingClientRect()
  // Catch pointer hit-testing while keyed rows move; do not hover underlying controls.
  const shield=document.createElement('div');shield.className='pin-drag-shield';shield.setAttribute('aria-hidden','true')
  document.body.append(shield);g.shield=shield;document.documentElement.classList.add('pin-drag-active')
  const ghost=document.createElement('div');ghost.className='pin-drag-ghost';ghost.setAttribute('aria-hidden','true');ghost.inert=true
  ghost.style.width=bounds.width+'px';ghost.style.height=bounds.height+'px'
  const table=document.createElement('table');table.className='instances';table.style.tableLayout='fixed';table.style.height='100%'
  const group=document.createElement('tbody'),clone=row.cloneNode(true) as HTMLTableRowElement
  clone.classList.remove('pin-drag-source');clone.removeAttribute('tabindex');clone.removeAttribute('data-instance')
  for(const element of Array.from(clone.querySelectorAll('[id]')))element.removeAttribute('id')
  Array.from(clone.cells).forEach((cell,index)=>{const width=row.cells[index]!.getBoundingClientRect().width;cell.style.width=width+'px';cell.style.maxWidth=width+'px'})
  group.append(clone);table.append(group);ghost.append(table);document.body.append(ghost);g.ghost=ghost;positionGhost()
 }
 function scroll(){
  const g=gesture;if(!g || !dragging.value)return
  const bounds=g.viewport.getBoundingClientRect()
  if(x>=bounds.left && x<=bounds.right){
   if(y<bounds.top+24 && y>=bounds.top)g.viewport.scrollTop-=8
   else if(y>bounds.bottom-24 && y<=bounds.bottom)g.viewport.scrollTop+=8
  }
  target();frame=requestAnimationFrame(scroll)
 }
 function begin(event:PointerEvent,name:string){
  if(event.button!==0 || !event.isPrimary || event.ctrlKey || event.metaKey || event.shiftKey || !options.enabled() || !options.pins().includes(name))return
  if(event.target instanceof Element && event.target.closest('button,input,select,textarea,a,summary,[contenteditable=true]'))return
  const source=event.currentTarget as HTMLElement,table=source.closest('table'),viewport=source.closest<HTMLElement>('.overlay-viewport'),row=source.closest('tr')
  if(!table||!viewport||!row)return
  suppressName='';clear();window.getSelection()?.removeAllRanges();x=event.clientX;y=event.clientY
  const bounds=row.getBoundingClientRect(),pins=[...options.pins()]
  const rows=Array.from(table.querySelectorAll<HTMLTableRowElement>('tr[data-instance]')).filter(item=>pins.includes(item.dataset.instance!)).map(item=>{const rect=item.getBoundingClientRect();return {name:item.dataset.instance!,top:rect.top,bottom:rect.bottom}})
  gesture={intent:new PinDragIntent(performance.now(),x,y),name,pointer:event.pointerId,source,table,viewport,pins,rows,scrollTop:viewport.scrollTop,left:bounds.left,width:bounds.width,offsetX:x-bounds.left,offsetY:y-bounds.top,slot:null}
 }
 function move(event:PointerEvent){
  const g=gesture;if(!g || event.pointerId!==g.pointer)return
  x=event.clientX;y=event.clientY
  if(!dragging.value){
   if(g.intent.move(performance.now(),x,y)!=='dragging')return
   // Clone the real row before hiding its original content in the placeholder.
   makeGhost();g.source.setPointerCapture(g.pointer);dragging.value=g.name;frame=requestAnimationFrame(scroll)
  }
  event.preventDefault();positionGhost();target()
 }
 function end(event:PointerEvent){
  const g=gesture;if(!g || event.pointerId!==g.pointer)return
  if(dragging.value){
   x=event.clientX;y=event.clientY;target();suppressName=g.name
   const next=g.slot===null?g.pins:reorderPinsAtSlot(g.pins,g.name,g.slot)
   clear()
   if(JSON.stringify(next)!==JSON.stringify(g.pins))options.commit(next,g.pins)
  }else clear()
 }
 function cancel(){if(dragging.value && gesture)suppressName=gesture.name;clear()}
 function escape(event:KeyboardEvent){if(event.key==='Escape'&&gesture){event.preventDefault();event.stopPropagation();cancel()}}
 function nextPointerDown(){if(!gesture)suppressName=''}
 function consumeClick(name:string){return name===suppressName}
 window.addEventListener('pointermove',move,{passive:false});window.addEventListener('pointerup',end);window.addEventListener('pointercancel',cancel);window.addEventListener('blur',cancel);window.addEventListener('keydown',escape,true)
 watch(()=>JSON.stringify([options.pins(),options.visible(),options.enabled()]),cancel)
 onUnmounted(()=>{clear();window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',end);window.removeEventListener('pointercancel',cancel);window.removeEventListener('blur',cancel);window.removeEventListener('keydown',escape,true)})
 return {dragging,preview,begin,cancel,consumeClick,nextPointerDown}
}
