import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as wait } from 'node:timers/promises'
const root = fileURLToPath(new URL('.', import.meta.url))
const [workspace, expected, name, binaryDirectory, mode] = process.argv.slice(2)
assert.ok(workspace && expected && name)
const exe = join(binaryDirectory ?? join(root, 'src-tauri', 'target', 'debug'), 'InstanceHarbor.exe')
async function until(check, message, ms = 12000) {
  const end = Date.now() + ms
  do {
    const result = await check()
    if (result) return result
    await wait(75)
  } while (Date.now() < end)
  throw new Error(message)
}
async function port() {
  const server = createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const {port} = server.address()
  await new Promise(resolve => server.close(resolve))
  return port
}
async function window(test) {
  const debug = await port()
  const child = spawn(exe, [], {detached:true, windowsHide:true, stdio:'ignore', env:{
    ...process.env, IH_WORKSPACE:workspace, WEBVIEW2_USER_DATA_FOLDER:join(workspace,'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+debug,
  }})
  let launchError
  child.on('error', error=>{ launchError=error })
  let ws
  try {
    const target = await until(async ()=>{
      if (launchError) throw launchError
      if (child.exitCode !== null) throw new Error('Native GUI exited: '+child.exitCode)
      try {
        return (await (await fetch('http://127.0.0.1:'+debug+'/json/list',{signal:AbortSignal.timeout(300)})).json())
          .find(page=>page.type==='page'&&page.url.includes('tauri.localhost'))
      } catch { return null }
    },'Native WebView2 unavailable')
    ws=new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve,reject)=>{ws.onopen=resolve;ws.onerror=reject})
    let id=0
    async function evaluate(expression){
      const request=++id
      const reply=await new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>{ws.removeEventListener('message',receive);reject(new Error('CDP response timed out'))},5000)
        const receive=({data})=>{
          const result=JSON.parse(data)
          if(result.id!==request)return
          clearTimeout(timer);ws.removeEventListener('message',receive);resolve(result)
        }
        ws.addEventListener('message',receive)
        ws.send(JSON.stringify({id:request,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}))
      })
      if(reply.error||reply.result.exceptionDetails)throw new Error(JSON.stringify(reply.error??reply.result.exceptionDetails))
      return reply.result.result.value
    }
    if(!['core-prefetch','instance-diagnostics'].includes(mode)){
      await until(()=>evaluate('Boolean(document.querySelector(".row"))'),'Instance was not loaded')
      await evaluate('document.querySelector(".row").click()')
      assert.equal(await evaluate('document.querySelector("aside h2").textContent'),name)
      assert.equal(await evaluate('document.querySelector("aside code").textContent'),expected)
    }
    async function send(method, params){
      const request=++id
      await new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>{ws.removeEventListener('message',receive);reject(new Error('CDP command timed out'))},5000)
        const receive=({data})=>{
          const reply=JSON.parse(data)
          if(reply.id!==request)return
          clearTimeout(timer);ws.removeEventListener('message',receive)
          if(reply.error)reject(new Error(JSON.stringify(reply.error)));else resolve()
        }
        ws.addEventListener('message',receive)
        ws.send(JSON.stringify({id:request,method,params}))
      })
    }
    return await test(evaluate,child.pid,send)
  } finally {
    ws?.close()
    if(child.pid && child.exitCode===null){
      try{
        execFileSync('pwsh',['-NoProfile','-Command','(Get-Process -Id '+child.pid+').CloseMainWindow()'],{windowsHide:true,stdio:'ignore'})
        await until(()=>child.exitCode!==null,'Window did not close',4000)
      }catch{child.kill();await until(()=>child.exitCode!==null||child.signalCode!==null,'Window did not exit')}
    }
  }
}
if(mode==='instance-diagnostics'){
  const cli = join(binaryDirectory, 'ihctl.exe')
  const options = {proxy:'diagnostics-velocity',route:'paper',plugin:'LuckPerms-Bukkit-5.5.85.jar'}
  const report = JSON.parse(execFileSync(cli,[workspace,'instance-diagnostics',name,JSON.stringify(options)],{encoding:'utf8'}))
  assert.equal(report.runtime.backendRun ?? 'stopped',expected)
  assert.equal(report.configuration.state,'Match')
  assert.equal(report.player.state,'Unverified')
  const result = await window(async (evaluate,pid)=>{
    const row = 'tr[data-instance='+JSON.stringify(name)+']'
    await until(()=>evaluate('Boolean(document.querySelector('+JSON.stringify(row)+'))'),'Instance list unavailable')
    await evaluate('document.querySelector('+JSON.stringify(row)+').click()')
    await until(()=>evaluate('document.querySelector("aside h2")?.textContent === '+JSON.stringify(name)),'Wrong selected instance')
    await evaluate('document.querySelector("aside details").open=true')
    const panel = '.instance-diagnostics'
    await until(()=>evaluate('document.querySelectorAll('+JSON.stringify(panel+' form input, '+panel+' form select')+').length === 3'),'Diagnostics controls unavailable')
    assert.equal(await evaluate('!!document.querySelector(".diagnostic-results")'),false)
    await evaluate('(() => { const form = document.querySelector(".instance-diagnostics form"); const inputs = form.querySelectorAll("input,select"); '+JSON.stringify(Object.values(options))+'.forEach((value,i)=>{inputs[i].value=value; inputs[i].dispatchEvent(new Event("input", {bubbles:true})); inputs[i].dispatchEvent(new Event("change", {bubbles:true}))}); form.querySelector("button").click() })()')
    const badges = await until(async ()=>{
      const states = await evaluate('[...document.querySelectorAll(".diagnostic-results .badge")].map(node=>node.textContent.trim())')
      return states.length === 4 && states
    },'Diagnostic findings did not render')
    assert.deepEqual(badges, ['設定一致',
      report.runtime.state === 'Ready' ? '當次程序已就緒' : '已停止',
      report.plugin.state === 'Enabled' ? '已核對啟用' : '已找到插件檔案',
      '未驗證'])
    const panelText = await evaluate('document.querySelector(".instance-diagnostics").textContent')
    if (report.runtime.backendRun) assert.ok(panelText.includes(report.runtime.backendRun))
    if (report.runtime.proxyRun) assert.ok(panelText.includes(report.runtime.proxyRun))
    return {pid,badges,run:report.runtime.backendRun,guiCliParity:true,manualOnly:true}
  })
  console.log(JSON.stringify(result))
  process.exit(0)
}
if(mode==='core-prefetch'){
  const click=(evaluate,label)=>evaluate(`[...document.querySelectorAll('button,a')].find(b=>b.textContent.trim()===${JSON.stringify(label)})?.click()`)
  const toggle="document.querySelector('input[aria-describedby=core-prefetch-help]')"
  async function settings(evaluate){
    await until(()=>evaluate("[...document.querySelectorAll('button,a')].some(b=>b.textContent.trim()==='應用程式設定')"),'Application settings unavailable')
    await click(evaluate,'應用程式設定')
    await until(()=>evaluate("[...document.querySelectorAll('button')].some(b=>b.textContent.trim()==='核心更新')"),'Core settings tab missing')
    await click(evaluate,'核心更新')
    await until(()=>evaluate(`Boolean(${toggle} && !${toggle}.matches(':disabled'))`),'Core prefetch setting unavailable')
  }
  await window(async evaluate=>{
    await settings(evaluate)
    assert.equal(await evaluate(`${toggle}.checked`),false)
    await evaluate(`${toggle}.click()`)
    await click(evaluate,'儲存設定')
    await until(()=>evaluate("document.querySelector('.settings-footer').textContent.includes('設定已儲存')"),'Prefetch opt-in not saved')
  })
  await window(async evaluate=>{
    await settings(evaluate)
    assert.equal(await evaluate(`${toggle}.checked`),true)
    await evaluate(`${toggle}.click()`)
    await click(evaluate,'儲存設定')
    await until(()=>evaluate("document.querySelector('.settings-footer').textContent.includes('設定已儲存')"),'Prefetch opt-out not saved')
  })
  console.log(JSON.stringify({defaultOff:true,enabledSaved:true,retainedOnReopen:true,disabledSaved:true}))
  process.exit(0)
}
if(mode==='restart-stopped'){
  const result=await window(async (evaluate,_,send)=>{
    await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:1,y:1})
    const styles=[]
    for(const theme of ['light','dark']){
      await send('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme}]})
      const matches=await evaluate(`(() => {
        const [start,stop,restart]=document.querySelectorAll('aside .instance-action')
        const signature=button=>{const s=getComputedStyle(button);return [s.color,s.backgroundColor,s.border,s.opacity].join('|')}
        if(start.disabled || !stop.disabled || restart.disabled || signature(start)!==signature(restart))return false
        restart.disabled=true
        const same=signature(stop)===signature(restart)
        restart.disabled=false
        return same
      })()`)
      assert.ok(matches,theme)
      styles.push(theme)
    }
    await evaluate("[...document.querySelectorAll('aside button')].find(b=>b.textContent==='重新啟動').click()")
    await until(()=>evaluate("document.querySelector('main > .feedback').textContent.includes('重啟已受理')"),'Restart from stopped not accepted')
    return {styles,restartFromStopped:true}
  })
  console.log(JSON.stringify(result))
  process.exit(0)
}
if(mode==='force'){
  const result=await window(async (evaluate,_,send)=>{
    const click=(label,scope='aside')=>evaluate(`[...document.querySelectorAll(${JSON.stringify(scope+' button')})].find(b=>b.textContent.trim()===${JSON.stringify(label)}).click()`)
    async function checkActions(){
      assert.equal(await evaluate(`(() => {
        const dialog=document.querySelector('dialog[open]')
        const actions=dialog.querySelector('.dialog-actions')
        const last=actions.lastElementChild.getBoundingClientRect()
        const bounds=actions.getBoundingClientRect()
        return Math.abs(bounds.right-last.right)<1 && Math.abs(bounds.bottom-last.bottom)<1
      })()`),true)
    }
    for(const label of ['建立實例','匯入已有伺服器','工作紀錄']){
      await click(label,'header + .toolbar')
      await checkActions()
      if(label==='工作紀錄')await until(()=>evaluate("Boolean(document.querySelector('dialog[open] li'))"),'Work history did not load')
      await click(label==='工作紀錄'?'關閉':'取消','dialog[open]')
    }
    const stopBounds=()=>evaluate("JSON.stringify(document.querySelector('.stop-action').getBoundingClientRect().toJSON())")
    const normalBounds=await stopBounds()
    await click('正常停止')
    assert.equal(await stopBounds(),normalBounds)
    await click('緊急停止')
    await checkActions()
    assert.equal(await evaluate('document.activeElement.textContent'),'取消')
    assert.equal(await evaluate('document.querySelector("dialog[open]").getAttribute("aria-describedby")'),'force-description')
    const colors=[]
    for(const theme of ['light','dark']){
      await send('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme}]})
      const contrast=await evaluate(`(() => {
        const button=document.querySelector('dialog[open] .danger-confirm')
        const style=getComputedStyle(button)
        const luminance=color=>color.match(/\\d+/g).slice(0,3).map(Number).map(x=>x/255).map(x=>x<=.04045?x/12.92:((x+.055)/1.055)**2.4).reduce((sum,x,i)=>sum+x*[.2126,.7152,.0722][i],0)
        const values=[luminance(style.color),luminance(style.backgroundColor)].sort((a,b)=>b-a)
        return (values[0]+.05)/(values[1]+.05)
      })()`)
      assert.ok(contrast>=4.5)
      colors.push({theme,contrast})
    }
    await send('Emulation.setFocusEmulationEnabled',{enabled:true})
    await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27})
    await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27})
    await until(()=>evaluate('!document.querySelector("dialog[open]")'),'Escape did not cancel confirmation')
    assert.equal(await evaluate('document.querySelector("aside code").textContent'),expected)
    await click('緊急停止')
    await click('強制停止','dialog[open]')
    await until(()=>evaluate("document.querySelector('main > .feedback').textContent.includes('已強制停止伺服器')"),'Force stop not confirmed')
    await until(()=>evaluate("document.querySelector('aside').textContent.includes('觀察狀態：已停止')"),'Stopped status not shown')
    return {cancelFocus:true,escapeCancelled:true,colors,run:expected,forceConfirmed:true,stableStopBounds:true,rightAlignedDialogs:true}
  })
  console.log(JSON.stringify(result))
  process.exit(0)
}
const first = await window(async (_,pid)=>pid)
const reopened = await window(async (evaluate,pid)=>{
  const result = await evaluate(`new Promise((resolve,reject)=>{
    const feedback=document.querySelector('main > .feedback');
    const button=[...document.querySelectorAll('aside button')].find(b=>b.textContent==='重新啟動');
    if(!button||button.disabled){reject(new Error('Restart unavailable'));return}
    const begin=performance.now();
    const timer=setTimeout(()=>{observer.disconnect();reject(new Error('Manual feedback exceeded 500 ms'))},500);
    const observer=new MutationObserver(()=>{if(feedback.textContent.includes('處理中')){clearTimeout(timer);observer.disconnect();resolve(performance.now()-begin)}});
    observer.observe(feedback,{childList:true,subtree:true,characterData:true});
    button.click(); button.click();
  })`)
  await until(()=>evaluate("document.querySelector('main > .feedback').textContent.includes('已受理')"),'Restart not accepted',65000)
  return {pid,feedbackMs:result}
})
assert.notEqual(first,reopened.pid)
console.log(JSON.stringify({firstPid:first,reopenedPid:reopened.pid,reopenedRun:expected,feedbackMs:reopened.feedbackMs}))
