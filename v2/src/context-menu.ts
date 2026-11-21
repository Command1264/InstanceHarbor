export function keepEditingContext(target:EventTarget|null,point?:{clientX:number;clientY:number}):boolean{
 if(!(target instanceof Element))return false
 if(target.closest('input,textarea,[contenteditable=true]'))return true
 const selection=window.getSelection()
 if(!selection || selection.isCollapsed || !selection.rangeCount)return false
 const range=selection.getRangeAt(0)
 return !!point && range.intersectsNode(target) && Array.from(range.getClientRects()).some(rect=>point.clientX>=rect.left&&point.clientX<=rect.right&&point.clientY>=rect.top&&point.clientY<=rect.bottom)
}
export function suppressBrowserMenu(event:MouseEvent){if(!keepEditingContext(event.target,event))event.preventDefault()}
