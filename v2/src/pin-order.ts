export function reorderPins(pins: readonly string[], source: string, target: string, after: boolean): string[] {
  if(source===target || !pins.includes(source) || !pins.includes(target))return [...pins]
  const next=pins.filter(name=>name!==source)
  next.splice(next.indexOf(target)+(after?1:0),0,source)
  return next
}

export function validPinOrder(current: readonly string[], proposed: readonly string[], expected: readonly string[]): boolean {
  return JSON.stringify(current)===JSON.stringify(expected) && current.length===proposed.length &&
    new Set(proposed).size===proposed.length && proposed.every(name=>current.includes(name))
}

export class PinDragIntent {
  private readonly x: number
  private readonly y: number
  constructor(_now:number,x:number,y:number){this.x=x;this.y=y}
  move(_now:number,x:number,y:number):'waiting'|'dragging'{
    const distance=Math.hypot(x-this.x,y-this.y)
    return distance>6?'dragging':'waiting'
  }
}

// Removing the source closes its old gap. Crossing a remaining row's midpoint
// with the dragged row's top means half of that target has been covered.
export function pinInsertionSlot(rows:readonly {name:string;top:number;bottom:number}[],source:string,dragTop:number,currentSlot=rows.findIndex(row=>row.name===source)):number{
  const sourceRow=rows.find(row=>row.name===source)
  if(!sourceRow)return currentSlot
  const height=sourceRow.bottom-sourceRow.top
  let slot=0
  for(const row of rows){
    if(row.name===source)continue
    const midpoint=(row.top+row.bottom)/2-(row.top>sourceRow.top?height:0)
    // At exactly 50%, retain the previous slot instead of oscillating.
    if(Math.abs(dragTop-midpoint)<0.001)return Math.max(slot,Math.min(currentSlot,slot+1))
    if(dragTop>midpoint)slot++
  }
  return slot
}
export function reorderPinsAtSlot(pins:readonly string[],source:string,slot:number):string[]{
  if(!pins.includes(source)||!Number.isInteger(slot)||slot<0||slot>=pins.length)return [...pins]
  const next=pins.filter(name=>name!==source);next.splice(slot,0,source);return next
}
