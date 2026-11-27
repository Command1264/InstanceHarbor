import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const [directory, mode = 'fixture'] = process.argv.slice(2)
assert.ok(directory && ['fixture', 'paper'].includes(mode), 'Specify an isolated test package')
const evidence = resolve(directory)
const workspace = join(evidence, 'workspace')
const native = join(evidence, 'native')
const cli = join(native, 'ihctl.exe')
const guiPath = join(native, 'InstanceHarbor.exe')
assert.match(evidence, /[\\/]\.evidence[\\/]native-console-[a-z0-9-]+$/i)

mkdirSync(join(evidence, 'appdata'), { recursive: true })
const env = { ...process.env, IH_WORKSPACE: workspace,
  LOCALAPPDATA: join(evidence, 'appdata'), WEBVIEW2_USER_DATA_FOLDER: join(evidence, 'webview') }
function ih(...args) {
  return JSON.parse(execFileSync(cli, [workspace, ...args], { env, encoding:'utf8', windowsHide:true }))
}
async function until(fn, message, timeout = 20000) {
  const end = Date.now() + timeout
  do {
    const value = await fn()
    if (value) return value
    await delay(120)
  } while (Date.now() < end)
  throw new Error(message)
}

function powershell(script) {
  return execFileSync('pwsh',['-NoProfile','-Command',script], { encoding:'utf8', windowsHide:true })
}
function processPath(pid) {
  return powershell('(Get-Process -Id '+Number(pid)+' -ErrorAction SilentlyContinue).Path').trim()
}
function closeWindow(pid, expected) {
  assert.equal(processPath(pid).toLowerCase(), expected.toLowerCase())
  assert.match(powershell('(Get-Process -Id '+Number(pid)+').CloseMainWindow()'), /True/i)
}
function isVisible(pid) {
  return Boolean(powershell('(Get-Process -Id '+Number(pid)+' -ErrorAction SilentlyContinue).MainWindowTitle').trim())
}
const portServer = createServer()
await new Promise(resolve => portServer.listen(0, '127.0.0.1', resolve))
const { port } = portServer.address()
await new Promise(resolve => portServer.close(resolve))
env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port='+port

