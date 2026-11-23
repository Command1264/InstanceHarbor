import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { createServer } from 'node:net'
import { createHash } from 'node:crypto'
import { once } from 'node:events'
import { setTimeout as delay } from 'node:timers/promises'

if(process.argv.length!==4)throw Error('Usage: node verify-native-file-access.mjs <binary-directory> <isolated-run>')
const evidence=resolve(process.argv[3])
const report=JSON.parse(readFileSync(join(evidence,'cli-results.json'),'utf8'))
const workspace=join(evidence,'workspace')
assert.equal(workspace.toLowerCase(),report.workspace.toLowerCase())
const guiPath=join(resolve(process.argv[2]),'InstanceHarbor.exe')
const cliPath=join(resolve(process.argv[2]),'ihctl.exe')
const prefs=join(workspace,'preferences.json'), registry=join(workspace,'registry','probe-broken.json')
const sha=path=>createHash('sha256').update(readFileSync(path)).digest('hex')
const expectedHashes=[sha(prefs),sha(registry)]
const expectedOpacity=String(JSON.parse(readFileSync(prefs,'utf8')).selectionOpacity)
assert.equal(sha(cliPath),report.cliSha256.toLowerCase())
const appdata=join(evidence,'appdata')
const profile=join(evidence,'webview')
mkdirSync(profile,{recursive:true})
const ps=script=>execFileSync('pwsh',['-NoProfile','-NonInteractive','-Command',script],{encoding:'utf8',windowsHide:true,timeout:15000}).trim()
const quote=s=>"'"+s.replaceAll("'","''")+"'"
const canonical=s=>s.replace(/^\\\\\?\\/,'').toLowerCase()
const mains=ps("(Get-CimInstance Win32_Process -Filter \"Name='InstanceHarbor.exe' OR Name='instanceharbor-acceptance.exe'\"|Where-Object {$_.CommandLine -notmatch '\\s+native-console$'}).ProcessId")
assert(!mains,'Another InstanceHarbor main window is open')
const server=createServer()
await new Promise(ok=>server.listen(0,'127.0.0.1',ok))
const {port}=server.address()
await new Promise(ok=>server.close(ok))
const env={...process.env,IH_WORKSPACE:workspace,LOCALAPPDATA:appdata,WEBVIEW2_USER_DATA_FOLDER:profile,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-address=127.0.0.1 --remote-debugging-port='+port}
const cli=(...args)=>JSON.parse(execFileSync(cliPath,[workspace,...args],{env,encoding:'utf8',windowsHide:true,timeout:12000}))
const results={guiSha256:sha(guiPath),cliSha256:sha(cliPath),cases:{}}
let gui,socket,send
async function until(fn,message,timeout=17000){
  const end=Date.now()+timeout
  do{const found=await fn();if(found)return found;await delay(120)}while(Date.now()<end)
  throw Error(message)
}
async function open(){
  gui=spawn(guiPath,[],{env,windowsHide:true,stdio:'ignore'})
  const tab=await until(async()=>{
    if(gui.exitCode!==null)throw Error('The isolated GUI exited early')
    try{return (await(await fetch('http://127.0.0.1:'+port+'/json/list',{signal:AbortSignal.timeout(700)})).json()).find(t=>t.type==='page'&&t.url.includes('tauri.localhost'))}
    catch{return undefined}
  },'Isolated WebView unavailable')
  socket=new WebSocket(tab.webSocketDebuggerUrl)
  await new Promise((ok,no)=>{socket.onopen=ok;socket.onerror=no})
  let id=0
  send=(method,params={})=>new Promise((ok,no)=>{
    const expected=++id
    const timer=setTimeout(()=>{socket.removeEventListener('message',listener);no(Error(method+' timed out'))},4500)
    function listener(event){
      const reply=JSON.parse(event.data)
      if(reply.id!==expected)return
      clearTimeout(timer)
      socket.removeEventListener('message',listener)
      if(reply.error)no(Error(JSON.stringify(reply.error)))
      else ok(reply.result)
    }
    socket.addEventListener('message',listener)
    socket.send(JSON.stringify({id:expected,method,params}))
  })
}
async function close(){
  socket?.close()
  socket=undefined
  if(gui?.exitCode===null){
    const pid=gui.pid
    assert.equal(canonical(ps('(Get-Process -Id '+pid+').Path')),canonical(guiPath))
    assert.match(ps('(Get-Process -Id '+pid+').CloseMainWindow()'),/true/i)
    await until(()=>gui.exitCode!==null,'GUI did not close')
  }
}
const evaluate=async (fn,...args)=>{
  const reply=await send('Runtime.evaluate',{expression:'('+fn.toString()+')('+args.map(x=>JSON.stringify(x)).join(',')+')',returnByValue:true})
  assert(!reply.exceptionDetails,JSON.stringify(reply.exceptionDetails))
  return reply.result.value
}
const snapshot=()=>evaluate(()=>({
  rows:[...document.querySelectorAll('tr[data-instance]')].map(row=>row.dataset.instance).sort(),
  selected:[...document.querySelectorAll('tr[aria-selected=true]')].map(row=>row.dataset.instance),
  theme:document.documentElement.dataset.theme,
  opacity:document.documentElement.dataset.selectionOpacity,
  status:document.querySelector('.statusbar')?.innerText,
  error:document.querySelector('dialog[open] #failure-message')?.innerText,
  text:document.body.innerText,
  healthyStartDisabled:[...document.querySelectorAll('aside button')].find(b=>b.textContent.trim()==='啟動')?.disabled
}))
const click=label=>until(()=>evaluate(label=>{
  const item=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===label&&b.getClientRects().length&&!b.disabled)
  if(!item)return false
  item.click()
  return true
},label),'Button unavailable: '+label)
const screenshot=async name=>writeFileSync(join(evidence,name+'.png'),Buffer.from((await send('Page.captureScreenshot')).data,'base64'))
async function held(path,run){
  const holder=spawn('pwsh',['-NoProfile','-NonInteractive','-Command',
    '$ErrorActionPreference="Stop";$f=[IO.File]::Open('+quote(path)+',[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);try{[Console]::WriteLine("LOCKED");[void][Console]::ReadLine()}finally{$f.Dispose()}'
  ],{windowsHide:true,stdio:['pipe','pipe','pipe']})
  const done=once(holder,'exit')
  try{
    let ready=''
    holder.stdout.on('data',data=>{ready+=data})
    await until(()=>ready.includes('LOCKED')||holder.exitCode!==null,'Windows lock holder did not respond')
    assert(ready.includes('LOCKED'))
    return await run()
  }finally{
    holder.stdin.end('\n')
    const [code]=await done
    assert.equal(code,0)
  }
}
async function normal(){
  return until(async()=>{const s=await snapshot();return s.status?.includes('已連線')&&s.rows?.length===2&&s.theme==='dark'&&s},'Healthy snapshot not restored')
}
const compact=({text,...rest})=>rest
try{
  assert(Array.isArray(cli('manager-connect')))
  results.residentManagerStarted=true
  await open()
  await normal()
  await evaluate(()=>document.querySelector('tr[data-instance=probe-healthy]').click())
  await held(prefs,async()=>{
    await click('重新核對')
    const s=await until(async()=>{const s=await snapshot();return s.status?.includes('保留上次設定')&&s},'No warm read failure indicator')
    assert.equal(s.theme,'dark')
    assert.equal(s.opacity,expectedOpacity)
    assert.equal(s.rows.length,2)
    assert.deepEqual(s.selected,['probe-healthy'])
    await screenshot('warm-preferences-locked')
    results.cases.preferencesWarm=compact(s)
  })
  await until(async()=>{const s=await snapshot();return s.theme==='dark'&&s.status?.includes('已連線')&&!s.status.includes('外觀設定')},'Preferences did not recover')
  await close()
  await held(prefs,async()=>{
    await open()
    const s=await until(async()=>{const s=await snapshot();return s.rows.length===2&&s.status?.includes('外觀設定尚未讀取')&&s},'Cold preference failure was not identified')
    results.cases.preferencesCold=compact(s)
    await screenshot('cold-preferences-locked')
  })
  await normal()
  await evaluate(()=>document.querySelector('tr[data-instance=probe-healthy]').click())
  await held(registry,async()=>{
    const s=await until(async()=>{const s=await snapshot();return s.rows.length===1&&s.text.includes('檔案被占用')&&s},'No registry lock warning')
    assert.deepEqual(s.rows,['probe-healthy'])
    assert.deepEqual(s.selected,['probe-healthy'])
    assert(s.status.includes('已連線'))
    assert(s.text.includes('probe-broken.json'))
    results.cases.registryWarmLocked=compact(s)
    await screenshot('warm-registry-locked')
  })
  await normal()
  await close()
  const original=readFileSync(registry)
  try{
    writeFileSync(registry,'{ this is invalid JSON')
    await held(prefs,async()=>{
      await open()
      const s=await until(async()=>{const s=await snapshot();return s.rows.length===1&&s.status?.includes('外觀設定尚未讀取')&&s.text.includes('登記內容無效')&&s},'Combined faults were not independently shown')
      assert.deepEqual(s.rows,['probe-healthy'])
      assert(s.status.includes('已連線'))
      assert(s.text.includes('probe-broken.json'))
      results.cases.combinedCold=compact(s)
      await screenshot('cold-combined-faults')
      writeFileSync(registry,original)
      const repaired=await until(async()=>{const s=await snapshot();return s.rows.length===2&&!s.text.includes('登記內容無效')&&s},'Restored registry is still unavailable')
      assert(repaired.status.includes('外觀設定尚未讀取'))
    })
  }finally{writeFileSync(registry,original)}
  await normal()
  await click('應用程式設定')
  const prior=readFileSync(prefs)
  try{
    const selectTheme=theme=>evaluate(theme=>{
      const field=document.querySelector('#app-theme')
      if(!field)throw Error('Appearance input unavailable')
      field.value=theme
      field.dispatchEvent(new Event('change',{bubbles:true}))
    },theme)
    await selectTheme('light')
    await held(prefs,async()=>{
      await click('儲存設定')
      await until(async()=>(await snapshot()).text.includes('preferences.json 正被其他程序占用'),'Save lock not surfaced')
      assert.equal(await evaluate(()=>document.querySelector('#app-theme').value),'light')
      await screenshot('appearance-draft-preserved')
    })
    assert.equal(sha(prefs),expectedHashes[0])
    cli('save-preferences',JSON.stringify({theme:'dark',pins:[],selectionOpacity:46}))
    await click('儲存設定')
    await until(async()=>(await snapshot()).text.includes('設定已由其他操作變更'),'Stale draft was not rejected')
    assert.equal(await evaluate(()=>document.querySelector('#app-theme').value),'light')
    assert.equal(cli('preferences').selectionOpacity,46)
    await click('重試讀取')
    await click('儲存設定')
    await until(async()=>(await snapshot()).text.includes('設定已儲存'),'Draft could not be saved after recheck')
    const saved=cli('preferences')
    assert.equal(saved.theme,'light')
    assert.equal(saved.selectionOpacity,46)
    results.cases.appearanceDraft={preservedOnSharingViolation:true,conflictRejected:true,unrelatedExternalOpacityPreserved:true}
  } finally {writeFileSync(prefs,prior)}
  await click('實例')
  await click('重新核對')
  await normal()
  assert.deepEqual([sha(prefs),sha(registry)],expectedHashes)
  results.restored=true
}catch(e){
  results.error=e.stack
  if(socket&&gui?.exitCode===null){
    try{results.lastSnapshot=compact(await snapshot());await screenshot('native-probe-failure')}catch(captureError){results.captureError=String(captureError)}
  }
  throw e
}finally{
  try{
    await close()
    const file=join(workspace,'resident-manager.json')
    if(existsSync(file)){
      const pid=JSON.parse(readFileSync(file,'utf8')).pid
      const command=ps("(Get-CimInstance Win32_Process -Filter 'ProcessId="+pid+"').CommandLine")
      if(command){
        assert(command.toLowerCase().includes(workspace.toLowerCase())&&/manager-serve\s*$/.test(command),'Unexpected Manager command')
        const descriptor=ps("$rec=Get-Content -LiteralPath "+quote(file)+" -Raw|ConvertFrom-Json; $p=Get-Process -Id $rec.pid; if($p.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $rec.created){throw 'Manager identity changed'};$p.Path")
        assert.equal(canonical(descriptor),canonical(cliPath))
        assert.equal(cli('manager-exit').exiting,true)
        await until(()=>!ps('(Get-Process -Id '+pid+' -ErrorAction SilentlyContinue).Id'),'Isolated Manager did not exit')
      }
    }
  }finally{writeFileSync(join(evidence,'gui-results.json'),JSON.stringify(results,null,2))}
}
console.log(JSON.stringify(results,null,2))
