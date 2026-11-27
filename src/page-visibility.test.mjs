import {test} from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
import {pageHidden,onPageShown} from './page-visibility.ts'

function fakeDocument(state){
  const listeners=new Set()
  return {
    visibilityState:state,
    addEventListener:(type,listener)=>{if(type==='visibilitychange')listeners.add(listener)},
    removeEventListener:(type,listener)=>{if(type==='visibilitychange')listeners.delete(listener)},
    change(next){this.visibilityState=next;for(const listener of [...listeners])listener()},
    listeners,
  }
}

test('a page hidden by fast open reports hidden, a shown page does not',()=>{
  assert.equal(pageHidden(fakeDocument('hidden')),true)
  assert.equal(pageHidden(fakeDocument('visible')),false)
})

test('showing the window again refreshes once, and stopping removes the listener',()=>{
  const page=fakeDocument('visible')
  let shown=0
  const stop=onPageShown(()=>shown++,page)
  page.change('visible')
  assert.equal(shown,0)
  page.change('hidden')
  assert.equal(shown,0)
  page.change('visible')
  assert.equal(shown,1)
  stop()
  assert.equal(page.listeners.size,0)
  page.change('hidden');page.change('visible')
  assert.equal(shown,1)
})

test('every background poll of the main window pauses while the page is hidden',()=>{
  const app=readFileSync(new URL('./App.vue',import.meta.url),'utf8')
  // The 1.5 s list poll, the catalog poll and the log follow skip their work while hidden.
  assert.match(app,/timer=setInterval\(\(\)=>\{\s*if\(pageHidden\(\)\)return/)
  assert.match(app,/catalogTimer = setInterval\(\(\)=>\{if\(!pageHidden\(\)\)refreshCatalog\(\)\}/)
  assert.match(app,/logsFollowTimer=run\?setInterval\(\(\)=>\{\s*if\(pageHidden\(\)\)return/)
  assert.match(app,/onPageShown\(/)
  const dialog=readFileSync(new URL('./components/CoreMaintenanceDialog.vue',import.meta.url),'utf8')
  assert.match(dialog,/if \(!pageHidden\(\) && dialog\.value\?\.open/)
  const settings=readFileSync(new URL('./SettingsPage.vue',import.meta.url),'utf8')
  assert.match(settings,/setInterval\(\(\) => \{ if \(!pageHidden\(\)\) void readWorldLive\(\) \}, 2000\)/)
})