async function pages() {
  try {
    return await (await fetch('http://127.0.0.1:'+port+'/json/list',{signal:AbortSignal.timeout(500)})).json()
  } catch { return [] }
}
async function connect(page) {
  const ws = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((resolve,reject)=>{ws.onopen=resolve;ws.onerror=reject})
  let sequence=0
  function send(method, params={}) {
    const id=++sequence
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{ws.removeEventListener('message',listener);reject(Error('CDP timeout '+method))},5000)
      function listener({data}) {
        const reply=JSON.parse(data)
        if(reply.id!==id)return
        clearTimeout(timer)
        ws.removeEventListener('message',listener)
        if(reply.error)reject(Error(JSON.stringify(reply.error)))
        else resolve(reply.result)
      }
      ws.addEventListener('message',listener)
      ws.send(JSON.stringify({id,method,params}))
    })
  }
  async function evaluate(expression) {
    const reply=await send('Runtime.evaluate',{expression,returnByValue:true})
    if(reply.exceptionDetails)throw Error(JSON.stringify(reply.exceptionDetails))
    return reply.result.value
  }
  return {
    send, evaluate, close:()=>ws.close(),
    text:()=>evaluate('document.querySelector(".console-panel:not([hidden]) .xterm-rows")?.textContent ?? document.body?.innerText ?? ""'),
    async capture(name) {
      writeFileSync(join(evidence,name+'.png'), Buffer.from((await send('Page.captureScreenshot')).data,'base64'))
    },
    async key(key, code, virtualKey) {
      const fields={key,code,windowsVirtualKeyCode:virtualKey,nativeVirtualKeyCode:virtualKey}
      await send('Input.dispatchKeyEvent',{type:'rawKeyDown',...fields})
      await send('Input.dispatchKeyEvent',{type:'keyUp',...fields})
    },
    async textInput(text) { await send('Input.insertText',{text}) },
  }
}
const managerPath = join(workspace,'resident-manager.json')
const getManager = () => JSON.parse(readFileSync(managerPath,'utf8'))
const rows = ih('list')
assert.ok(rows.every(row=>row.state==='Stopped'))
const name = mode==='paper' ? 'resident-paper-agent' : 'fixture-a'
assert.ok(rows.some(row=>row.name===name))
let previousRun
if (mode==='fixture') {
  ih('start',name,'native-console-previous-start')
  previousRun=await until(()=>{
    const row=ih('status',name)
    return row.state==='Listening'&&row.run
  },'Previous test run did not start')
  assert.equal(ih('stop',name,'native-console-previous-stop',previousRun.id).state,'Stopped')
}
let gui, main, terminal, viewerPid, originalRun, secondRun
let result
try {
  gui=spawn(guiPath,[],{env,windowsHide:true,stdio:'ignore'})
  main=await connect(await until(async()=>{
    if (gui.exitCode!==null) throw Error('Isolated GUI exited early')
    return (await pages()).find(page=>page.type==='page'&&page.url.includes('tauri.localhost')&&!page.url.includes('console.html'))
  },'Main window unavailable'))
  const rowSelector=JSON.stringify('[data-instance="'+name+'"]')
  await until(()=>main.evaluate('!!document.querySelector('+rowSelector+')'),'Registered row missing')
  const button = label => 'Array.from(document.querySelectorAll("aside button")).find(b=>b.textContent.trim()==='+JSON.stringify(label)+')'
  const selectRow = (name, extend = false) => main.evaluate('document.querySelector('+JSON.stringify('[data-instance="'+name+'"]')+').dispatchEvent(new MouseEvent("click",{bubbles:true,ctrlKey:'+extend+'}))')
  if (mode==='fixture') {
    await selectRow(name)
    await selectRow('fixture-b',true)
    const group = await main.evaluate(`Array.from(document.querySelectorAll('.detail-actions button'), b=>{
      const rect=b.getBoundingClientRect()
      const text=document.createRange()
      text.selectNodeContents(b)
      return {label:b.textContent.trim(),x:rect.x,y:rect.y,width:rect.width,
        fits:text.getBoundingClientRect().width<=rect.width,disabled:b.disabled,tooltip:b.parentElement.title}
    })`)
    // Batch buttons append the number of rows they act on, e.g. 啟動所選（2）.
    assert.deepEqual(group.map(item=>item.label.replace(/（\d+）$/,'')),['啟動所選','開啟所選後台','正常停止所選','重新啟動所選'])
    assert.equal(group[0].width,group[1].width)
    assert.equal(group[0].y,group[1].y)
    assert.equal(group[2].y,group[3].y)
    assert.ok(group.every(item=>item.fits))
    assert.ok(group[1].disabled)
    assert.match(group[1].tooltip,/具備後台.*啟動或監聽/)
  }
  await selectRow(name)
  await until(()=>main.evaluate(button('啟動')+'?.disabled===false'),'Start unavailable')
  assert.ok(await main.evaluate(button('啟動')+'.classList.contains("primary")'))
  await main.evaluate(button('啟動')+'.click()')
  const original=await until(()=>{
    const row=ih('status',name)
    return row.state==='Listening'&&row.consoleReady&&row.nativeConsole&&row
  },'ConPTY server not ready',150000)
  originalRun = original.run
  await until(()=>main.evaluate(button('開啟後台')+'?.disabled===false'),'Vue console button unavailable')
  assert.ok(await main.evaluate(button('開啟後台')+'.classList.contains("primary")'))
  const openStartedAt = performance.now()
  await main.evaluate(button('開啟後台')+'.click()')
  const terminalPage=await until(async()=>(await pages()).find(page=>page.url.includes('console.html')),'Terminal WebView missing')
  terminal=await connect(terminalPage)
  const ready=await until(async()=>(await terminal.text()).includes('Done ('),'Startup output missing in xterm')
  const openToFirstOutputMs = Math.round(performance.now() - openStartedAt)
  assert.ok(ready)
  const opened=ih('console-open',name,original.run.id)
  viewerPid=opened.viewerPid
  await until(()=>isVisible(viewerPid),'Terminal window unavailable')
  const firstManager=getManager()
  if (mode === 'paper') await terminal.send('Emulation.setFocusEmulationEnabled',{enabled:true})
  const inputFocused = () => terminal.evaluate('!!document.activeElement?.closest(".console-panel:not([hidden])") && document.activeElement.classList.contains("xterm-helper-textarea")')
  await until(inputFocused,'Terminal input did not receive focus')
  assert.equal(await terminal.evaluate('!!document.querySelector("#terminal-state")'),false)
  const selectedTab = () => terminal.evaluate('document.querySelector("[role=tab][aria-selected=true]")?.textContent')
  const terminalPages = async () => (await pages()).filter(page=>page.url.includes('console.html'))
  async function sendPing() {
    await terminal.evaluate(`(() => {
      const data = new DataTransfer()
      data.setData('text/plain', 'test-ping\\r')
      document.activeElement.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }))
    })()`)
    await until(async()=>(await terminal.text()).includes('IH-PONG'),'Synthetic command did not respond')
  }
  if(mode==='fixture') {
    // Dispatch through xterm's paste listener without depending on which desktop
    // window owns the physical keyboard while the test runs.
    await sendPing()
    await selectRow('fixture-b')
    await until(()=>main.evaluate(button('啟動')+'?.disabled===false'),'Second Start unavailable')
    await main.evaluate(button('啟動')+'.click()')
    secondRun = await until(()=>{
      const row=ih('status','fixture-b')
      return row.state==='Listening'&&row.consoleReady&&row.nativeConsole&&row.run
    },'Second ConPTY server not ready')
    await until(()=>main.evaluate(button('開啟後台')+'?.disabled===false'),'Second console unavailable')
    await selectRow(name,true)
    await selectRow('fixture-c',true)
    await until(()=>main.evaluate(button('開啟所選後台')+'?.disabled===false'),'Batch console unavailable')
    await main.capture('console-batch-actions')
    await main.evaluate(button('開啟所選後台')+'.click()')
    await until(async()=>await selectedTab()==='fixture-b' && await inputFocused(),'Batch did not open the second tab and focus it')
    assert.equal((await terminalPages()).length,1)
    assert.equal(await terminal.evaluate('document.querySelectorAll("[role=tab]").length'),2)
    assert.equal(ih('status','fixture-c').state,'Stopped')
    assert.ok(!(await terminal.text()).includes('IH-PONG'))
    await sendPing()
    await terminal.capture('console-two-tabs')
    await terminal.evaluate('document.querySelector("#tab-'+originalRun.id+'").click()')
    await until(async()=>await selectedTab()===name && await inputFocused(),'Selecting first tab did not restore focus')
    assert.equal((await terminal.text()).split('IH-PONG').length-1,1)
    await selectRow(name)
  } else {
    await terminal.textInput('list')
    await terminal.key('Enter','Enter',13)
    await until(async()=>(await terminal.text()).includes('players online'),'Paper list command did not respond')
    await terminal.textInput('say ')
    // Use WebView2's composition event path before committing Traditional Chinese.
    await terminal.send('Input.imeSetComposition',{text:'繁體中文驗證',selectionStart:6,selectionEnd:6})
    await terminal.textInput('繁體中文驗證')
    await terminal.key('Enter','Enter',13)
    await until(async()=>(await terminal.text()).includes('[Server] 繁體中文驗證'),'Paper did not echo committed Chinese command')
    await terminal.textInput('lis')
    await terminal.key('Enter','Enter',13)
    await until(async()=>(await terminal.text()).includes('Unknown or incomplete command'),'Uncompleted command did not produce the expected Paper error')
    await terminal.textInput('lis')
    await terminal.key('Tab','Tab',9)
    await terminal.key('Enter','Enter',13)
    await until(async()=>(await terminal.text()).split('players online').length===3,'Paper Tab did not complete list')
  }
  await terminal.capture('console-before-close')
  await terminal.evaluate('document.activeElement.blur()')
  assert.equal(await inputFocused(),false)
  closeWindow(viewerPid,guiPath)
  await until(()=>!isVisible(viewerPid),'Closing console did not hide it')
  assert.equal(processPath(firstManager.pid).toLowerCase(),cli.toLowerCase())
  assert.equal(ih('manager-exit').exiting,true)
  await until(()=>!processPath(firstManager.pid),'Original Manager did not exit')
  if(mode==='fixture'){
    writeFileSync(join(workspace,'instances',name,'fixture-console-output'),'')
    await until(async()=>(await terminal.text()).includes('IH-HIDDEN-02'),'Hidden terminal did not parse output')
  }
  await main.evaluate(button('開啟後台')+'.click()')
  await until(()=>isVisible(viewerPid),'Original terminal did not reopen')
  await until(async()=>await selectedTab()===name && await inputFocused(),'Reopening did not restore terminal focus')
  assert.equal((await pages()).find(page=>page.url.includes('console.html')).id,terminalPage.id)
  const secondManager=getManager()
  assert.notEqual(secondManager.pid,firstManager.pid)
  const recovered=ih('status',name)
  assert.deepEqual(recovered.run,original.run)
  const visibleText=await terminal.text()
  const log=readFileSync(join(workspace,'runs',name,'stdout.log'),'utf8')
  if(mode==='fixture'){
    for(const marker of ['IH-PONG','IH-HIDDEN-01','IH-HIDDEN-02']){
      assert.equal(visibleText.split(marker).length-1,1,marker+' duplicated or lost in renderer')
      assert.equal(log.split(marker).length-1,1,marker+' duplicated or lost in Host output')
    }
    const longLine = 'IH-LONG-START'+'0123456789abcdef'.repeat(24)+'IH-LONG-END'
    assert.equal(visibleText.split(longLine).length-1,1,'Long wrapped line was lost, fragmented or duplicated')
    const bLog=readFileSync(join(workspace,'runs','fixture-b','stdout.log'),'utf8')
    assert.equal(bLog.split('IH-PONG').length-1,1)
    assert.ok(!bLog.includes('IH-HIDDEN-02'))
    await terminal.evaluate('document.querySelector("#tab-'+secondRun.id+' + .tab-close").click()')
    await until(async()=>await terminal.evaluate('document.querySelectorAll("[role=tab]").length')===1,'Second tab did not close')
    assert.equal(ih('status','fixture-b').state,'Listening')
    assert.equal(ih('console-open','fixture-b',secondRun.id).viewerPid,viewerPid)
    await until(async()=>await selectedTab()==='fixture-b' && await inputFocused(),'Reopened second tab did not take focus')
    assert.equal(ih('stop','fixture-b','native-console-stop-b',secondRun.id).state,'Stopped')
    try {
      await until(async()=>await selectedTab()===name && await inputFocused(),'Ending second run did not return to first tab')
    } catch (error) {
      const observed = await terminal.evaluate(`({
        tabs: [...document.querySelectorAll('[role=tab]')].map(tab => [tab.textContent,tab.getAttribute('aria-selected')]),
        active: document.activeElement?.className, focused: document.hasFocus(),
        errors: [...document.querySelectorAll('.console-error:not([hidden])')].map(item => item.textContent),
      })`)
      writeFileSync(join(evidence,'focus-failure.json'),JSON.stringify(observed,null,2))
      throw error
    }
  }
  const stale=spawn(cli,[workspace,'console-open',name,previousRun?.id??'old-run-id'],{env,windowsHide:true})
  let failure=''
  stale.stderr.on('data',chunk=>failure+=chunk)
  await new Promise((resolve,reject)=>{stale.on('error',reject);stale.on('close',resolve)})
  assert.notEqual(stale.exitCode,0)
  assert.match(failure,/ConsoleRunChanged/)
  await terminal.capture('console-after-reopen')
  const stopped=ih('stop',name,'native-console-stop',original.run.id)
  assert.equal(stopped.state,'Stopped')
  const finalLog=readFileSync(join(workspace,'runs',name,'stdout.log'),'utf8')
  if(mode==='paper') assert.match(finalLog,/Saving (chunks|world|players)/)
  await until(()=>!processPath(viewerPid),'Terminal did not end with the run')
  assert.equal(ih('manager-exit').exiting,true)
  await until(()=>!processPath(secondManager.pid),'Test Manager did not exit')
  main.close()
  main=undefined
  closeWindow(gui.pid,guiPath)
  await until(()=>gui.exitCode!==null,'Main GUI did not exit')
  result={passed:true,mode,originalRun:original.run,recoveredRun:recovered.run,previousRun:previousRun?.id,
    firstManager,secondManager,viewerPid,webviewId:terminalPage.id,staleRunRejected:true,
    openToFirstOutputMs,longLineIntact:mode==='fixture',
    hiddenOutputOnce:mode==='fixture',multipleTabs:mode==='fixture',batchConsole:mode==='fixture',automaticFocus:true,paperCommand:mode==='paper',paperTab:mode==='paper',
    webviewComposition:mode==='paper',paperSaved:mode==='paper',stopped:true}
  writeFileSync(join(evidence,'native-console-result.json'),JSON.stringify(result,null,2))
  console.log(JSON.stringify(result))
} finally {
  terminal?.close()
  main?.close()
  if(!result) {
    if(originalRun || secondRun) {
      try {
        for (const [instance, run] of [[name, originalRun], ['fixture-b', secondRun]]) {
          if (!run) continue
          const current = ih('status',instance)
          if(current.state!=='Stopped' && current.run?.id===run.id &&
              processPath(run.host.pid).toLowerCase()===cli.toLowerCase()) {
            ih('stop',instance,'native-console-cleanup-'+instance,run.id)
          }
        }
        if(existsSync(managerPath) && processPath(getManager().pid).toLowerCase()===cli.toLowerCase()) ih('manager-exit')
      } catch(error) { console.error('Isolated cleanup: '+error.message) }
    }
    if(gui?.pid && processPath(gui.pid).toLowerCase()===guiPath.toLowerCase()) {
      try { closeWindow(gui.pid,guiPath) } catch(error) { console.error(error.message) }
    }
    console.error('Preserved the isolated workspace for diagnosis: '+workspace)
  }
}
