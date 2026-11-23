import {test} from 'node:test'
import assert from 'node:assert/strict'
import {reorderPins,validPinOrder,PinDragIntent,pinInsertionSlot,reorderPinsAtSlot} from './pin-order.ts'
test('moves only existing pinned names and does not mutate the old order',()=>{
 const pins=['velocity','lobby','play']
 assert.deepEqual(reorderPins(pins,'play','velocity',false),['play','velocity','lobby'])
 assert.deepEqual(reorderPins(pins,'velocity','play',true),['lobby','play','velocity'])
 assert.deepEqual(reorderPins(pins,'other','play',true),pins)
 assert.deepEqual(pins,['velocity','lobby','play'])
})
test('rejects stale snapshots, missing names and duplicates before saving',()=>{
 const pins=['a','b'];assert(validPinOrder(pins,['b','a'],pins))
 assert(!validPinOrder(['a','c'],['b','a'],pins))
 assert(!validPinOrder(pins,['a','a'],pins))
 assert(!validPinOrder(pins,['a'],pins))
})
test('drag starts on movement immediately without a long-press wait',()=>{
 const click=new PinDragIntent(0,10,10);assert.equal(click.move(1,10,10),'waiting')
 assert.equal(click.move(2,12,11),'waiting')
 assert.equal(click.move(3,15,10),'waiting')
 const drag=new PinDragIntent(0,10,10);assert.equal(drag.move(1,20,10),'dragging')
 assert.equal(drag.move(2,30,10),'dragging')
})

test('both sides of one row boundary select the same single insertion slot',()=>{
 const rows=[{name:'a',top:0,bottom:40},{name:'b',top:40,bottom:80},{name:'c',top:80,bottom:120}]
 for(const y of [39,40,41])assert.equal(pinInsertionSlot(rows,'c',y),1)
 assert.deepEqual(reorderPinsAtSlot(['a','b','c'],'c',1),['a','c','b'])
 assert.deepEqual(reorderPinsAtSlot(['a','b','c'],'a',2),['b','c','a'])
 assert.deepEqual(reorderPinsAtSlot(['a','b','c'],'a',-1),['a','b','c'])
 assert.deepEqual(reorderPinsAtSlot(['a','b','c'],'other',1),['a','b','c'])
})

test('adjacent rows exchange only beyond half overlap in either direction',()=>{
 const rows=[{name:'a',top:0,bottom:40},{name:'b',top:40,bottom:80},{name:'c',top:80,bottom:120}]
 assert.equal(pinInsertionSlot(rows,'c',60.4,2),2)
 assert.equal(pinInsertionSlot(rows,'c',60,2),2)
 assert.equal(pinInsertionSlot(rows,'c',59.6,2),1)
 assert.equal(pinInsertionSlot(rows,'a',19.6,0),0)
 assert.equal(pinInsertionSlot(rows,'a',20,0),0)
 assert.equal(pinInsertionSlot(rows,'a',20.4,0),1)
 assert.equal(pinInsertionSlot(rows,'c',60,1),1)
 assert.equal(pinInsertionSlot(rows,'c',60.4,1),2)
 assert.equal(pinInsertionSlot(rows,'c',20,2),1)
})
test('half-overlap uses target row height and removes the dragged row from layout',()=>{
 const rows=[{name:'a',top:0,bottom:40},{name:'b',top:40,bottom:100},{name:'c',top:100,bottom:140}]
 assert.equal(pinInsertionSlot(rows,'c',70.6,2),2)
 assert.equal(pinInsertionSlot(rows,'c',69.4,2),1)
 assert.equal(pinInsertionSlot(rows,'a',29.4,0),0)
 assert.equal(pinInsertionSlot(rows,'a',30.6,0),1)
})

test('pinned order clamps at its last slot when dragging over unpinned rows',()=>{
 const rows=[{name:'a',top:0,bottom:40},{name:'b',top:40,bottom:80},{name:'c',top:80,bottom:120}]
 const slot=pinInsertionSlot(rows,'a',130,0)
 assert.equal(slot,2)
 assert.deepEqual(reorderPinsAtSlot(['a','b','c'],'a',slot),['b','c','a'])
 assert.equal(pinInsertionSlot(rows,'c',-50,2),0)
})
