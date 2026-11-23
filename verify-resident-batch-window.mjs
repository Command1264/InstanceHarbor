import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { resolve, join } from 'node:path'
import { setTimeout as wait } from 'node:timers/promises'

const evidence = resolve(process.argv[2] ?? '')
const root = join(evidence, 'workspace')
const native = join(evidence, 'native')
const cli = join(native, 'ihctl.exe')
const names = ['fixture-a','fixture-b','fixture-c']
const manifest = JSON.parse(readFileSync(join(evidence, 'fixture-manifest.json'),'utf8'))
assert.equal(manifest.owner,'resident-manager-controlled')
assert.equal(manifest.workspace.toLowerCase(),root.toLowerCase())

function ih(...args) {
  return JSON.parse(execFileSync(cli,[root,...args],{windowsHide:true,encoding:'utf8'}))
}
async function until(check, message, ms = 18000) {
  const end = Date.now() + ms
  do {
    const result = await check()
    if (result) return result
    await wait(120)
  } while (Date.now() < end)
  throw new Error(message)
}
async function debugPort() {
  const server = createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const {port} = server.address()
  await new Promise(resolve => server.close(resolve))
  return port
}
const childEnv = {
  ...process.env, IH_WORKSPACE:root, IH_TEST_STOP_WAIT_MS:'10000',
  LOCALAPPDATA:join(evidence,'appdata'), WEBVIEW2_USER_DATA_FOLDER:join(evidence,'webview'),
}
const children = []
async function closeWindow(child) {
  if (child.exitCode !== null) return
  execFileSync('pwsh',['-NoProfile','-Command','(Get-Process -Id '+child.pid+').CloseMainWindow()'],{windowsHide:true})
  await until(()=>child.exitCode!==null,'Native window did not close')
}
async function openWindow() {
  const port = await debugPort()
  const child = spawn(join(native,'InstanceHarbor.exe'),[],{
    windowsHide:true,stdio:'ignore',env:{...childEnv,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port},
  })
  children.push(child)
  const target = await until(async()=>{
    if (child.exitCode !== null) throw Error('Isolated native GUI exited early')
    try {
      const pages = await (await fetch('http://127.0.0.1:'+port+'/json/list',{signal:AbortSignal.timeout(500)})).json()
      return pages.find(page=>page.type==='page' && page.url.includes('tauri.localhost'))
    } catch { return undefined }
  },'Native WebView unavailable')
  const ws = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((resolve,reject)=>{ws.onopen=resolve;ws.onerror=reject})
  let sequence = 0
  function send(method,params={}) {
    const id=++sequence
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{ws.removeEventListener('message',listener);reject(new Error('CDP response timed out'))},5000)
      function listener({data}) {
        const reply=JSON.parse(data)
        if(reply.id!==id)return
        clearTimeout(timer)
        ws.removeEventListener('message',listener)
        if(reply.error)reject(new Error(JSON.stringify(reply.error)))
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
  const capture=async name=>writeFileSync(join(evidence,name+'.png'),Buffer.from((await send('Page.captureScreenshot')).data,'base64'))
  await until(()=>evaluate('document.querySelectorAll("tr[data-instance]").length===3'),'Fixture rows not loaded')
  async function click(label){
    await until(()=>evaluate(`(() => {
      const target = [...document.querySelectorAll('button,summary')].find(b=>b.textContent.trim()===${JSON.stringify(label)} && !b.disabled && b.getClientRects().length)
      if (!target) return false
      target.click()
      return true
    })()`), "Control unavailable: "+label)
  }
  async function close(){ws.close();await closeWindow(child)}
  return {child,evaluate,capture,click,close}
}
const batch=(action)=>ih('batch-list').find(job=>job.action===action)
const gate=(name, file)=>join(root,'instances',name,file)
const held=gate('fixture-a','fixture-hold-stop')
try {
  for (const name of names) writeFileSync(gate(name,'fixture-ready'),'')
  let gui=await openWindow()
  const first=gui.child.pid
  await gui.click('全部操作')
  await gui.click('全部啟動')
  await until(()=>gui.evaluate('document.activeElement.textContent.trim()').then(text=>text==='取消'),'Cancel did not receive default focus')
  await gui.click('確認操作')
  await until(()=>batch('batch-start')?.state==='Completed','GUI batch start did not complete')
  const originals=Object.fromEntries(names.map(name=>[name,ih('status',name).run.id]))
  writeFileSync(held,'')
  // Exercise the selected path, with one instance outside the requested scope.
  await gui.evaluate(`(() => {
    document.querySelector('[data-instance="fixture-a"]').click()
    document.querySelector('[data-instance="fixture-b"]').dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:true}))
  })()`)
  await until(()=>gui.evaluate('document.querySelectorAll("tr[aria-selected=true]").length===2'),'Ctrl selection did not select two fixtures')
  await gui.click('重新啟動所選')
  await until(()=>gui.evaluate('document.activeElement.textContent.trim()').then(text=>text==='取消'),'Cancel did not receive default focus')
  assert.match(await gui.evaluate('document.querySelector("dialog[open]").innerText'),/2 台.*停止逾時/s)
  await gui.capture('selected-restart-confirmation')
  await gui.click('確認操作')
  const restart=await until(()=>batch('batch-restart'),'GUI restart not submitted')
  assert.deepEqual(restart.items.map(item=>item.name).sort(),['fixture-a','fixture-b'])
  await gui.close()
  const recovered=await until(()=>{
    const job=batch('batch-restart')
    return job?.items.some(item=>item.name==='fixture-a' && item.reason==='StopTimeout')
      && job.items.some(item=>item.name==='fixture-b' && item.phase==='Completed') && job
  },'Restart did not continue after the GUI closed')
  assert.equal(recovered.id,restart.id)
  rmSync(held)
  await until(()=>ih('status','fixture-a').state==='Stopped','Timed-out target did not finish stop')
  assert.equal(ih('status','fixture-a').run.id,originals['fixture-a'])
  assert.notEqual(ih('status','fixture-b').run.id,originals['fixture-b'])
  assert.equal(ih('status','fixture-c').run.id,originals['fixture-c'])
  gui=await openWindow()
  assert.notEqual(gui.child.pid,first)
  await gui.click('操作紀錄')
  await until(()=>gui.evaluate(`document.body.innerText.includes(${JSON.stringify(restart.id)}) && document.body.innerText.includes('停止逾時')`),'Reopened GUI did not show the original work and timeout')
  await gui.capture('recovered-restart')
  await gui.click('實例')
  await gui.click('全部操作')
  await gui.click('全部正常停止')
  const copy=await gui.evaluate('document.querySelector("dialog[open]").innerText')
  assert.match(copy,/3 台/)
  assert.doesNotMatch(copy,/最多同時啟動/)
  await gui.click('確認操作')
  const stopped=await until(()=>{
    const job=batch('batch-stop')
    return job?.state==='Completed' && job
  },'GUI batch stop did not complete')
  assert.equal(stopped.items.find(item=>item.name==='fixture-a').phase,'Skipped')
  assert.ok(names.every(name=>ih('status',name).state==='Stopped'))
  await gui.close()
  ih('manager-exit')
  writeFileSync(join(evidence,'native-batch-result.json'),JSON.stringify({
    passed:true,firstGui:first,reopenedGui:gui.child.pid,originals,restart:recovered,stop:stopped,
  },null,2))
  console.log('Native Vue selected restart, close and reopen, timeout, and global stop passed.')
} finally {
  if(existsSync(held))rmSync(held)
  for (const child of children) {
    if (child.exitCode===null) {
      try { await closeWindow(child) } catch { child.kill() }
    }
  }
  if(existsSync(join(root,'batches')) && !existsSync(join(evidence,'native-batch-result.json'))) {
    for (const job of ih('batch-list').filter(job=>job.state==='Pending')) ih('batch-cancel',job.id)
    for (const name of names) {
      const state=ih('status',name)
      if (['Starting','Listening'].includes(state.state)) ih('stop',name,'cleanup-'+crypto.randomUUID(),state.run.id)
    }
    ih('manager-exit')
  }
}
