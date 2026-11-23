import { createLifecycle } from './lifecycle.js';
import { renderDialogContent, renderInstanceRows, renderInstanceDetail, renderInstancePage, clearPreviewPage, renderSettingsPage, renderActivityPage, renderJavaPicker, renderCoreMaintenance } from './regions';
import { readActivity, saveActivity, exportActivity } from './activity-store';
import { inventory, titleCatalog, statusIcons } from './fixtures';
import { settingsPresentation } from './settings-presentation';
// Offline interaction model only. No RPC, file-system access or persistent storage.
const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const iconPin = '<svg class="pin" aria-hidden="true" viewBox="0 0 16 20"><path d="M5 2h6M6 2v6l-3 4h10l-3-4V2M8 12v7" fill="none" stroke="currentColor" stroke-width="1.8"/></svg>';
const demoBase='C:\\ExampleServers';
const seed = [
 {id:'velocity',name:'Velocity',project:'路由',kind:'Human',core:'Velocity',version:'4.2.1',port:49193,status:'running',build:32,latest:38,pin:0},
 {id:'example-survival',name:'example-survival',project:'範例',kind:'Paper',core:'Paper',version:'26.3',port:49170,status:'running',build:18,latest:18,pin:1},
 {id:'lobby',name:'lobby',project:'Lobby',kind:'Human',core:'Paper',version:'26.3',port:49180,status:'stopped',build:18,latest:18,pin:null},
 {id:'invite',name:'examplepack-spigot-1.8.8',project:'ExamplePack',kind:'Human',core:'Spigot',version:'1.8.8',port:49181,status:'stopped',build:'582b',latest:'635a',pin:null},
 {id:'drop',name:'exampleitems-paper-26.2',project:'ExampleItems',kind:'Human',core:'Paper',version:'26.2',port:49189,status:'stopped',build:112,latest:133,pin:null},
 {id:'folia',name:'examplepack-folia-26.1.2',project:'ExamplePack',kind:'Human',core:'Folia',version:'26.1.2',port:49194,status:'stopped',build:7,latest:10,pin:null},
 {id:'agent',name:'Agent 隔離相容性測試',project:'InstanceHarbor',kind:'Agent',core:'Spigot',version:'1.14',port:31001,status:'unknown',build:39,latest:39,pin:null}
];
const demoLocations={velocity:'instances/Velocity',example-survival:'instances/Human/Example/survival-sample',lobby:'instances/Human/Lobby/lobby',invite:'instances/Human/ExamplePack/examplepack-spigot-1.8.8',drop:'instances/Human/ExampleItems/exampleitems-paper-26.2',folia:'instances/Human/ExamplePack/examplepack-folia-26.1.2',agent:'instances/Agent/compat'};
for(const row of seed)row.path=demoBase+'\\'+demoLocations[row.id].replace(/\//g,'\\');
const names = {running:'正在監聽',stopped:'已停止',starting:'啟動中',stopping:'停止中',failed:'啟動失敗',unknown:'尚未核對'};
const marks = {running:'●',stopped:'○',starting:'▲',stopping:'▲',failed:'■',unknown:'◇'};
const defaults = {java:'C:\\Java\\temurin-21\\bin\\java.exe',xms:'1G',xmx:'3G',gc:'G1',args:'-Dfile.encoding=UTF-8',stop:90,start:120,buffer:300,crash:false,unexpected:false,delays:'5, 15, 30',attempts:3,window:300,stable:120,title:'{instance}'};
const descriptors = {
 runtime:[['java','Java 執行檔','text'],['xms','初始記憶體（-Xms）','text'],['xmx','最大記憶體（-Xmx）','text'],['gc','垃圾回收器','select',['預設','G1','Parallel','ZGC']],['args','進階 JVM 參數（每行一項）','textarea']],
 restart:[['stop','正常停止期限（秒）','number'],['start','啟動就緒期限（秒）','number'],['buffer','資源釋放後緩衝（毫秒）','number']],
 autorestart:[['crash','崩潰後自動重啟','checkbox'],['unexpected','非手動正常退出後重啟','checkbox'],['delays','重試等待秒數','text'],['attempts','最多自動重試次數','number'],['window','重試計數時間窗（秒）','number'],['stable','穩定運行判定（秒）','number']],
 terminal:[['title','原生後台標題範本','text']]
};
const policySeed = [
 ['view','視距',10,'number','所有已登記版本'],['simulation','模擬距離',4,'number','新版支援；1.8.8／1.14 略過'],
 ['spawn','出生點保護範圍',0,'number','所有已登記版本'],['players','人數上限',20,'number','所有已登記版本'],
 ['motd','伺服器說明','Human 測試世界','text','所有已登記版本'],['difficulty','難度','普通','select','所有已登記版本'],
 ['mode','預設遊戲模式','生存','select','所有已登記版本'],['flat','世界生成類型','基本超平坦','select','只影響新世界'],
 ['structures','生成結構',true,'checkbox','各版本使用對應設定'],['nether','載入地獄維度',true,'checkbox','各核心使用對應設定'],
 ['portal','允許地獄傳送門進入',true,'checkbox','依核心與版本能力套用'],['flight','允許飛行',true,'checkbox','所有已登記版本'],
 ['whitelist','啟用玩家白名單',false,'checkbox','與掃描路徑無關'],['keep','死亡保留物品（防噴）',true,'checkbox','所有已登記版本'],
 ['mob','生物與方塊互動',false,'checkbox','所有已登記版本'],['day','日夜循環',false,'checkbox','所有已登記版本'],
 ['weather','天氣循環',false,'checkbox','舊版本未提供時略過'],['cramming','實體擠壓上限',24,'number','1.8.8 不提供'],
 ['item','掉落物消失時間（tick）',6000,'number','Spigot 系設定；各版本對應'],['examplePermission','範例權限顯示',true,'checkbox','新建測服政策；不回溯既有玩家']
];
const javaRecords=[
 {label:'Eclipse Temurin 21.0.8',path:'C:\\Java\\temurin-21\\bin\\java.exe',source:'一般安裝',version:'21.0.8',exists:true,runnable:true,aliases:[]},
 {label:'mise local-25 · OpenJDK 25.0.5.0',path:'D:\\DemoJava\\mise\\25.0.5.0\\bin\\java.exe',source:'mise',version:'25.0.5.0',exists:true,runnable:true,aliases:['mise local-25']},
 {label:'mise local-25 · OpenJDK 25.0.5.0',path:'D:\\DemoJava\\mise\\25.0.5.0\\bin\\java.exe',source:'PATH',version:'25.0.5.0',exists:true,runnable:true,aliases:['mise local-25']},
 {label:'Oracle Java 17.0.16',path:'C:\\Program Files\\Java\\jdk-17\\bin\\java.exe',source:'一般安裝',version:'17.0.16',exists:true,runnable:true,aliases:[]},
 {label:'OpenJDK 8u462',path:'C:\\Java\\jdk8\\bin\\java.exe',source:'JAVA_HOME',version:'8u462',exists:true,runnable:true,aliases:[]},
 {label:'舊 mise local-25 · 25.0.4.1',path:'D:\\DemoJava\\mise\\25.0.4.1\\bin\\java.exe',source:'歷史參照',version:'25.0.4.1',exists:false,runnable:false,aliases:['mise local-25（舊）']},
 {label:'存在但無法執行的 Java',path:'D:\\DemoJava\\blocked\\bin\\java.exe',source:'手動路徑',version:'尚未核對',exists:true,runnable:false,aliases:[]}
];
function javaKey(path){return String(path).replace(/\//g,'\\').replace(/\\+$/,'').toLowerCase();}
function dedupeJava(records){const active=new Map(),invalid=new Map();for(const record of records){const key=javaKey(record.canonicalPath||record.path),map=record.exists&&record.runnable?active:invalid;if(map.has(key)){const previous=map.get(key);previous.aliases=[...new Set([...previous.aliases,...record.aliases])];previous.sources=[...new Set([...previous.sources,record.source])];}else map.set(key,{...record,aliases:[...record.aliases],sources:[record.source]});}return {active:[...active.values()],invalid:[...invalid.values()]};}
const javaCatalog=dedupeJava(javaRecords);
const javaChoices=javaCatalog.active.map(record=>[record.label,record.path,record.sources.join('／')]);
const tabs={runtime:'Java 與 JVM',restart:'停止與重啟',autorestart:'自動重啟',terminal:'原生後台',initialization:'初始化預設',scan:'掃描範圍',appearance:'顯示設定',world:'世界政策'};
function probeJava(path){const record=javaRecords.find(r=>javaKey(r.path)===javaKey(path));if(!record)return {status:'unknown',message:'尚未核對這個路徑。本線框不讀本機檔案，請選清單中的示範 Java。'};if(!record.exists)return {status:'missing',message:'Java 檔案已不存在。請重新選擇有效安裝；不會自動切換到其他版本。'};if(!record.runnable)return {status:'unrunnable',message:'路徑存在，但 java -version 未能成功執行。請檢查安裝或權限，或重新選擇。'};return {status:'ready',message:'示範核對通過：檔案存在，java -version 可執行。沒有在妳的電腦執行 Java。',version:record.version};}

// Logical design fields; real file/key mapping remains version-capability driven.
const policyGroups={server:'伺服器',spigot:'Spigot／Bukkit',paper:'Paper',world:'世界',rules:'遊戲規則'};
const policyMeta={};
for(const field of policySeed)policyMeta[field[0]]={family:'server',source:'server.properties',advanced:false};
for(const key of ['keep','mob','day','weather','cramming','portal'])policyMeta[key]={family:'rules',source:'遊戲規則 · 依核心／版本對應',advanced:['cramming','weather'].includes(key)};
for(const key of ['flat','structures'])policyMeta[key]={family:'world',source:'server.properties · 世界生成',newOnly:true,advanced:false};
policyMeta.nether={family:'spigot',source:'bukkit.yml／新版核心對應設定',advanced:false};
policyMeta.item={family:'spigot',source:'spigot.yml · 世界設定',advanced:false};
policyMeta.examplePermission={family:'server',source:'新建管理權限政策',advanced:true};
const policyExtras=[
 ['pvp','允許玩家互相傷害',true,'checkbox','server','server.properties',false],
 ['online','驗證玩家帳號',false,'checkbox','server','server.properties',true],
 ['sync-chunk-writes','同步寫入區塊',null,'checkbox','server','server.properties · sync-chunk-writes',true],
 ['compression','網路壓縮閾值',256,'number','server','server.properties',true],
 ['idle','閒置踢出時間（分鐘）',0,'number','server','server.properties',true],
 ['commandBlock','啟用指令方塊',false,'checkbox','server','server.properties',true],
 ['animals','動物生成上限',10,'number','spigot','bukkit.yml · spawn-limits',false],
 ['monsters','怪物生成上限',70,'number','spigot','bukkit.yml · spawn-limits',false],
 ['autosave','自動儲存間隔（tick）',6000,'number','spigot','bukkit.yml · ticks-per',false],
 ['activation','動物活躍範圍',32,'number','spigot','spigot.yml · world-settings',false],
 ['tracking','實體追蹤範圍',48,'number','spigot','spigot.yml · world-settings',true],
 ['merge','掉落物合併半徑',2.5,'decimal','spigot','spigot.yml · world-settings',true],
 ['hopper','漏斗檢查間隔（tick）',1,'number','spigot','spigot.yml · world-settings',true],
 ['paperAutosave','每 tick 自動儲存區塊上限',24,'number','paper','config/paper-world-defaults.yml',false],
 ['perPlayerMobs','依玩家計算生物生成',true,'checkbox','paper','config/paper-world-defaults.yml',false],
 ['entitySave','每區塊實體儲存上限（示例）',100,'number','paper','config/paper-world-defaults.yml',true],
 ['explosion','保留爆炸擊退',true,'checkbox','paper','config/paper-world-defaults.yml',true],
 ['packetLimit','封包限制（示例政策）',500,'number','paper','config/paper-global.yml',false],
 ['chunkThreads','區塊 I/O 執行緒（示例）',2,'number','paper','config/paper-global.yml',true],
 ['seed','世界種子','','text','world','server.properties · level-seed',false,true],
 ['levelName','世界資料夾名稱','world','text','world','server.properties · level-name',false,true],
 ['worldSize','世界最大尺寸',29999984,'number','world','server.properties · max-world-size',true,true],
 ['randomTick','隨機刻速度',3,'number','rules','遊戲規則 · 依核心／版本對應',true],
 ['mobSpawn','自然生成生物',true,'checkbox','rules','遊戲規則 · 依核心／版本對應',false],
 ['fire','火焰蔓延',false,'checkbox','rules','遊戲規則 · 依核心／版本對應',false],
 ['insomnia','失眠生成幻翼',false,'checkbox','rules','遊戲規則 · 依核心／版本對應',true],
 ['respawn','立即重生',false,'checkbox','rules','遊戲規則 · 依核心／版本對應',true],
 ['feedback','指令回饋',true,'checkbox','rules','遊戲規則 · 依核心／版本對應',true],
 ['sleepPercent','跳過夜晚所需玩家比例',100,'number','rules','遊戲規則 · 依核心／版本對應',true]
];
for(const [key,label,value,type,family,source,advanced,newOnly]of policyExtras){policySeed.push([key,label,value,type,'實際支援按核心／版本核對']);policyMeta[key]={family,source,advanced,newOnly:!!newOnly};}
const popups=new Map(); let pendingTimers=[];let dialogCounter=0;
let state;
function resetState(){
 state={route:'instances',rows:structuredClone(seed.filter(r=>r.kind!=='Agent')),selected:new Set(),anchor:null,query:'',filter:'all',sort:'name',direction:1,scenario:'normal',connected:true,records:[],tasks:[],tab:'runtime',target:null,draft:null,baseline:null,dirty:false,settingsError:'',revision:1,globals:{...defaults},instanceSettings:Object.fromEntries(seed.filter(r=>r.kind!=='Agent').map(r=>[r.id,{...defaults}])),templateTab:'runtime',policyCategory:'server',advancedPolicy:false,appearance:{theme:'system',accent:'system',language:'zh-TW',time:'local24'},policies:{Human:Object.fromEntries(policySeed.map(x=>[x[0],x[2]]))},audience:'Human',policyFilter:'',worldTab:'files',worldValues:{},initTime:false,worldTime:false,scanAllow:'.\\instances\\Human\nC:\\ExampleServers\\instances\\Velocity',scanDeny:'',wizard:null,recordFilter:'all',legacyCandidates:legacyFixtures(),legacyBackups:{},activeStartCount:0,startPumps:new Set()};
}
resetState();
const rowBy = id => state.rows.find(r=>r.id===id);
const readableTime = date => new Intl.DateTimeFormat('zh-TW',{dateStyle:'short',timeStyle:'medium',...(state.appearance.time==='utc'?{timeZone:'UTC'}:{hour12:state.appearance.time==='local12'})}).format(date || new Date());
function confirmAtButton(button, label, announcement=label) {
 if (button.dataset.localFeedback) return;
 button.dataset.localFeedback = 'true';
 const original = button.textContent;
 const width = button.style.minWidth;
 button.style.minWidth = button.getBoundingClientRect().width + 'px';
 button.textContent = label;
 announce(announcement);
 timer(() => {
  if (!button.isConnected || button.textContent !== label) return;
  button.textContent = original;
  button.style.minWidth = width;
  delete button.dataset.localFeedback;
 }, 1400);
}
function announce(text){$('#announcement').textContent=text;}
function messageBox(title,text){return mountDialog(title,`<p>${esc(text)}</p>`,button('確定','close-dialog'),true);}
function timer(fn,ms){const id=setTimeout(fn,ms);pendingTimers.push(id);return id;}
function statusIcon(status){const name={running:'circle-dot',stopped:'circle',starting:'circle-dashed',stopping:'circle-dashed',failed:'circle-x',unknown:'circle-help'}[status]||'circle-help';return statusIcons[name];}
function badge(status){return `<span class="badge ${esc(status)}">${statusIcon(status)}<span class="status-text">${names[status]||'未知'}</span></span>`;}
function button(label,action,extra=''){return `<button type="button" data-action="${action}" ${extra}>${label}</button>`;}
function pageHead(title,subtitle='',actions=''){return `<header class="page-head"><div><h1>${title}</h1>${subtitle?`<p>${subtitle}</p>`:''}</div><div class="actions">${actions}</div></header>`;}
function setConnection(connected){state.connected=connected;$('#connection').innerHTML=statusIcon(connected?'running':'failed')+`<span>${connected?'管理服務：已連線（示範）':'管理服務：連線中斷，保留最後核對資料'}</span>`;$('.statusbar').classList.toggle('disconnected',!connected);}
function applyTheme(theme=state.appearance.theme,accent=state.appearance.accent){const actual=theme==='system'?(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'):theme;document.documentElement.dataset.theme=actual;document.documentElement.dataset.accent=accent; $('meta[name=theme-color]').content=actual==='dark'?'#121c2b':'#f4f6fa';}
applyTheme();
function guardLeave(next){if(!state.dirty)return next();confirmDialog('放棄尚未儲存的變更？','離開後會回復已儲存的設定。','放棄變更',()=>{state.dirty=false;applyTheme();next();});}
function navigate(route,options={}){guardLeave(()=>{
 state.policyFilter='';
 state.route=route;state.settingsError='';if(route==='settings'){state.target=options.target??null;state.tab=options.tab||(state.target?'runtime':'appearance');if(!state.target&&['runtime','restart','autorestart','terminal'].includes(state.tab)){state.templateTab=state.tab;state.tab='templates';}state.draft=null;state.baseline=null;state.dirty=false;}
 render();const heading=$('h1',$('#view'));heading.tabIndex=-1;heading.focus();
});}
function render(){
 $('#view').className=($('#view').dataset.overlay?'overlay-scroll ':'')+'route-'+state.route;
 $$('[data-nav]', $('.rail')).forEach(a=>{if(a.dataset.nav===(state.route==='settings'&&state.target?'instances':state.route))a.setAttribute('aria-current','page');else a.removeAttribute('aria-current');});
 if(state.route==='instances')renderInstances();else if(state.route==='settings')renderSettings();else if(state.route==='activity')renderActivity();else if(state.route==='create'||state.route==='import')renderWizard();else renderReview();
 updateCount();enhanceScrolls();
}

function resolveDemoPath(input){
 let value=input.trim().replace(/\//g,'\\');if(!value)return '';
 if(!/^[a-z]:\\/i.test(value)&&!value.startsWith('\\\\')){
  if(!value.includes('\\'))throw Error('請輸入相對路徑（例如 .\\instances\\Human）或完整絕對路徑，不使用用途單詞。');
  value=demoBase+'\\'+value.replace(/^\.\\/,'');
 }
 const parts=[];for(const part of value.split('\\')){if(part==='.')continue;if(part==='..')throw Error('相對路徑不能以 .. 逸出示範基準目錄。');parts.push(part);}
 return parts.join('\\').replace(/\\+$/,'').toLowerCase();
}
function demoRowPath(row){return resolveDemoPath(row.path);}
function scanIncludes(row,allowText,denyText){
 const target=demoRowPath(row);
 const matches=text=>text.split(/\r?\n/).filter(x=>x.trim()).map(resolveDemoPath).some(rule=>target===rule||target.startsWith(rule+'\\'));
 return matches(allowText)&&!matches(denyText);
}
function visibleRows(){let rows=state.scenario==='empty'?[]:state.rows.filter(r=>(!state.scanActive||scanIncludes(r,state.scanAllow,state.scanDeny))&&r.kind!=='Agent'&&!r.removed&&r.managed!==false&&r.hasManagementData!==false&&!r.external&&(state.filter==='all'||r.status===state.filter)&&[r.name,r.project,r.kind,r.port,r.core,r.version,r.path].join(' ').toLowerCase().includes(state.query.toLowerCase()));
 return rows.sort((a,b)=>{if(a.pin!==null||b.pin!==null){if(a.pin===null)return 1;if(b.pin===null)return -1;return a.pin-b.pin;}if(state.direction===0)return state.rows.indexOf(a)-state.rows.indexOf(b);return String(a[state.sort]).localeCompare(String(b[state.sort]),'zh-TW',{numeric:true})*state.direction;});}
function renderInstances(){
 $('#view').className=($('#view').dataset.overlay?'overlay-scroll ':'')+'route-instances';
 renderInstancePage($('#view'),state,names);
 $('#status-filter').value=state.filter;renderRows();renderDetail();
}
function renderRows(){
 const focusId=document.activeElement?.dataset?.rowselect;const rows=visibleRows(),ids=new Set(rows.map(r=>r.id));if(!ids.has(state.anchor))state.anchor=null;state.selected=new Set([...state.selected].filter(id=>ids.has(id)));
 $('#selection-count').textContent=`${rows.length} 台顯示　／　已選取 ${state.selected.size} 台`;
 renderInstanceRows($('#instance-rows'),rows,state.selected,state.anchor);
 $('#list-empty').hidden=rows.length!==0;if(focusId)$(`[data-rowselect="${focusId}"]`)?.focus({preventScroll:true});
}
function recentDetail(r){const record=state.records.find(x=>x.ids.includes(r.id)),task=record&&state.tasks.find(t=>t.id===record.id);let result='尚無操作';
 if(record){const item=task?.results.find(x=>x.id===r.id||x.name===r.name),raw=item?.result||record.result;
 result=record.action+'：'+(task&&!task.done?(task.stages?.[r.id]||'處理中'):/逾時/.test(raw)?'停止逾時':/未確認/.test(raw)?'結果未確認':/失敗/.test(raw)?'失敗':/取消/.test(raw)?'已取消':/略過/.test(raw)?'已略過':raw==='成功'||raw==='已完成'?'已完成':raw);}
 return {text:result,tooltip:r.last||result,recordId:record?.id};
}
function renderDetail(){const ids=[...state.selected],panel=$('#detail');if(!panel)return;const same=panel.dataset.selection===ids.join('|'),expanded=same&&!!$('.diagnostics',panel)?.open,menuOpen=same&&!!$('.management-actions',panel)?.open,offset=same?panel.scrollTop:0,active=document.activeElement;let focus='';
 if(same&&panel.contains(active))focus=active.hasAttribute('data-primary-power')?'[data-primary-power]':active.matches('.management-actions>summary')?'.management-actions>summary':active.dataset.action?`[data-action="${active.dataset.action}"]`:'';
 panel.dataset.selection=ids.join('|');panel.setAttribute('aria-label','實例詳細資訊');
 const rows=ids.map(rowBy).filter(Boolean);
 if(rows.length!==1){renderInstanceDetail(panel,{rows});panel.scrollTop=offset;return;}
 const r=rows[0];
 const safe=state.connected&&!['unknown','starting','stopping'].includes(r.status);
 const busy=state.tasks.some(t=>!t.done&&t.ids.includes(r.id));
 const pendingStop=state.tasks.find(task=>task.action==='stop'&&task.scope==='selected'&&task.ids.length===1&&task.stages[r.id]==='已送停止，等待存檔與退出');
 const emergency=!!pendingStop && state.connected && r.status==='stopping';
 const phase=emergency?'緊急停止':r.status==='starting'?'啟動中':r.status==='stopping'?'停止中':busy?'等待工作完成':r.status==='running'?'正常停止':'啟動';
 const power=emergency?'emergency-stop':r.status==='running'?'stop':'start';
 const reason=!state.connected?'管理服務未連線，請重新連線核對。':r.status==='unknown'?'尚未核對實例身分；請展開診斷核對。':busy||!safe?'已有工作進行中，請查看操作結果。':'';
 const pins=state.rows.filter(x=>managedRow(x)&&x.pin!==null).sort((a,b)=>a.pin-b.pin);
 renderInstanceDetail(panel,{rows,detail:{
  row:{...r},blocked:!safe||busy,emergency,phase,power,reason,expanded,
  connected:state.connected,consoleDisabled:!safe||r.status!=='running',
  pinIndex:pins.findIndex(x=>x.id===r.id),pinCount:pins.length,recent:recentDetail(r),
 }});
 if(!same&&$('.management-actions',panel))$('.management-actions',panel).open=false;
 if(menuOpen)$('.management-actions',panel).open=true;panel.scrollTop=offset;
 if(focus){
  // Retain focus during stop -> emergency, but never hand a held key to
  // the opposite operation when the asynchronous stop/start completes.
  const changedIntent=focus==='[data-primary-power]'&&active.dataset.action!==power&&power!=='emergency-stop';
  const target=changedIntent?$('h2',panel):$(focus,panel);
  if(changedIntent&&target)target.tabIndex=-1;
  target?.focus({preventScroll:true});
 }
}
function selectRow(id,event,toggle=false){const rows=visibleRows();if(event.shiftKey&&state.anchor){const a=rows.findIndex(r=>r.id===state.anchor),b=rows.findIndex(r=>r.id===id);if(a>=0&&b>=0){if(!event.ctrlKey&&!event.metaKey)state.selected.clear();rows.slice(Math.min(a,b),Math.max(a,b)+1).forEach(r=>state.selected.add(r.id));}}
 else if(toggle||event.ctrlKey||event.metaKey){if(state.selected.has(id))state.selected.delete(id);else state.selected.add(id);state.anchor=id;}else{state.selected=new Set([id]);state.anchor=id;}
 renderRows();renderDetail();const focus=$(`[data-rowselect="${id}"]`);if(focus)focus.focus();announce(`已選取 ${state.selected.size} 台實例`);}
function mountDialog(title,body,actions,compact=false){$$('dialog:not([open])').forEach(d=>d.remove());const previous=document.activeElement;const d=document.createElement('dialog');if(compact)d.className='compact';const id='dialog-title-'+(++dialogCounter);d.setAttribute('aria-labelledby',id);renderDialogContent(d,id,title,body,actions);$('#modal-root').append(d);d.addEventListener('close',()=>{d.remove();enhanceScrolls();const top=$$('dialog[open]').at(-1);if(previous?.isConnected&&(!top||top.contains(previous))&&(document.activeElement===document.body||d.contains(document.activeElement)))previous.focus();if(!top&&state.route==='instances'){renderRows();renderDetail();}});d.showModal();enhanceScrolls();const safe=$('[data-action=close-dialog]',d);if(safe)safe.focus();else{const titleNode=$('h2',d);titleNode.tabIndex=-1;titleNode.focus();}return d;}
function confirmDialog(title,body,label,callback,danger=false){const d=mountDialog(title,body,button('取消','close-dialog')+button(label,'confirm-current',danger?'class="danger"':'class="primary"'),true);d._confirm=callback;const cancel=$('[data-action=close-dialog]',d);cancel.focus();return d;}
function closeDialogs(){Array.from($$('dialog')).reverse().forEach(d=>{d.close();d.remove();});}
function updateCount(){const n=state.tasks.filter(t=>!t.done).length;$('#task-count').textContent=n?`(${n})`:'';}
function newRecord(action,ids,result='進行中'){const record={id:'demo-'+(state.records.length+1),action,ids:[...ids],targets:ids.map(id=>rowBy(id)?.name||id),result,time:readableTime(),detail:'示範資料；不會送出伺服器命令。'};state.records.unshift(record);return record;}
function triggerOperation(action,ids=[...state.selected]){if(!state.connected)return messageBox('需要處理', '管理服務未連線。先重新連線並核對實例。');const targets=ids.map(rowBy).filter(Boolean);if(targets.some(r=>r.removed||r.hasManagementData===false||r.managed===false||r.external))return messageBox('不在管理範圍','請先透過匯入已有伺服器重新加入，不能直接控制未登記的實例。');if(!targets.length)return messageBox('需要處理', '請先選取目標實例。');if(targets.some(r=>state.tasks.some(t=>!t.done&&t.ids.includes(r.id))))return messageBox('需要處理', '所選實例已有工作進行中，可在操作紀錄查看。');
 if(action==='restore'&&targets.some(r=>r.status!=='stopped'))return messageBox('需要處理', '請先正常停止目標，再核對備份回復；本操作不自動停止。');
 if(targets.length===1&&['start','restart'].includes(action)){const check=probeJava(state.instanceSettings[targets[0].id]?.java);if(check.status!=='ready'){const record=newRecord(action==='start'?'啟動':'重新啟動',ids,'失敗');record.detail=check.message;targets[0].last='啟動前核對失敗：Java 不可用';if(state.route==='instances'){renderRows();renderDetail();}return messageBox('Java 無法使用',check.message);}}
 const label={start:'啟動',stop:'正常停止',restart:'重新啟動',update:'更新核心',restore:'回復核心'}[action];const run=()=>runOperation(action,targets.map(r=>r.id),label);
 if(targets.length===1&&(action==='start'||action==='stop'||(action==='restart'&&targets[0].status==='stopped')))run();else confirmDialog(`${label}${targets.length>1?'所選實例':''}？`,`<p>${targets.map(r=>esc(r.name)).join('<br>')}</p><p>${action==='update'||action==='restore'?'核對停止條件與核心備份後才替換；完成保持停止。':action==='start'?'示範以有限併發處理，不一次啟動所有目標。':'會讓目標玩家斷線，先等待正常停止完成；逾時不強制終止。'}</p>`,`${label}`,run);
}
function allTargets(){return state.rows.filter(r=>r.kind!=='Agent'&&!r.removed&&r.hasManagementData!==false&&r.managed!==false&&!r.external);}
function triggerAll(action){const menu=$('.all-actions');if(menu){menu.open=false;$('summary',menu).focus();}if(!state.connected)return messageBox('需要處理','管理服務未連線。先重新連線並核對實例。');const targets=allTargets().map(r=>({id:r.id,name:r.name}));if(!targets.length)return messageBox('沒有管理目標','目前沒有已納入管理的使用者實例。');const label={start:'全部啟動',stop:'全部正常停止',restart:'全部重啟'}[action],impact={start:'已運作的實例略過；以有限併發啟動，可能增加 CPU 與記憶體負載。',stop:'運作中的玩家會斷線；等待正常停止，已停止的略過。逾時不強制終止。',restart:'運作中的玩家會斷線，正常停止後重開；已停止的也會啟動。以有限併發處理。'}[action];const dialog=confirmDialog(label+'？',`<p>範圍：所有已納入管理的使用者實例，包含被搜尋或狀態篩選隱藏的項目。不含 Agent、已移除管理與外部程序。</p><p>${impact}</p><p>啟停中、身分未確認或已有衝突工作的目標會個別略過並保留原因。</p><details><summary>查看本次目標名稱</summary><ul>${targets.map(r=>`<li>${esc(r.name)}</li>`).join('')}</ul></details><p class="hint">只有離線示範，不會向服務送出命令。</p>`,label,()=>{if(!state.connected)return messageBox('需要處理','管理服務已斷線，本次未受理；請重新核對。');runOperation(action,targets.map(r=>r.id),label,'all');});dialog.id='all-confirm-dialog';dialog.classList.remove('compact');}
function runOperation(action,ids,label,scope='selected'){if(['start','stop','restart'].includes(action))return runLifecycle(action,ids,label,scope);const outcome=['failure','timeout','unknown'].includes(state.scenario)?state.scenario:'normal';if(outcome!=='normal'){$('#scenario').value='normal';state.scenario='normal';}
 const record=newRecord(label,ids);record.scope=scope;const task={id:record.id,ids,action,label,scope,done:false,progress:0,record,results:[]};state.tasks.push(task);let cursor=0,active=0,finished=0;const queue=[...ids];
 function finishOne(r,result){task.results.push({name:r.name,result});r.last=`${label}：${result}`;finished++;active--;task.progress=Math.round(finished/ids.length*100);if(finished===ids.length){task.done=true;record.result=task.results.some(x=>/失敗|逾時|未確認/.test(x.result)||(scope==='all'&&/身分|衝突/.test(x.result)))?'需要處理':task.results.some(x=>x.result.includes('取消'))?(task.results.every(x=>x.result.includes('取消'))?'已取消':'部分取消'):'已完成';record.detail=task.results.map(x=>`${x.name}：${x.result}`).join('\n');announce(`${label}${record.result}`);}refreshTaskUI();pump();}
 function pump(){while(active<2&&cursor<queue.length){const id=queue[cursor++],r=rowBy(id);active++;
  if(task.cancelled){finishOne(r,'已取消尚未開始的工作');continue;}
  if(scope==='all'&&(r.kind==='Agent'||r.managed===false||r.external)){finishOne(r,'已略過：不在管理範圍');continue;}
  if(scope==='all'&&(state.tasks.some(t=>t!==task&&!t.done&&t.ids.includes(r.id))||['starting','stopping'].includes(r.status))){finishOne(r,'已略過：正在啟停或有衝突工作');continue;}
  if(action==='update'&&String(r.build)===String(r.latest)){finishOne(r,'已略過：沒有可用更新');continue;}if(action==='update'&&r.core==='Spigot'&&!state.localCandidates?.has(r.id)){finishOne(r,'略過：請先選擇已核對的本機核心');continue;}
  if(r.status==='unknown'){finishOne(r,'略過：尚未核對身分');continue;}
  if((action==='start'&&r.status==='running')||(action==='stop'&&r.status==='stopped')){finishOne(r,'已略過');continue;}
  if(['start','restart'].includes(action)){const check=probeJava(state.instanceSettings[r.id]?.java);if(check.status!=='ready'){finishOne(r,'失敗：'+check.message);continue;}}
  const before=r.status;if((action==='update'||action==='restore')&&before==='running')r.status='stopping';if(action==='start'||(action==='restart'&&before==='stopped'))r.status='starting';else if(action==='stop'||action==='restart')r.status='stopping';if(action==='update'||action==='restore'){task.stage=before==='running'?'等待正常停止（示範）':'核對停止證據、雜湊與備份（示範）';record.detail=task.stage;}r.last=`${label}：進行中`;refreshTaskUI();
  timer(()=>{if(outcome==='timeout'&&(action==='stop'||action==='restart'||(action==='update'&&before==='running'))){r.status=before;finishOne(r,'停止逾時，尚未確認停止');return;}if(outcome==='unknown'){r.status='unknown';finishOne(r,'結果未確認，請唯讀核對');return;}if(outcome==='failure'){r.status=action==='start'?'failed':before;finishOne(r,'失敗：示範安全核對未通過，原核心未變');return;}
   if(action==='restart'){r.status='starting';refreshTaskUI();}if(action==='update'||action==='restore'){r.status='stopped';task.stage='停止、核心身分及備份核對完成（示範）；準備替換';record.detail=task.stage;refreshTaskUI();}
   timer(()=>{r.status=(action==='start'||action==='restart')?'running':'stopped';if(action==='update')r.build=r.latest;if(action==='restore')r.build=state.restoreCandidate?.id===r.id?state.restoreCandidate.build:r.core==='Spigot'?'582b':r.core==='Paper'?112:32;r.checked=readableTime();finishOne(r,'成功');},450);
  },500);
 }}
 pump();refreshTaskUI();announce(`${label}已受理。進度與逐台結果保留在操作紀錄。`);
}
function refreshTaskUI(){updateCount();if(state.route==='instances'&&!$('dialog')){renderRows();renderDetail();}if(state.route==='activity')renderActivity();$$('[data-task-detail]').forEach(d=>{const t=state.tasks.find(x=>x.id===d.dataset.taskDetail);if(t){$('.task-progress',d).value=t.progress;$('.task-state',d).textContent=t.done?t.record.result:'執行中';if(t.lifecycle&&$('.schedule-demo',d))$('.schedule-demo',d).innerHTML=lifecycleDiagramBody(t);$('.task-results',d).textContent=t.results.map(x=>`${x.name}：${x.result}`).join('\n')||'尚無完成項目';}});}
function renderActivity(){
 const records=state.records.filter(record=>state.recordFilter==='all'||/核心/.test(record.action));
 renderActivityPage($('#view'),{core:state.recordFilter==='core',records:records.map(record=>{
  const task=state.tasks.find(item=>item.id===record.id);
  return {...record,targets:(record.targets??record.ids.map(id=>rowBy(id)?.name||id)).join('、'),progress:task&&!task.done?task.progress:undefined};
 })});
}
function recordDialog(id){const r=state.records.find(r=>r.id===id);if(!r)return;const task=state.tasks.find(t=>t.id===id);const d=mountDialog(esc(r.action),`<p class="task-state">${esc(r.result)}</p><p class="hint">${esc(task?.stage||'')}</p>${task?`<progress class="task-progress" value="${task.progress}" max="100" aria-label="工作進度"></progress>`:''}${task?.lifecycle?lifecycleDiagram(task):''}<pre class="task-results">${esc(r.detail)}</pre>${/未確認/.test(r.detail)?button('唯讀核對結果','reconcile',`data-id="${r.id}"`):''}${/逾時/.test(r.detail)?button('繼續等待','wait-again')+button('查看停止診斷','stop-diagnosis'):''}`,(task&&!task.done?button('取消此工作剩餘項目','cancel-remaining',`data-id="${id}"`):'')+button('關閉','close-dialog'));if(task)d.dataset.taskDetail=id;}
function coreDialog(id=[...state.selected][0]){
 const row=rowBy(id);if(!row)return;
 const dialog=mountDialog(esc(row.name)+' · 核心維護','', '',true);
 renderCoreMaintenance($('.dialog-body',dialog),{row,connected:state.connected,update:String(row.build)!==String(row.latest)});
 dialog.dataset.coreId=id;
 $('[data-action=close-dialog]',dialog).focus();
}
function backupsDialog(id){const r=rowBy(id);mountDialog(`${esc(r.name)} · 備份與診斷`,`<label for="backup-select">可回復核心</label><p><select id="backup-select"><option value="${seed.find(x=>x.id===r.id)?.build??r.build}">前一版 · build ${seed.find(x=>x.id===r.id)?.build??r.build}</option><option value="${r.core==='Spigot'?'550c':100}">最近成功啟動版 · build ${r.core==='Spigot'?'550c':100}</option></select></p><p class="hint">回復前核對備份雜湊與停止條件；不改世界或插件資料。</p><div class="actions">${button('核對並回復','restore',`data-id="${id}"`)}${button('核對未確認結果','resolve-core',`data-id="${id}"`)}${button('停止證據與舊資料核對','legacy-info')}</div><details><summary>維護診斷</summary><pre id="core-json">${esc(JSON.stringify({instance:id,core:r.core,version:r.version,localBuild:r.build,latestBuild:r.latest,hash:'示範雜湊',run:'demo-'+id,source:'示範來源；未連網'},null,2))}</pre>${button('複製診斷','copy-diagnosis')}${button('重新讀取狀態','core-readback')}<p id="diagnostic-copy-error" class="form-error"></p></details>`,button('關閉','close-dialog'));}
function logsDialog(id=[...state.selected][0]){const r=rowBy(id);mountDialog(`${esc(r?.name||'實例')} · 啟動紀錄`,`<label for="log-kind">紀錄來源</label><p><select id="log-kind"><option value="console">伺服器一般輸出</option><option value="stdout">Host stdout</option><option value="stderr">Host stderr</option><option value="events">Host 事件</option><option value="exit">退出證據</option><option value="missing">缺少／讀取失敗示例</option></select></p><p class="hint">示範 run，已移除終端控制碼；不是原生終端畫面。</p><pre id="log-content">[示範] Java 程序已啟動\n[示範] 正在載入伺服器\n[示範] Done。這些不是本機實際紀錄。</pre>`,button('重新讀取','reload-log')+button('開啟紀錄資料夾','log-folder','title="離線預覽不開啟本機目錄"')+button('關閉','close-dialog'));}
function terminalWindow(){const id=[...state.selected][0],r=rowBy(id);if(!r)return;if(!state.connected||r.status!=='running')return messageBox('需要處理', '先核對實例已由管理器持有，並且正在運作。');const existing=popups.get(id);if(existing&&!existing.closed){existing.focus();return;}
 const p=window.open('','ih-wireframe-'+id,'popup,width=860,height=550');if(!p)return messageBox('需要處理', '瀏覽器阻擋示範視窗。允許此頁彈出視窗後再試；不會改動伺服器。');popups.set(id,p);
 p.document.open();p.document.write(`<!doctype html><html lang="zh-Hant"><meta charset="utf-8"><title>${esc(r.name)} · 示範後台</title><style>body{background:#142032;color:#e5edf9;font:16px/1.7 Consolas,"Microsoft JhengHei",monospace;padding:20px}input,button{font:inherit;padding:8px;background:#24364d;color:#fff;border:1px solid #8c9fbb}input{width:60%}:focus-visible{outline:3px solid #8bb4ff}pre{white-space:pre-wrap}p{font-family:"Segoe UI","Microsoft JhengHei",sans-serif}</style><h1 style="font-size:20px">${esc(r.name)}</h1><p>獨立觀看視窗示意。不是 ConPTY，也沒有連接任何程序。關閉此視窗不停止示範實例。</p><pre id="output">[示範] 目前正在監聽\n[示範] 真實 Tab 補全、編碼與換行需另做終端驗證。</pre><form id="command"><label for="input">示範命令（只接受 help）</label><p><input id="input" name="command" autocomplete="off"><button>送出</button></p></form></html>`);p.document.close();p.document.querySelector('form').addEventListener('submit',e=>{e.preventDefault();const input=p.document.querySelector('input');p.document.querySelector('pre').textContent+='\n'+(input.value==='help'?'[示範] 產品版會把輸入送往正確 run 的 pseudoconsole。':'[示範] 本線框不執行伺服器命令。');input.value='';});p.focus();
}

function settingsPage(){return state.tab==='templates'?state.templateTab:state.tab;}
function initialDraft(){if(state.tab==='appearance')return {...state.appearance};if(state.tab==='scan')return {allow:state.scanAllow,deny:state.scanDeny};if(state.tab==='initialization')return {...state.policies[state.audience],_timeEnabled:Object.hasOwn(state.policies[state.audience],'_initialTime'),_timeTicks:state.policies[state.audience]._initialTime??6000};if(state.tab==='world')return {...Object.fromEntries(policySeed.map(x=>[x[0],x[2]])),...(state.worldValues[state.target]||{}),_timeEnabled:false,_timeTicks:state.worldTimes?.[state.target]??6000};return {...(state.target?state.instanceSettings[state.target]||defaults:state.globals)};}
function changeSettingsTab(tab){guardLeave(()=>{applyTheme();if(state.tab!==tab)state.policyFilter='';state.tab=tab;state.draft=null;state.baseline=null;state.dirty=false;state.settingsError='';renderSettings();enhanceScrolls();});}
let fieldRevision=0;
function renderSettings(){
 const view=$('#view');view.className=(view.dataset.overlay?'overlay-scroll ':'')+'route-settings';
 if(!state.draft){state.draft=initialDraft();state.baseline=structuredClone(state.draft);}
 settingsPresentation.message=state.dirty?'有尚未儲存的變更':'';
 settingsPresentation.error=state.settingsError;
 const row=rowBy(state.target);
 const tabs=row?(row.core==='Velocity'?['runtime','restart','autorestart','terminal']:['runtime','restart','autorestart','terminal','world']):['appearance','scan','initialization','templates'];
 renderSettingsPage(view,{
  instance:row?.name,title:row?row.name+' · 實例設定':'應用程式設定',
  subtitle:row?'此實例獨立儲存啟動設定，不持續繼承其他實例或建立範本。':'設定介面與新建範本；新建範本的修改不會改動既有實例。',
  tabs:tabs.map(key=>({key,label:settingLabel(key,!!row)})),tab:state.tab,
  heading:settingLabel(state.tab,!!row),templates:state.tab==='templates'?templateNav():'',
  fields:settingsContent(),fieldRevision:++fieldRevision,
 });
 enhanceScrolls();
}
function editor(key,type,value,options=[],disabled=false){const attrs=`id="field-${key}" name="${key}" data-field="${key}" ${disabled?'disabled':''}`;if(type==='checkbox')return `<input type="checkbox" ${attrs} ${value?'checked':''}>`;if(type==='textarea')return `<textarea ${attrs} spellcheck="false">${esc(value)}</textarea>`;if(type==='select')return `<select ${attrs}>${options.map(v=>`<option ${v===value?'selected':''}>${esc(v)}</option>`).join('')}</select>`;return `<input ${attrs} type="${type==='decimal'?'number':type}" ${type==='number'?'min="0" step="1" inputmode="numeric"':type==='decimal'?'min="0" step="any" inputmode="decimal"':''} autocomplete="off" value="${esc(value)}">`;}
function settingLabel(key,instance){const labels={templates:'新建啟動範本',appearance:'顯示與語言',scan:'掃描目錄',initialization:'初始化預設'};return labels[key]||tabs[key];}
function templateNav(){return `<p class="hint">僅供之後建立實例複製。既有實例不連動；可在各實例內獨立修改。</p><div class="tabs" role="group" aria-label="新建啟動範本分類">${['runtime','restart','autorestart','terminal'].map(key=>`<button type="button" data-template-tab="${key}" aria-pressed="${state.templateTab===key}">${tabs[key]}</button>`).join('')}</div>`;}
function settingsContent(){const page=settingsPage();if(state.tab==='initialization'||state.tab==='world')return policyContent();if(state.tab==='scan')return scanContent();if(state.tab==='appearance')return appearanceContent();
 const fields=(descriptors[page]||[]).filter(([key])=>page!=='runtime'||key!=='java');let content=`<div class="fields">${fields.map(([key,label,type,options])=>`<div class="field"><label for="field-${key}">${label}</label><div class="editor">${editor(key,type,state.draft[key],options)}${state.target?`<div class="source"><span>此實例獨立設定</span>${button('還原本次修改','reset-runtime-field',`data-key="${key}"`)}</div>`:''}${key==='args'?'<span class="hint">每行為一個參數，不將整行交給 shell 解析。</span>':''}</div></div>`).join('')}</div>`;
 if(page==='runtime')content=`<section class="java-summary"><h3>Java 執行環境</h3><strong id="java-summary-name">${esc(javaRecords.find(r=>javaKey(r.path)===javaKey(state.draft.java))?.label||'手動指定的 Java')}</strong><label class="sr-only" for="field-java">Java 執行檔路徑</label>${editor('java','text',state.draft.java)}${button('選擇 Java','java-picker')}${button('核對 Java 與啟動參數','java-check')}<p id="java-check-result" class="${probeJava(state.draft.java).status==='ready'?'hint':'form-error'}" role="status">${esc(probeJava(state.draft.java).message)}</p></section>`+content+`<details><summary>預覽 Java 啟動參數</summary><pre id="java-command">${esc(javaCommand())}</pre></details>`;
 if(page==='terminal')content+=`<h3>標題預覽</h3><p id="title-preview" class="inset">${esc(titlePreview())}</p><p class="hint">點選佔位符可插入。取得不到的值會提示，不顯示成猜測資料。</p><div class="chips">${titleCatalog.map(entry=>button(`{${entry.key}}　${esc(entry.description)}`,'placeholder',`data-key="${entry.key}"`)).join('')}</div><p class="hint">已列出 1.0 的全部 28 個佔位符。只有已核對的示範資料會代入；其他顯示尚未核對。核心 build 是建立時記錄，不能冒充目前 JAR。</p>`;
 return content;
}
function javaPicker(){
 state.javaPending=state.draft.java;
 const dialog=mountDialog('選擇 Java','',button('取消','close-dialog')+button('使用此版本','java-use','class="primary"'));
 renderJavaPicker($('.dialog-body',dialog),{choices:javaChoices,invalid:javaCatalog.invalid,pending:state.javaPending,message:probeJava(state.javaPending).message});
 enhanceScrolls();
 dialog.id='java-picker-dialog';
 $('button[data-action=java-use]',dialog).disabled=probeJava(state.javaPending).status!=='ready';
}
function choosePendingJava(path){state.javaPending=path;$$('#java-picker-dialog [data-java]').forEach(b=>b.setAttribute('aria-pressed',String(b.dataset.java===path)));$('#java-pending-result').textContent=probeJava(path).message;$('[data-action=java-use]',$('#java-picker-dialog')).disabled=probeJava(path).status!=='ready';}
function javaCommand(){return `${state.draft.java} -Xms${state.draft.xms} -Xmx${state.draft.xmx} ${state.draft.gc==='預設'?'':`[${state.draft.gc}]`} ${state.draft.args} -jar server.jar nogui`;}
function titlePreview(){const r=rowBy(state.target)||seed[4];const values={instance:r.name,port:r.port,type:r.core,minecraft_version:r.version,project:r.project,kind:r.kind,path:r.path};return String(state.draft.title||'').replace(/\{([^}]+)\}/g,(_,key)=>values[key]??(titleCatalog.some(x=>x.key===key)?'[尚未核對]':`[未知佔位符：${key}]`));}
function policyContent(){
 const world=state.tab==='world',r=rowBy(state.target),old=world&&r?.version==='1.8.8';
 const normalizeSearch = value => value.toLowerCase().replace(/[-_\s]+/g,' ').trim();
 const query = normalizeSearch(state.policyFilter);
 const matches = (...values) => values.some(value=>normalizeSearch(value).includes(query));
 const relevant=policySeed.filter(([key,label])=>{
  const meta=policyMeta[key];if(key==='examplePermission'&&world)return false;
  if(world){
   const inTab = state.worldTab==='rules' ? meta.family==='rules' : meta.family!=='rules';
   return inTab&&(!query||matches(key,label,meta.source));
  }
  if(query)return matches(key,label,meta.source);
  return meta.family===state.policyCategory&&(state.advancedPolicy||!meta.advanced);
 });
 const header=world?`<div class="tabs" role="group" aria-label="世界設定類型">${button('設定檔','world-files',`aria-pressed="${state.worldTab==='files'}"`)}${button('世界規則','world-rules',`aria-pressed="${state.worldTab==='rules'}"`)}</div><p class="hint">${state.worldTab==='files'?'需要停止後套用；只改此實例的設定檔。':'需要已核對的運作中實例；只修改這個世界，時間另外選擇。'}</p>`:
 `<p class="hint">這是新建實例的預設，不會回寫既有世界。</p><nav class="policy-categories" aria-label="初始化設定二級分類">${Object.entries(policyGroups).map(([key,label])=>`<button type="button" data-policy-category="${key}" aria-pressed="${!query&&state.policyCategory===key}">${label}</button>`).join('')}</nav>`;
 const inventoryLink='<p class="hint"><a href="settings-inventory.html" target="_blank" rel="noopener">查看各核心與版本的完整政策欄位盤點（另開分頁）</a></p>';
 const paperNotice=!world&&!query&&state.policyCategory==='paper'?'<div class="notice">Paper 設定只會套用到支援的 Paper 核心／版本；Spigot 不會讀取這些檔案。Paper 全域與世界預設仍分開儲存。</div>':'';
 const tools=`<div class="policy-tools"><label for="policy-search">${world?'搜尋本頁':'搜尋所有分類'}</label><input id="policy-search" type="search" autocomplete="off" name="policy-search" placeholder="搜尋設定名稱、鍵名或檔名" value="${esc(state.policyFilter)}">${!world?`<label class="check-label"><input type="checkbox" id="policy-advanced" ${state.advancedPolicy?'checked':''}>顯示本分類進階設定</label>${button('檢視各版本套用結果','version-preview')}`:''}</div>`;
 const rows=relevant.map(([key,label,value,type,support])=>{
  const meta=policyMeta[key];const unsupported=world&&((old&&['simulation','cramming','sleepPercent','respawn','insomnia'].includes(key))||(meta.family==='paper'&&!['Paper','Folia'].includes(r?.core))||meta.newOnly);
  const requiresVerification=key==='sync-chunk-writes';
  const unknown=requiresVerification||world&&r?.status==='unknown';const disabled=unsupported||unknown;
  const reason=requiresVerification?'尚未核對：需確認版本與 JVM 旗標':unknown?'目前值尚未核對':meta.newOnly&&world?'只在建立新世界時設定':meta.family==='paper'&&world&&!['Paper','Folia'].includes(r?.core)?'此核心不適用：需要 Paper 支援':unsupported?'此版本略過：沒有對應設定':world?(state.draft[key]===state.baseline[key]?'未修改':`${state.baseline[key]} → ${state.draft[key]}`):meta.family==='paper'?'Paper 專用；Spigot 略過':support;
  const options=key==='difficulty'?['和平','簡單','普通','困難']:key==='mode'?['生存','創造','冒險','旁觀']:['基本超平坦','一般'];
  return `<tr data-policy-name="${esc(label+' '+meta.source)}" class="${disabled?'unsupported':''}"><td><label for="field-${key}">${label}</label><span class="policy-source">${esc(meta.source)}</span></td><td>${editor(key,type,unknown?'':state.draft[key]??value,options,disabled)}</td><td class="hint" data-difference="${key}">${esc(reason)}</td><td>${button('還原','reset-field',`data-key="${key}" ${disabled?'disabled':''}`)}</td></tr>`;
 }).join('');
 const time=(query?!matches(world?'另外修改遊戲時間':'自訂新世界初始時間','initialTime','initial time','time')||(world&&state.worldTab!=='rules'):world?state.worldTab!=='rules':state.policyCategory!=='world')?'':`<p class="time-policy"><label><input type="checkbox" id="${world?'world-time':'init-time'}" ${state.draft._timeEnabled?'checked':''}>${world?'另外修改遊戲時間':'自訂新世界初始時間'}</label><input type="number" id="${world?'world-ticks':'initial-ticks'}" name="ticks" aria-label="${world?'遊戲':'初始'}時間（tick）" min="0" max="23999" value="${state.draft._timeTicks}" ${state.draft._timeEnabled?'':'disabled'}></p>`;
 return header+paperNotice+tools+inventoryLink+(query?`<p class="hint">${world?'本頁搜尋結果':'跨分類搜尋結果'}，共 ${relevant.length+(time?1:0)} 項。</p>`:'')+`<div class="table-wrap"><table class="policy-table"><thead><tr><th scope="col">設定／實際來源</th><th scope="col">${world?'目前設定':'建立時預設'}</th><th scope="col">${world?'差異／可用性':'適用範圍'}</th><th scope="col">還原</th></tr></thead><tbody>${rows}</tbody></table></div>${!relevant.length&&!time?'<p class="empty">沒有符合的設定。</p>':''}${time}${world?button('結果與診斷','world-results'):'<p class="hint">欄位值為線框示範；正式版須按核心與版本核對完整能力目錄。未指定值保留原版預設，不用第一個版本猜值。</p>'}`;
}
function appearanceContent(){return `<p class="hint">外觀可立即預覽；儲存保留，離開或還原回復。本線框以繁體中文審查，語言完整英文稿另行驗收。</p><div class="fields">${[['theme','外觀',['system','light','dark'],['跟隨系統','淺色','深色']],['accent','清單選取色',['system','harbor'],['跟隨系統（紫色示意）','InstanceHarbor']],['language','介面語言',['zh-TW','en','system'],['繁體中文','English','跟隨系統']],['time','時間格式',['local24','local12','utc'],['本地 24 小時制','本地 12 小時制','UTC']]].map(([key,label,values,labels])=>`<div class="field"><label for="field-${key}">${label}</label><div class="editor"><select id="field-${key}" data-field="${key}" name="${key}">${values.map((v,i)=>`<option value="${v}" ${state.draft[key]===v?'selected':''}>${labels[i]}</option>`).join('')}</select></div></div>`).join('')}</div><p class="inset" id="appearance-preview">時間預覽：${esc(readableTime())}</p><p class="hint">瀏覽器不能可靠取得 Windows 強調色，紫色只是線框示例；產品需由桌面環境讀取。</p>`;}
function scanContent(){return `<p class="hint">只管理使用者登記的伺服器目錄，不是玩家白名單，也不管理 Agent 工作目錄。</p><p class="path-base">相對路徑基準（示範）<br><code>${esc(demoBase)}</code></p><p class="hint">相對路徑：<code>.\\instances\\Human</code><br>絕對路徑：<code>C:\\ExampleServers\\instances\\Velocity</code></p><div class="form-row"><label for="field-allow">允許目錄（每行一個路徑）<textarea id="field-allow" data-field="allow" name="allow" spellcheck="false">${esc(state.draft.allow)}</textarea></label><label for="field-deny">排除目錄（優先）<textarea id="field-deny" data-field="deny" name="deny" spellcheck="false">${esc(state.draft.deny)}</textarea></label></div>${button('預覽顯示範圍','scan-preview')}<p class="hint">不使用 Human／SurvivalSample 等用途名稱當別名。預覽解析後的完整路徑，隱藏仍運作的實例不會停止或刪除它。</p>`;}
function markDirty(){state.dirty=Object.keys(state.draft||{}).some(key=>JSON.stringify(state.draft[key])!==JSON.stringify(state.baseline?.[key]));settingsPresentation.message=state.dirty?'有尚未儲存的變更':'';}
function editField(input){const key=input.dataset.field;state.draft[key]=input.type==='checkbox'?input.checked:input.type==='number'?Number(input.value):input.value;markDirty();const diff=$(`[data-difference="${key}"]`);if(diff&&state.tab==='world')diff.textContent=state.draft[key]===state.baseline[key]?'未修改':`${state.baseline[key]} → ${state.draft[key]}`;
 if(settingsPage()==='terminal')$('#title-preview').textContent=titlePreview();if(settingsPage()==='runtime'&&$('#java-command')){$('#java-command').textContent=javaCommand();const result=probeJava(state.draft.java);$('#java-summary-name').textContent=javaRecords.find(r=>javaKey(r.path)===javaKey(state.draft.java))?.label||'手動指定的 Java';$('#java-check-result').textContent=result.status==='ready'?'已選擇可用示範安裝；儲存與啟動前會再次核對。':result.message;$('#java-check-result').className=result.status==='ready'?'hint':'form-error';}if(state.tab==='appearance'){applyTheme(state.draft.theme,state.draft.accent);$('#appearance-preview').textContent=state.draft.language==='en'?'English 預覽入口已選取；本輪線框文字仍以繁體中文審查。':`時間預覽：${readableTime()}`;}
}
function saveSettings(){state.settingsError='';let error='';if(state.scenario==='conflict'){state.scenario='normal';$('#scenario').value='normal';error='其他視窗已修改設定。草稿保留；請重新核對後再儲存。';}else if(!state.connected)error='管理服務未連線。草稿已保留，可重新連線後再試。';else if(['runtime','restart','autorestart'].includes(settingsPage())&&Object.entries(state.draft).some(([k,v])=>typeof v==='number'&&(!Number.isInteger(v)||v<0)))error='數值必須是零或正整數。請修正欄位後重試。';else if(settingsPage()==='runtime'&&!/java(?:\.exe)?$/.test(state.draft.java||''))error='請指定 Java 執行檔，例如 C:\\Java\\bin\\java.exe。';;if(state.tab==='scan'){try{for(const line of (state.draft.allow+'\n'+state.draft.deny).split(/\r?\n/).filter(Boolean))resolveDemoPath(line);}catch(e){error=e.message;}}
 if(!error&&settingsPage()==='runtime'&&probeJava(state.draft.java).status!=='ready')error=probeJava(state.draft.java).message;
 if(!error&&['initialization','world'].includes(state.tab)&&state.draft._timeEnabled&&(!Number.isInteger(state.draft._timeTicks)||state.draft._timeTicks<0||state.draft._timeTicks>23999))error='時間必須為 0 至 23999 的整數 tick。';
 if(!error&&settingsPage()==='terminal'&&(state.draft.title.length>256||/[\x00-\x1f]/.test(state.draft.title)||[...state.draft.title.matchAll(/\{([^}]+)\}/g)].some(m=>!titleCatalog.some(x=>x.key===m[1]))))error='標題含未知佔位符、控制字元或超過 256 字；請使用下方目錄。';
 if(error){state.settingsError=error;settingsPresentation.error=error;const e=$('#form-error');e.tabIndex=-1;e.focus();return;}
 if(state.tab==='world'){if(!state.draft._timeEnabled&&!Object.entries(state.draft).some(([k,v])=>!k.startsWith('_')&&v!==state.baseline[k])){settingsPresentation.message='沒有需要套用的變更';return;}const r=rowBy(state.target);if((state.worldTab==='files'&&r.status!=='stopped')||(state.worldTab==='rules'&&r.status!=='running')){state.settingsError=state.worldTab==='files'?'請先正常停止實例；本畫面不會自動停止。':'需要正在運作且已核對的工作階段；不會自動啟動。';settingsPresentation.error=state.settingsError;return;}return confirmDialog('套用世界設定？',`<p>只套用本頁實際修改項目${state.draft._timeEnabled?'，另含遊戲時間變更':''}。不會重啟。</p>`,'套用',()=>commitSettings());}
 commitSettings();
}
function commitSettings(){if(state.tab==='appearance'){state.appearance={...state.draft};applyTheme();}else if(state.tab==='scan'){state.scanAllow=state.draft.allow;state.scanDeny=state.draft.deny;state.scanActive=true;}else if(state.tab==='initialization'){const values={...state.draft};delete values._timeEnabled;delete values._timeTicks;if(state.draft._timeEnabled)values._initialTime=state.draft._timeTicks;else delete values._initialTime;state.policies[state.audience]=values;}else if(state.tab==='world'){const changes=Object.fromEntries(Object.entries(state.draft).filter(([k,v])=>!k.startsWith('_')&&v!==state.baseline[k]));state.worldValues[state.target]={...(state.worldValues[state.target]||{}),...changes};if(state.draft._timeEnabled){state.worldTimes??={};state.worldTimes[state.target]=state.draft._timeTicks;}const record=newRecord('套用世界政策',[state.target],'示範已套用');record.detail=Object.keys(changes).join('、')+(state.draft._timeEnabled?'；遊戲時間 '+state.draft._timeTicks+' tick':'')||'沒有設定差異；不送出寫入。';}else if(state.target){state.instanceSettings[state.target]={...state.draft};}else state.globals={...state.draft};state.baseline=structuredClone(state.draft);state.dirty=false;state.revision++;settingsPresentation.error='';settingsPresentation.message='已儲存示範設定';announce('已儲存示範設定，真實設定未變。');}
function renderWizard(){const w=state.wizard,importing=w.mode==='import';const name=importing?'匯入已有伺服器':'建立實例';let body='';if(w.step===0)body=`<div class="form-row"><label>實例名稱<input id="wizard-name" name="instanceName" value="${esc(w.name)}" autocomplete="off"></label><label>用途<select id="wizard-purpose"><option>Human</option><option>SurvivalSample</option></select></label></div><div class="form-row"><label>核心<select id="wizard-core"><option>Paper</option><option>Folia</option><option>Spigot</option><option>Velocity</option></select></label><label>Minecraft／核心版本<select id="wizard-version"><option>26.3</option><option>26.2</option><option>26.1.2</option><option>1.8.8</option></select></label></div><div class="form-row"><label>實例位置<input id="wizard-path" name="path" value="${esc(w.path)}" autocomplete="off"></label><label>Port<input id="wizard-port" type="number" min="1024" max="65535" name="port" value="${w.port}"></label></div><p class="hint">路徑與核心只作示範，不會讀寫本機。本介面不建立或管理 Agent 測服。</p>`;
 else if(w.step===1)body=importing?importReadback(w):`<div class="form-row"><label>Java<select id="wizard-java">${javaChoices.map(x=>`<option ${w.java===x[0]?'selected':''}>${x[0]}</option>`).join('')}</select></label><label>初始化政策<select id="wizard-policy"><option>套用新建實例預設</option><option>只保留物品，其餘使用原版</option><option>使用各版本原版設定</option></select></label></div><p class="inset">建立前預覽：不支援的版本設定略過並記錄；既有世界不套用新建預設。</p>`;
 else body=`<h3>確認${importing?'匯入':'建立'}</h3><dl class="core-facts"><dt>名稱</dt><dd>${esc(w.name)}</dd><dt>核心</dt><dd>${esc(w.core)} ${esc(w.version)}</dd><dt>位置</dt><dd class="mono">${esc(w.path)}</dd><dt>Port</dt><dd>${w.port}</dd><dt>啟動</dt><dd>完成後保持停止</dd><dt>Java</dt><dd>${esc(importing?'首次啟動前另行核對；不修改既有啟動檔':w.java||'尚未選擇')}</dd><dt>政策</dt><dd>${esc(w.policy||'不套用新建政策')}</dd></dl><p class="hint">本次只新增記憶體內示範項目，不下載核心或建立資料夾。${w.java?.includes('8u')&&!w.version.startsWith('1.')?' 示範相容性提醒：所選 Java 與新版核心可能不相容，請回上一步核對；本線框不會執行它。':''}</p>`;

 const content=`<p class="hint">完成前不建立或更改資料。關閉此流程會詢問是否放棄草稿。</p><ol class="steps" aria-label="流程步驟">${['基本資料',importing?'唯讀核對':'Java 與政策','確認'].map((x,i)=>`<li class="${w.step===i?'current':''}" ${w.step===i?'aria-current="step"':''}>${x}</li>`).join('')}</ol>${body}<div id="wizard-error" class="form-error" role="alert"></div>`;
 const actions=button('取消','wizard-cancel')+(w.step?button('上一步','wizard-back'):'')+button(w.step===2?`確認${importing?'匯入':'建立'}`:'下一步','wizard-next',`class="primary" ${importing&&w.step===1&&importProblem()?'disabled aria-describedby="import-problem"':''}`);
 let dialog=$('#wizard-dialog');
 if(!dialog){dialog=mountDialog(name,content,actions);dialog.id='wizard-dialog';dialog.classList.add('wizard-modal');dialog.addEventListener('cancel',event=>{event.preventDefault();requestWizardClose();});dialog.addEventListener('close',()=>{if(state.wizardDialog===dialog){state.wizardDialog=null;state.wizard=null;state.wizardDirty=false;}});state.wizardDialog=dialog;}
 else{$('.dialog-body',dialog).innerHTML=content;$('.dialog-actions',dialog).innerHTML=actions;}
 if(!w.step){$('#wizard-purpose').value=w.kind;$('#wizard-core').value=w.core;$('#wizard-version').value=w.version;}
 if(w.step===1&&!importing)$('#wizard-policy').value=w.policy;
 if(w.step===1&&importing&&$('#import-ack'))$('#import-ack').checked=!!w.ack;
 const focus=$('input:not([type=checkbox]),select',dialog)||$('h2',dialog);focus.tabIndex=focus.tagName==='H2'?-1:focus.tabIndex;focus.focus();enhanceScrolls();
}
function captureWizard(){const w=state.wizard;if(!w)return;
 if(w.step===0&&$('#wizard-name')){w.name=$('#wizard-name').value.trim();w.path=$('#wizard-path').value.trim();w.port=Number($('#wizard-port').value);w.kind=$('#wizard-purpose').value;w.core=$('#wizard-core').value;w.version=$('#wizard-version').value;}
 if(w.step===1&&w.mode==='create'){w.java=$('#wizard-java').value;w.policy=$('#wizard-policy').value;}
 if(w.step===1&&w.mode==='import'&&$('#import-ack'))w.ack=$('#import-ack').checked;
}
function finishWizardClose(){const dialog=$('#wizard-dialog');state.wizardDirty=false;state.wizard=null;state.wizardDialog=null;if(dialog){dialog.close();dialog.remove();}}
function requestWizardClose(){captureWizard();if(state.wizardDirty)return confirmDialog('放棄建立／匯入草稿？','尚未完成確認，不會寫入資料或啟動伺服器。取消可繼續編輯。','放棄變更',finishWizardClose);finishWizardClose();}
function wizardNext(){const w=state.wizard;if(!w)return;captureWizard();if(w.mode==='import'&&w.step===0&&importDuplicate(w))return;if(w.step===0){w.name=$('#wizard-name').value.trim();w.path=$('#wizard-path').value.trim();w.port=Number($('#wizard-port').value);w.kind=$('#wizard-purpose').value;w.core=$('#wizard-core').value;w.version=$('#wizard-version').value;if(!w.name||!w.path||state.rows.some(r=>r.id!==w.existingId&&r.managed!==false&&!r.removed&&r.hasManagementData!==false&&r.port===w.port)||!Number.isInteger(w.port)||w.port<1024||w.port>65535){$('#wizard-error').textContent='請輸入名稱、位置與有效且未重複的 Port。資料已保留。';return;}}
 if(w.step===1&&w.mode==='create'){w.java=$('#wizard-java').value;w.policy=$('#wizard-policy').value;}if(w.step===1&&w.mode==='import'&&importProblem()){wizardError(importProblem());return;}if(w.step===1&&w.mode==='import'&&!$('#import-ack')?.checked){$('#wizard-error').textContent='請先確認唯讀檢查與管理範圍。';return;}if(w.step===2&&w.mode==='import')return completeImport(w);
 if(w.step===2&&state.scenario==='create-download-failure'){
  state.scenario='normal';
  $('#scenario').value='normal';
  state.wizardDirty=true;
  $('[data-action=wizard-next]', $('#wizard-dialog')).textContent='重試建立';
  wizardError('核心下載失敗：示範來源暫時無回應。草稿已保留，尚未建立實例。可重試建立，或回上一步修改。');
  return;
 }
 if(w.step===2){const id='new-'+Date.now();state.rows.push({id,name:w.name,project:w.kind==='SurvivalSample'?'生存世界':'新建實例',kind:w.kind,core:w.core,version:w.version,port:w.port,path:w.path,status:'stopped',build:18,latest:18,pin:null});newRecord('建立實例',[id],'已完成').detail='已建立示範實例，保持停止。';state.instanceSettings[id]={...state.globals,java:javaChoices.find(x=>x[0]===w.java)?.[1]||state.globals.java};state.rows[state.rows.length-1].initializationPolicy=w.policy||'保留既有政策';finishWizardClose();state.selected=new Set([id]);state.query='';state.filter='all';state.route='instances';render();announce('已建立示範實例，保持停止。');return;}w.step++;renderWizard();}

function remainingReview(){return `<section class="review-roadmap" aria-labelledby="remaining-review-title"><h2 id="remaining-review-title">接下來只確認未定案流程，不重審已接受版面</h2><p>使用者已表示目前版除底部密度外沒有問題。本輪匯入／管理登記／舊版接入及排程示意待確認；其他既有線框不因下表重新變成未驗收。來源：需求頁待決事項與49 組 1.0 對照與新增管理需求。</p><figure><div class="sequence" role="img" aria-label="先確認全域操作與資料邊界，再補例外畫面，之後規劃隔離技術驗證；目前沒有產品實作授權"><div><strong>先決定操作邊界</strong><p>全部操作、匯入、移除與資料保存</p></div><div><strong>再補例外畫面</strong><p>初次接入、失敗修正與完整設定範圍</p></div><div><strong>最後證明能實作</strong><p>終端、效能、Java、遷移的隔離驗證計畫</p></div></div><figcaption>Fig. 1　版面確認、功能決策與真實程式驗證分開完成。</figcaption></figure><h3>仍需要妳決定或確認的部分</h3><div class="table-wrap"><table><thead><tr><th>優先順序／項目</th><th>尚未定案的問題</th><th>建議與需要補看的畫面</th></tr></thead><tbody><tr><td>先：全部啟動／停止／重啟</td><td>方向已確認：保留全部啟動／正常停止／重啟，與所選操作分開。所選實例批次已確認保留。</td><td>本輪已加入首頁「全部操作」選單與獨占確認，待使用者確認實際操作感受；保留結果與取消剩餘。</td></tr><tr><td>先：匯入、移除與舊版接入</td><td>資料邊界已確認：匯入保留原位置；移除保留設定或只清除管理資料；伺服器檔案保留。</td><td>本輪候選已補唯讀匯入、兩種移除、重新加入、舊版持有阻擋與接入還原。請由首頁「匯入已有伺服器」及左側「接入舊版資料」試看。</td></tr><tr><td>再：紀錄與任務追蹤</td><td>保留期限、清除與匯出範圍；程式重開後如何找回進度／未確認結果。</td><td>建議失敗與未確認結果不自動消失，保留可查證資料；容量與期限待資源估算。需補長紀錄、找不到檔案、重開後恢復畫面。</td></tr><tr><td>再：建立與匯入例外</td><td>匯入的權限不足、核心無法辨識、重複位置與外部程序阻擋已補；新建的核心下載失敗已補示範。從預覽工具選「建立：核心下載失敗一次」，再走到確認。</td><td>建議保留輸入，錯誤放步驟旁；需要決定才開 Dialog。現有 Dialog 排版已接受，補例外而非重做整個精靈。</td></tr><tr><td>再：初始化與世界設定完整範圍</td><td>分類與互動已確認；目前只畫代表欄位，尚須核對完整目錄、常用項目與進階項目的收納。</td><td>建議沿用共同設定介面＋版本能力判定，不逐版本造一套。補完整欄位清單、搜尋無結果、不支援／未知的例外。</td></tr><tr><td>後：語言、縮放與無障礙驗收</td><td>繁中／English／跟隨系統已定；完整英文文案、長字串、Windows DPI 與輔助技術仍未驗收。</td><td>建議版面穩定後一次補多語系、鍵盤順序、縮放與讀屏；不能由目前的繁中線框宣稱通過。</td></tr></tbody></table></div><h3>不必重問版面，但尚需技術驗證</h3><ul><li>ConPTY 換行／重複與原生後台 5～10 秒延遲：先量測程序與資料傳遞，不假設 Rust 自動解決。</li><li>Java 路徑存在、可執行性、mise alias 重新解析與啟動前核對：現在只有契約與模擬。</li><li>併發上限、正常停止逾時與停止後緩衝：用隔離環境與負載證據決定；模擬併發 2 不是正式預設。</li><li>Rust 服務生命週期、核心更新來源、持續任務與 1.0 資料接入：另做隔離驗證及回復證據。</li><li>前端元件庫、安裝包、更新與共用 JAR：另列技術與交付階段，共用 JAR 不混入目前 UI 修改。</li></ul><details><summary>這一輪的建議下一步與授權邊界</summary><p>全部操作方向已確認，本輪提供離線互動驗收。本輪匯入／移除與舊版接入流程已補為離線候選，待使用者確認。下一項可討論紀錄保留、匯出與工作恢復；尚未開始該工作或產品實作。</p><p>已確認的主頁選取／排序／釘選、設定獨立保存、建立時範本、背景靜默查詢、Java Dialog、一般回饋／訊息 Dialog 方向保留。剩下的技術未驗證不代表妳必須重新驗收每個現有頁面。</p><p>依據：<a href="./v2-requirements-review.html#pending">需求頁：待決事項</a>、<a href="feature-inventory.json">功能對照與管理需求</a>、<a href="./instanceharbor-future-plan.md">專案 roadmap：2.0 需求整理</a>。歷史 1.0 roadmap 不當作 2.0 完成證據。</p></details></section>`;}
function renderReview(){clearPreviewPage($('#view'));$('#view').innerHTML=pageHead('線框審查','這是完整操作路徑示意，尚未接入 Tauri、Rust、ConPTY 或任何本機資料。',button('回到示範','back-instances','class="primary"'))+`<section class="sheet">${remainingReview()}<h2>最新修訂：統一匯入與精簡主頁</h2><p>搜尋右側只保留狀態、刷新、核心維護與全部操作；核心選單不影響所選操作範圍。實例詳細資訊依身分、觀察狀態、主要操作、最近操作與診斷排序。啟動／正常停止共用位置，釘選在名稱旁，上下移動與移除管理收在管理操作。未選取與多選均保持右側寬度。</p><p>首頁「匯入已有伺服器」保留路徑與輸入，唯讀辨識完成才登記；頂部預覽情境可試權限不足、核心無法辨識與外部程序持有。移除入口在單台「管理操作」；重新加入統一走「匯入已有伺服器」。有保留設定時讀回並將 removed 改為 false，沒有時建立新管理設定，不另設管理清單。左側「接入舊版資料」可試運作中阻擋、閒置設定預覽及接入還原。批次操作紀錄可看到停止並行、共用啟動名額的排程示意。</p><p>整列選取、欄位對齊與穩定狀態色已更新；用途 combo 先隱藏，Agent 不在一般介面管理。初始化分為伺服器、Spigot／Bukkit、Paper、世界、遊戲規則五類，另有常用／進階，掃描使用相對或絕對路徑。實例獨立儲存，建立範本只在新建時複製。</p><p><a href="./refinement-notes.md">完整修訂與檔案來源依據</a></p><h3>精簡底部與回饋</h3><p>操作列保留按鈕上下各約 8px，狀態列使用較大字體與單行高度。Java 重新探索完成與阻擋原因使用訊息 Dialog；一般完成訊息留在操作附近，不再從右下角浮出。背景核對仍保持安靜。</p><h3>固定操作區與 Java 選擇</h3><p>設定頁的取消與儲存固定在內容底部，與服務狀態列分開；只捲動設定內容。首頁先容納視窗高度，清單或詳情放不下才各自捲動，小視窗保留必要外層捲動。Java 摘要放在 JVM 參數前，安裝清單另開獨占選擇視窗；使用此版本只修改草稿，仍須儲存。取消選擇不改原值。</p><h3>前次五項調整</h3><ul><li>世界與遊戲規則分開；世界放生成與資料設定，遊戲規則放 gamerule。</li><li>Java 必須通過檔案存在與實際執行兩道檢查。mise 以解析後執行檔去重，不以 local-25 名稱當唯一鍵；失效舊路徑另外列出且不可選，不自動替換實例設定。此頁只模擬結果，沒有執行 Java。</li><li>建立與匯入使用獨占 dialog；背景主畫面不能操作，取消保留或確認放棄草稿。核心維護、確認、備份診斷亦使用 dialog；長設定與操作紀錄保留內頁。</li><li>頂部虛線工具列僅供預覽；左側主導覽為正式產品設計候選。實例設定歸屬實例，應用程式設定不再顯示返回實例。</li><li>狀態使用同組 Lucide SVG 圖示與固定尺寸，標題、圖示與文字垂直置中，保留語意色。</li></ul><h2>建議試用順序</h2><ol><li>選一台、Ctrl／Shift 多選、搜尋隱藏選取；核對右側操作目標。</li><li>啟動已停止實例，再開獨立原生後台示範視窗；關閉觀看窗，實例仍在運作。</li><li>到實例設定修改記憶體，取消、儲存與還原本次修改；新建範本不回寫既有實例，儲存衝突保留草稿。</li><li>打開核心更新，手動查詢、更新、備份回復與結果未確認核對。</li><li>全域初始化與世界政策分開操作；未知／不支援的欄位不能誤當有效數值。</li><li>建立／匯入、掃描範圍預覽、斷線與首次接入預覽；全部只影響示範資料。</li></ol><div class="screen-list">${Object.entries(tabs).map(([key,label])=>button(label,'review-screen',`data-tab="${key}"`)).join('')}${button('建立','create')}${button('匯入','import')}${button('首次接入','onboarding')}${button('重設失敗範例','failure-example')}</div><h2 class="screen-section">1.0 → 2.0 功能對照</h2><p class="hint">1.0 來源：feat-gui-i18n@${inventory.baseline.slice(0,7)}。原始碼有入口不等於真實功能已驗收；2.0 當前只有線框。</p><div class="table-wrap"><table class="review-table"><thead><tr><th>功能</th><th>1.0 證據</th><th>2.0 處理</th></tr></thead><tbody>${inventory.features.map(f=>`<tr><td>${esc(f.name)}</td><td><code>${esc(f.v1Evidence)}</code><p class="hint">${{present:'GUI 已有入口','backend-only':'僅確認後端／client 方法',absent:'主 GUI 未見入口','not-audited':'本輪新增需求，不主張 1.0 缺漏'}[f.v1State]}</p></td><td><strong>${{retain:'保留',redesign:'重新安排',new:'新增',deferred:'暫緩',pending:'仍待決'}[f.decision]}</strong><p class="hint">${esc(f.note)}</p>${button('查看對應線框','inventory-screen',`data-screen="${f.v2Screen}"`)}</td></tr>`).join('')}</tbody></table></div><details><summary>設計依據與未驗證限制</summary><p>主要流程依本對話已確認需求。UI UX Pro Max 提供批次操作、可見焦點、錯誤復原與資料表密度建議。frontend-design 用於兩遍設計審視；Awesome Design MD 只借用 Linear 的層級與分隔，不複製宣傳頁。Web Design Guidelines 使用 pinned-1 本機規則。</p><p>模擬併發 2、短延時與假資料不是產品預設或效能證據。語言選單展示入口與儲存方式，本輪不提供完整英文稿。瀏覽器無法替代 Windows 系統強調色、title bar、DPI、輔助技術與真實終端驗收。</p><p>全部生命週期操作已確認保留在「全部操作」選單；與右側所選操作分開，這次只提供離線驗收。</p></details><p><a href="./v2-requirements-review.html">已確認需求</a>　<a href="./design-notes.md">設計計畫與取捨</a>　<a href="./validation.json">Agent 檢查記錄</a>　<a href="./review-notes.md">驗證範圍與真人清單</a></p></section>`;}
function inventoryScreen(screen){if(screen==='instances')return navigate('instances');if(screen==='activity')return navigate('activity');if(screen==='create'||screen==='import')return beginWizard(screen);if(screen==='onboarding')return onboarding();if(['core','backups','logs','terminal'].includes(screen)){state.selected=new Set(['drop']);state.route='instances';render();if(screen==='core')return coreDialog('drop');if(screen==='backups')return backupsDialog('drop');if(screen==='logs')return logsDialog('drop');state.selected=new Set(['example-survival']);renderRows();renderDetail();$('[data-action=console]').focus();return;}navigate('settings',{tab:screen==='world'?'world':screen,target:screen==='world'?'drop':null});}
function beginWizard(kind){if($('#wizard-dialog')){$('#wizard-dialog').focus();return;}guardLeave(()=>{state.dirty=false;state.wizardDirty=false;state.wizard={mode:kind,step:0,name:kind==='import'?'匯入的遊玩世界':'新的測試實例',path:'instances/Human/New/new-instance',port:12010,kind:'Human',core:'Paper',version:'26.3',java:kind==='import'?'':javaChoices[0][0],policy:kind==='import'?'不套用初始化政策':'套用新建實例預設'};renderWizard();});}
function onboarding(){return legacyOverview();}

export function startPreview(){
 const restored=readActivity();
 if(restored.length){state.records=restored;state.route='activity';}
 window.addEventListener('pagehide',()=>saveActivity(state.records,state.tasks));

document.addEventListener('click',event=>{
 $$('.action-menu[open]').forEach(menu=>{if(!menu.contains(event.target))menu.open=false;});
 const category=event.target.closest('[data-policy-category]');if(category){state.policyCategory=category.dataset.policyCategory;state.policyFilter='';renderSettings();enhanceScrolls();return;}
 const template=event.target.closest('[data-template-tab]');if(template){state.templateTab=template.dataset.templateTab;renderSettings();enhanceScrolls();return;}
 const nav=event.target.closest('[data-nav]');if(nav){event.preventDefault();return navigate(nav.dataset.nav);}
 const sort=event.target.closest('[data-sort]');if(sort){const key=sort.dataset.sort;if(state.sort===key)state.direction=state.direction===1?-1:state.direction===-1?0:1;else{state.sort=key;state.direction=1;}renderInstances();$(`[data-sort="${key}"]`).focus();return;}
 const checkbox=event.target.closest('[data-select]');if(checkbox){event.preventDefault();return selectRow(checkbox.dataset.select,event,true);}
 const rowButton=event.target.closest('[data-rowselect]');if(rowButton)return selectRow(rowButton.dataset.rowselect,event);
 const tab=event.target.closest('[data-tab]');if(tab&&!tab.dataset.action)return changeSettingsTab(tab.dataset.tab);
 const java=event.target.closest('[data-java]');if(java){if(java.closest('#java-picker-dialog'))return choosePendingJava(java.dataset.java);state.draft.java=java.dataset.java;$('#field-java').value=state.draft.java;editField($('#field-java'));$$('[data-java]').forEach(b=>b.setAttribute('aria-pressed',String(b===java)));return;}
 const a=event.target.closest('[data-action]');if(!a)return;const action=a.dataset.action,id=a.dataset.id;const ownMenu=a.closest('.action-menu');if(ownMenu){ownMenu.open=false;$('summary',ownMenu).focus();}
 if(action==='close-dialog'){const d=a.closest('dialog');d.close();d.remove();return;}
 if(action==='export-records'){
  const records=state.records.filter(record=>state.recordFilter==='all'||/核心/.test(record.action));
  return confirmDialog('匯出操作紀錄？',`<p>將匯出目前範圍內的 ${records.length} 筆示範紀錄。匯出只包含工作、目標與結果摘要。</p>`,'匯出 JSON',()=>exportActivity(records));
 }
 if(action==='confirm-current'){const dialog=a.closest('dialog'),fn=dialog._confirm;dialog.close();dialog.remove();return fn?.();}
 if(action==='explain-frame')return mountDialog('哪些會出現在正式程式？','<h3>僅供這份預覽</h3><p>頂端帶有「預覽工具」的整條工具列，包括情境、重設、背景核對與審查說明，不會放進正式程式。</p><h3>正式產品的設計候選</h3><p>左側主導覽、實例清單、設定分類、紀錄、dialog 與底部管理服務狀態，都是這次要確認的 2.0 介面。</p><h3>仍是模擬資料</h3><p>全部伺服器、Java 路徑與檢查結果都是示範；此頁不會存取本機檔案或執行 Java。原生後台示範視窗也不是實際 ConPTY。</p>',button('關閉','close-dialog'));
 if(action==='back-instances')return navigate('instances');if(action==='create'||action==='import')return beginWizard(action);
 if(action==='clear'){state.selected.clear();renderRows();return renderDetail();}
 if(['all-start','all-stop','all-restart'].includes(action))return triggerAll(action.slice(4));
 if(['start','stop','restart'].includes(action))return triggerOperation(action);
 if(action==='clear-stale'){state.scenario='normal';$('#scenario').value='normal';announce('已清理失效釘選。');return renderInstances();}
 if(action==='console')return terminalWindow();if(action==='logs')return logsDialog();
 if(action==='core')return coreDialog();if(action==='backups')return backupsDialog(id);
 if(action==='instance-settings')return navigate('settings',{target:[...state.selected][0]});
 if(action==='session'||action==='refresh'){if(!state.connected)return messageBox('需要處理', '無法核對：先重新連線。舊資料仍保留。');const r=rowBy([...state.selected][0]);if(action==='session'&&r){r.checked=readableTime();const time=$('#detail-checked');if(time)time.textContent=r.checked;}confirmAtButton(a,'已核對');return;}
 if(action==='pin'||action==='pin-up'||action==='pin-down'){const r=rowBy([...state.selected][0]);if(!r)return;if(action==='pin'){r.pin=r.pin===null?Math.max(-1,...state.rows.filter(x=>managedRow(x)&&x.pin!==null).map(x=>x.pin))+1:null;}else{const pins=state.rows.filter(x=>managedRow(x)&&x.pin!==null).sort((x,y)=>x.pin-y.pin);const i=pins.indexOf(r),other=pins[i+(action==='pin-up'?-1:1)];if(other)[r.pin,other.pin]=[other.pin,r.pin];}renderRows();return renderDetail();}
 if(action==='core-check'){if(a.dataset.busy)return;a.dataset.busy='1';a.textContent='檢查中';announce('正在檢查此實例的核心更新。');timer(()=>{if(!a.isConnected)return;a.textContent='檢查更新';a.dataset.busy='';if(state.scenario!=='failure')$('#core-time',a.closest('dialog')).textContent=readableTime();$('#core-status',a.closest('dialog')).textContent=state.scenario==='failure'?'檢查失敗：示範來源無回應，保留上次資料。':'檢查完成；已比對目前與最新版本。';},600);return;}
 if(action==='core-update'){const r=rowBy(id);if(r.core==='Spigot')return confirmDialog('選擇本機核心 · 示範',`<p>選取已核對的 ${r.core} ${r.version} build ${esc(r.latest)} 成品；不執行 BuildTools 或讀取檔案。</p>`,'使用示範候選',()=>{state.localCandidates??=new Set();state.localCandidates.add(id);closeDialogs();triggerOperation('update',[id]);});closeDialogs();return triggerOperation('update',[id]);}
 if(action==='restore'){state.restoreCandidate={id,build:$('#backup-select',a.closest('dialog')).value};closeDialogs();return triggerOperation('restore',[id]);}
 if(action==='resolve-core'||action==='reconcile'){const r=state.records.find(r=>r.id===id)||state.records.find(r=>r.ids.includes(id)&&/未確認/.test(r.detail));if(!r)return messageBox('需要處理', '尚無可對應的未確認操作紀錄；不宣稱核對完成。');r.result='唯讀核對完成';r.detail+='\n示範核對：未重送原始操作。';refreshTaskUI();const dialog=a.closest('dialog');const result=dialog?.querySelector('.task-results');if(result)result.textContent=r.detail;const status=dialog?.querySelector('.task-state');if(status)status.textContent=r.result;confirmAtButton(a,'已核對','已唯讀核對，未重送原操作。');return;}
 if(action==='legacy-info')return mountDialog('停止證據與舊資料核對','<p>沒有舊版運行紀錄不代表一定不能更新。需以程序、鎖、Port 與目前核心身分建立可核對的停止證據。</p><p>來源未知時不強行替換；遷移只處理管理紀錄，不更動世界。</p>',button('關閉','close-dialog'));
 if(action==='copy-diagnosis'){
  const diagnostic=$('#core-json',a.closest('dialog'));
  const manualCopy=()=>{
   if(!a.isConnected)return;
   $('#diagnostic-copy-error',a.closest('dialog')).textContent='無法自動複製。可選取上方診斷文字後複製。';
   announce('無法自動複製，請選取診斷文字後複製。');
  };
  if(!navigator.clipboard||!diagnostic){manualCopy();return;}
  navigator.clipboard.writeText(diagnostic.textContent).then(()=>{
   if(!a.isConnected)return;
   $('#diagnostic-copy-error',a.closest('dialog')).textContent='';
   confirmAtButton(a,'已複製');
  }).catch(manualCopy);
  return;
 }
 if(action==='core-readback'||action==='reload-log'){
  const missing=action==='reload-log'&&$('#log-kind',a.closest('dialog'))?.value==='missing';
  confirmAtButton(a,missing?'讀取失敗':'已讀取');return;
 }
 if(action==='log-folder'){confirmAtButton(a,'僅供預覽','離線預覽不開啟本機目錄。');return;}
 if(action==='record')return recordDialog(id);
 if(action==='all-records'){state.recordFilter='all';state.route='activity';return render();}
 if(action==='recent-core'){guardLeave(()=>{state.recordFilter='core';state.route='activity';render();});return;}
 if(action==='batch-core'||action==='selected-core'){const ids=action==='selected-core'?[...state.selected]:allTargets().filter(r=>r.kind!=='Agent'&&String(r.build)!==String(r.latest)).map(r=>r.id);return triggerOperation('update',ids);}
 if(action==='check-all'){
  if(state.coreQueryLabel==='檢查中')return;
  const context=state;
  clearTimeout(state.coreQueryReset);
  const failed=state.scenario==='failure';
  const updateLabel=(label,description)=>{
   state.coreQueryLabel=label;
   const labelNode=$('[data-core-query-label]');
   if(labelNode)labelNode.textContent=label;
   announce(description);
  };
  updateLabel('檢查中','已受理全部核心檢查。');
  timer(()=>{
   if(state!==context)return;
   updateLabel(failed?'檢查失敗':'檢查完成',failed?'核心來源未回應，保留上次資料。':'已完成全部核心檢查，各實例已比對。');
   state.coreQueryReset=timer(()=>{
    if(state!==context)return;
    state.coreQueryLabel='';
    const labelNode=$('[data-core-query-label]');
    if(labelNode)labelNode.textContent='核心維護';
   },1800);
  },600);
  return;
 }
 if(action==='cancel-remaining'){const targets=state.tasks.filter(t=>!t.done&&(!id||t.id===id));if(!targets.length){confirmAtButton(a,'無待取消工作');return;}const cancel=()=>{targets.forEach(t=>{t.cancelled=true;t.cancelPendingStarts?.();});announce(targets.some(t=>t.lifecycle)?'已取消尚未開始的啟動；已送出的正常停止仍會完成，不撤銷停機。':'已取消指定範圍內尚未開始的項目；不強行中斷執行中的步驟。');};if(id)return cancel();return confirmDialog('取消全部工作的剩餘項目？',targets.map(t=>'<p>'+esc(t.label)+'：'+t.ids.map(x=>esc(rowBy(x)?.name||x)).join('、')+'</p>').join(''),'取消全部剩餘項目',cancel);}
 if(action==='reconnect'){setConnection(true);state.scenario='normal';$('#scenario').value='normal';render();announce('管理服務已連線。');return;}
 if(action==='reset-draft'){state.draft=structuredClone(state.baseline);state.dirty=false;state.settingsError='';applyTheme();return renderSettings();}
 if(action==='reset-runtime-field'){state.draft[a.dataset.key]=state.baseline[a.dataset.key];markDirty();renderSettings();return;}
 if(action==='java-picker')return javaPicker();
 if(action==='java-use'){const d=$('#java-picker-dialog'),offset=$('.settings-scroll').scrollTop;state.draft.java=state.javaPending;markDirty();d.close();d.remove();renderSettings();$('.settings-scroll').scrollTop=offset;$('[data-action=java-picker]').focus({preventScroll:true});return;}
 if(action==='java-refresh'){if(a.dataset.busy||a.dataset.localFeedback)return;const owner=a.closest('dialog');a.dataset.busy='1';a.textContent='探索中';timer(()=>{a.textContent='重新探索';delete a.dataset.busy;if(owner?.isConnected&&owner.open)confirmAtButton(a,'已核對','已核對示範 Java 清單，沒有變更。');},600);return;}
 if(action==='java-browse')return confirmDialog('選擇 Java 執行檔 · 示範','<p>C:\\Java\\jdk8\\bin\\java.exe</p><p>不會開啟本機檔案選擇器。</p>','使用此示範路徑',()=>{choosePendingJava(javaChoices[3][1]);});
 if(action==='java-check'){const check=probeJava(state.draft.java);$('#java-check-result').textContent=check.message+(check.status==='ready'?' 版本：'+check.version+'。核心相容性仍另外核對。':'');$('#java-check-result').className=check.status==='ready'?'hint':'form-error';announce(check.message);return;}

 if(action==='placeholder'){const i=$('#field-title');i.value+=`{${a.dataset.key}}`;editField(i);return;}
 if(action==='version-preview')return mountDialog('各版本套用結果','<table><thead><tr><th>核心／版本</th><th>模擬距離</th><th>地獄維度與傳送門</th></tr></thead><tbody><tr><td>Paper 26.3</td><td>可套用</td><td>分開設定</td></tr><tr><td>Paper 26.2</td><td>可套用</td><td>分開設定</td></tr><tr><td>Folia 26.1.2</td><td>可套用</td><td>分開設定</td></tr><tr><td>Spigot 1.8.8</td><td>此版本略過</td><td>依舊版對應方式</td></tr></tbody></table><p class="hint">同核心與 Minecraft 版本合併，build 不重複列出；此處固定示範，尚無實際核心能力驗證。</p>',button('關閉','close-dialog'));
 if(action==='world-files'||action==='world-rules'){
  const tab=action==='world-files'?'files':'rules';
  if(tab===state.worldTab)return;
  return guardLeave(()=>{
   state.policyFilter='';
   state.worldTab=tab;
   state.dirty=false;
   state.draft=null;
   renderSettings();
   enhanceScrolls();
  });
 }
 if(action==='reset-field'){state.draft[a.dataset.key]=state.baseline[a.dataset.key];markDirty();return renderSettings();}
 if(action==='world-results')return mountDialog('世界政策 · 結果與診斷','<p>逐項成功、略過、失敗與結果未確認會保留於此。開啟不會重新套用。</p><pre>目前沒有本輪真實政策操作。\n備份：未建立（純示範）</pre>'+button('唯讀核對歷史','resolve-core'),button('關閉','close-dialog'));
 if(action==='scan-preview'){
 const allow=state.draft.allow,deny=state.draft.deny;
 let rows;try{rows=state.rows.map(r=>`<tr><td>${esc(r.name)}<br><code>${esc(demoRowPath(r))}</code></td><td>${!managedRow(r)?'略過：未納入管理':scanIncludes(r,allow,deny)?'顯示':'隱藏'}</td><td>${r.status==='running'?'仍繼續運作':'不啟動、不刪除'}</td></tr>`).join('');}catch(e){settingsPresentation.error=e.message;return;}
 mountDialog('掃描範圍預覽',`<p>排除優先；此處只比對示範目錄，沒有掃描磁碟。</p><table><thead><tr><th>實例</th><th>儲存後清單</th><th>程序影響</th></tr></thead><tbody>${rows}</tbody></table><p>隱藏仍在運作的實例不會停止它，也不會刪除資料。</p>`,button('返回編輯','close-dialog'));return;
 }
 if(action==='wizard-next')return wizardNext();if(action==='wizard-back'){captureWizard();state.wizard.step--;return renderWizard();}if(action==='wizard-cancel')return requestWizardClose();
 if(action==='import-existing')return importExisting(id);
 if(action==='remove-keep'||action==='remove-clear')return confirmRemoval(id,action==='remove-keep'?'keep':'clear');
 if(action==='legacy-preview')return legacyPreview(id);
 if(action==='legacy-rollback')return legacyRollback(id);
 if(action==='import-retry'){state.scenario='normal';$('#scenario').value='normal';return renderWizard();}
 if(action==='view-registered'){finishWizardClose();closeDialogs();state.selected=new Set([id]);state.query='';state.filter='all';state.route='instances';render();return;}
 if(action==='onboarding')return onboarding();
 if(action==='unmanage'){return removalDialog([...state.selected][0]);}
 if(action==='quit')return confirmDialog('結束管理服務？','目前示範中的伺服器會繼續運作；GUI 將失去控制與新的狀態核對。','結束管理服務',()=>{setConnection(false);navigate('instances');});
 if(action==='emergency-stop'){
  const row=rowBy([...state.selected][0]);
  const task=state.tasks.find(item=>item.action==='stop'&&item.scope==='selected'&&item.ids.length===1&&item.forceStop?.has(row?.id));
  if(!task)return;
  return confirmDialog('強制停止伺服器？',`<p>${esc(row.name)} 尚未存檔的世界變更可能遺失。此操作只影響示範資料。</p>`,'強制停止',()=>{
   if(!state.connected||rowBy(row.id)!==row||!task.forceStop.get(row.id)?.())return messageBox('工作階段已變更','已重新核對，沒有強制停止。');
  },true);
 }
 if(action==='stop-diagnosis')return mountDialog('停止尚未確認','<p>程序與 Port 仍可能存在。可繼續等待、查看後台；不自動強制終止或接著重啟。</p>'+button('強制終止','force-stop','class="danger"'),button('關閉','close-dialog'));
 if(action==='force-stop')return confirmDialog('強制終止？','可能遺失尚未儲存的世界資料。本線框只記錄示範決策。','確認強制終止',()=>confirmAtButton(a,'已記錄','已記錄示範選擇，未終止程序。'));
 if(action==='wait-again'){confirmAtButton(a,'等待中','示範繼續等待，尚未確認停止。');return;}
 if(action==='silent-check'){const epoch=state;const contextId=[...state.selected][0];timer(()=>{if(state!==epoch)return;const r=rowBy(contextId||'drop');if(!r)return;r.checked=readableTime();if(state.route==='instances'&&state.selected.has(r.id)&&$('#detail-checked')){const time=$('#detail-checked');if(time.textContent!==r.checked)time.textContent=r.checked;}const dialog=$('dialog[data-core-id]');if(dialog&&dialog.dataset.coreId===r.id)$('#core-time',dialog).textContent=r.checked;},600);return;}
 if(action==='review-screen')return inventoryScreen(a.dataset.tab);if(action==='inventory-screen')return inventoryScreen(a.dataset.screen);
 if(action==='failure-example'){state.scenario='failure';$('#scenario').value='failure';state.selected=new Set(['drop']);state.route='instances';render();$('[data-primary-power]')?.focus();announce('可按啟動查看失敗示例。');}
});
document.addEventListener('input',event=>{const i=event.target;if(i.closest('#wizard-dialog'))state.wizardDirty=true;if(i.id==='search'){state.query=i.value;renderRows();renderDetail();}else if(i.dataset.field)editField(i);else if(i.id==='world-ticks'||i.id==='initial-ticks'){state.draft._timeTicks=Number(i.value);markDirty();}else if(i.id==='policy-search'){const pos=i.selectionStart;state.policyFilter=i.value;$('#settings-content').innerHTML=policyContent();$('#policy-search').focus();$('#policy-search').setSelectionRange(pos,pos);enhanceScrolls();}});
document.addEventListener('change',event=>{const i=event.target;if(i.closest('#wizard-dialog'))state.wizardDirty=true;if(i.dataset.select)return selectRow(i.dataset.select,event,true);if(i.dataset.field&&(i.type==='checkbox'||i.tagName==='SELECT'))editField(i);if(i.id==='status-filter'){state.filter=$('#status-filter').value;renderRows();renderDetail();}if(i.id==='world-time'){state.draft._timeEnabled=i.checked;$('#world-ticks').disabled=!i.checked;markDirty();}if(i.id==='init-time'){state.draft._timeEnabled=i.checked;$('#initial-ticks').disabled=!i.checked;markDirty();}if(i.id==='policy-advanced'){state.advancedPolicy=i.checked;renderSettings();enhanceScrolls();}if(i.id==='log-kind')$('#log-content').textContent=i.value==='missing'?'無法讀取示範紀錄。可能尚未建立或無權限，請核對路徑後重試。':i.value==='stderr'?'此示範來源為空。空紀錄不代表啟動成功。':`[示範 ${i.value}] 此內容不是本機紀錄。`;});
document.addEventListener('submit',event=>{if(event.target.id==='settings-form'){event.preventDefault();saveSettings();}});
document.addEventListener('wheel',event=>{if(event.target.matches('input[type=number]')){event.preventDefault();const scroller=event.target.closest('.overlay-scroll,.dialog-body');if(scroller)scroller.scrollBy(0,event.deltaY);else window.scrollBy(0,event.deltaY);}}, {passive:false});
document.addEventListener('keydown',event=>{const menu=$('.action-menu[open]');if(event.key==='Escape'&&menu&&!$('dialog[open]')){event.preventDefault();menu.open=false;$('summary',menu).focus();return;}if(['ArrowUp','ArrowDown'].includes(event.key)&&event.target.matches('[data-rowselect]')){event.preventDefault();const rows=visibleRows(),current=rows.findIndex(r=>r.id===event.target.dataset.rowselect),next=rows[current+(event.key==='ArrowDown'?1:-1)];if(next){if(event.ctrlKey&&!event.shiftKey)$(`[data-rowselect="${next.id}"]`).focus();else selectRow(next.id,event);}return;}if([' ','Enter'].includes(event.key)&&event.target.matches('[data-rowselect]')){event.preventDefault();selectRow(event.target.dataset.rowselect,event,event.ctrlKey||event.metaKey);return;}if(event.key==='Escape'&&!$('dialog')&&state.route==='instances'&&!event.target.matches('input,textarea,select')){state.selected.clear();renderRows();renderDetail();announce('已清除選取');}if(event.key==='a'&&(event.ctrlKey||event.metaKey)&&state.route==='instances'&&!event.target.matches('input,textarea,select')){event.preventDefault();state.selected=new Set(visibleRows().map(r=>r.id));renderRows();renderDetail();}});
$('#scenario').addEventListener('change',()=>{state.scenario=$('#scenario').value;if(state.scenario==='legacy-ready')state.legacyCandidates.forEach(r=>{r.status='stopped';r.hostIdle=true;});if(state.scenario==='disconnect')setConnection(false);if(['java-stale','java-unrunnable'].includes(state.scenario)){const record=javaRecords.find(r=>state.scenario==='java-stale'?!r.exists:r.exists&&!r.runnable);const id=state.target||[...state.selected][0]||'drop';if(state.route==='settings'&&!state.target)state.globals.java=record.path;else state.instanceSettings[id].java=record.path;if(state.route==='settings'&&settingsPage()==='runtime'){state.draft.java=record.path;state.baseline.java=record.path;markDirty();renderSettings();}}if(state.scenario==='mise-duplicate'&&state.route==='settings'&&settingsPage()==='runtime')renderSettings();if(state.route==='instances')renderInstances();});
$('#reset-demo').addEventListener('click',()=>{guardLeave(()=>{pendingTimers.forEach(clearTimeout);pendingTimers=[];closeDialogs();for(const p of popups.values())if(!p.closed)p.close();popups.clear();resetState();$('#scenario').value='normal';setConnection(true);applyTheme();render();announce('已重設示範資料。');});});
window.addEventListener('beforeunload',event=>{if(state.dirty||state.wizardDirty){event.preventDefault();event.returnValue='';}});
setConnection(state.connected);render();
}
// Overlay scrollbars preserve the viewport width; keyboard/wheel use native scroll offsets.
function enhanceScrolls(){const areas=$$('#view,.rail,.button-list,.dialog-body,.settings-scroll,.workspace .table-wrap,.detail-panel');areas.forEach(area=>{if(area.dataset.overlay)return;area.dataset.overlay='1';area.classList.add('overlay-scroll');const shell=document.createElement('div');shell.className='scroll-shell';area.parentNode.insertBefore(shell,area);shell.append(area);const track=document.createElement('div');track.className='overlay-track';track.setAttribute('aria-hidden','true');const thumb=document.createElement('div');thumb.className='overlay-thumb';track.append(thumb);shell.append(track);function update(){const needed=area.scrollHeight>area.clientHeight+1;track.hidden=!needed;if(!needed)return;const available=track.clientHeight;const size=Math.max(28,available*area.clientHeight/area.scrollHeight);thumb.style.height=size+'px';thumb.style.top=(available-size)*area.scrollTop/(area.scrollHeight-area.clientHeight)+'px';}area.addEventListener('scroll',update,{passive:true});
 area.addEventListener('toggle',update,true);
 // A fixed viewport can keep its size while its contents change.
 const contentObserver=new MutationObserver(update);
 contentObserver.observe(area,{childList:true,subtree:true,characterData:true});
 const observer=new ResizeObserver(update);observer.observe(area);let y=0,offset=0;thumb.addEventListener('pointerdown',e=>{y=e.clientY;offset=area.scrollTop;thumb.setPointerCapture(e.pointerId);track.classList.add('dragging');e.preventDefault();});thumb.addEventListener('pointermove',e=>{if(!thumb.hasPointerCapture(e.pointerId))return;const range=track.clientHeight-thumb.clientHeight;area.scrollTop=offset+(e.clientY-y)*(area.scrollHeight-area.clientHeight)/Math.max(1,range);});thumb.addEventListener('pointerup',()=>track.classList.remove('dragging'));requestAnimationFrame(update);});}


// Offline fixtures only. No filesystem, process, or service access.
function legacyFixtures(){return [
 {id:'legacy-idle',name:'舊版測服 · 已停止',path:'D:\\DemoServers\\Legacy\\idle',port:12901,core:'Paper',version:'26.3',status:'stopped',hostIdle:true,settings:{...defaults,xmx:'5G'},connected:false},
 {id:'legacy-running',name:'舊版測服 · 運作中',path:'D:\\DemoServers\\Legacy\\running',port:12902,core:'Paper',version:'26.2',status:'running',hostIdle:false,settings:{...defaults,xmx:'4G'},connected:false}
 ];}
function managedRow(r){return !!r&&r.kind!=='Agent'&&!r.external&&!r.removed&&r.managed!==false&&r.hasManagementData!==false;}
function managementPath(path){return resolveDemoPath(path).replace(/\//g,'\\').replace(/\\+$/,'').toLowerCase();}
function wizardError(text){if($('#import-problem')&&text===importProblem()){$('#wizard-error').textContent='';$('#import-problem').tabIndex=-1;$('#import-problem').focus();return;}const e=$('#wizard-error');e.textContent=text;e.tabIndex=-1;e.focus();}
function importDuplicate(w){delete w.existingId;w.restoreExisting=false;let key;try{key=managementPath(w.path);}catch(error){wizardError(error.message);return true;}
 const row=state.rows.find(r=>managementPath(r.path)===key);if(!row)return false;
 if((row.hasManagementData===false||row.removed)&&!row.external){w.existingId=row.id;w.restoreExisting=row.hasManagementData!==false&&!!row.removed;Object.assign(w,{name:row.name,port:row.port,core:row.core,version:row.version,kind:row.kind});for(const [field,value]of Object.entries({name:w.name,port:w.port,core:w.core,version:w.version,purpose:w.kind}))if($('#wizard-'+field))$('#wizard-'+field).value=value;return false;}
 const e=$('#wizard-error');e.textContent='此位置已登記，請查看既有實例，不重複建立管理資料。';
 if(managedRow(row))e.insertAdjacentHTML('beforeend',button('查看已登記實例','view-registered',`data-id="${esc(row.id)}"`));e.tabIndex=-1;e.focus();return true;
}
function importProblem(){return {'import-permission':'無法讀取此位置。請確認讀取權限後重新辨識；已填內容保留。','import-unrecognized':'無法確認核心。請核對資料夾與核心檔案後重新辨識，不猜測版本。','import-external':'外部程序仍在使用這個位置。請在原管理工具正常停止後再辨識；本頁不接管或停止它。'}[state.scenario]||'';}
function importReadback(w){const problem=importProblem();return `<h3>唯讀辨識結果</h3><p class="hint">固定離線示例，沒有讀取磁碟或程序。原位置不搬動；不套初始化政策、不下載或啟動。登記後請核對 Java 與 JVM，不修改既有啟動檔。</p><dl class="core-facts"><dt>位置</dt><dd class="mono">${esc(w.path)}</dd><dt>核心</dt><dd>${problem?'尚未完成核對':esc(w.core+' '+w.version)}</dd><dt>檔案影響</dt><dd>世界、玩家、插件與伺服器設定全部保留</dd></dl>${w.restoreExisting?'<p class="inset">找到既有 InstanceHarbor 管理設定（removed: true）。確認匯入後讀回原設定，只將 removed 改為 false；不建立另一份設定。</p>':'<p class="hint">此位置沒有可恢復的管理設定，確認後建立新的 InstanceHarbor 管理設定。</p>'}${problem?`<p id="import-problem" class="form-error" role="alert">${esc(problem)}</p>${button('重新辨識（模擬問題已排除）','import-retry')}<p class="hint">預覽工具：重試以成功資料示範，正式版仍須重新核對實際來源。</p>`:'<label><input id="import-ack" type="checkbox">我已確認管理範圍，不接管外部運作中的程序</label>'}`;}
function completeImport(w){if(!state.connected){wizardError('管理服務未連線，尚未建立登記；草稿保留。');return;}if(state.rows.some(r=>managedRow(r)&&r.id!==w.existingId&&(managementPath(r.path)===managementPath(w.path)||r.port===w.port))){wizardError('此位置或 Port 已被登記，請返回重新辨識。');return;}if(importProblem()){wizardError(importProblem());return;}let r=w.existingId?rowBy(w.existingId):null;
 if(r&&(r.external||r.status!=='stopped'||managedRow(r)||state.tasks.some(t=>!t.done&&t.ids.includes(r.id)))){wizardError('此位置的管理狀態已改變，請返回重新辨識。');return;}
 if(!r){r={id:'import-'+Date.now(),name:w.name,project:'匯入實例',kind:w.kind,core:w.core,version:w.version,port:w.port,path:w.path,status:'stopped',build:18,latest:18,pin:null,serverFiles:{world:'保留',players:'保留',plugins:'保留',configuration:'保留'}};state.rows.push(r);}
 const restoring=!!(r.removed&&r.hasManagementData!==false&&w.restoreExisting);if(w.restoreExisting&&!restoring){wizardError('原管理設定狀態已改變，請返回重新辨識。');return;}r.hasManagementData=true;r.removed=false;r.managed=true;r.external=false;if(!restoring){r.pin=null;state.instanceSettings[r.id]={...state.globals};}
 const record=newRecord(restoring?'匯入恢復管理':'匯入管理設定',[r.id],'已完成');record.detail=(restoring?'已讀回保留設定並將 removed 改為 false；':'只建立示範管理設定；')+'原位置、世界、玩家、插件與伺服器設定未變。沒有啟動或套用政策。';
 finishWizardClose();state.selected=new Set([r.id]);state.query='';state.filter='all';state.route='instances';render();announce(restoring?'已透過匯入恢復原管理設定，保持停止；伺服器檔案保留。':'已透過匯入建立新的管理設定，保持停止；伺服器檔案保留。');}
function removalDialog(id){const r=rowBy(id);if(!managedRow(r))return messageBox('需要處理','此實例不在管理範圍。');
 const blocked=!state.connected||r.status!=='stopped'||state.tasks.some(t=>!t.done&&t.ids.includes(id));const d=mountDialog('移除管理 · '+esc(r.name),`<p class="mono">${esc(r.path)}</p><p>兩種方式都保留世界、玩家、插件與伺服器設定，不刪除伺服器資料夾。</p>${blocked?'<p class="form-error">請先正常停止並等待工作完成，再移除管理。本視窗不自動停止伺服器。</p>':''}<section class="management-choice"><h3>移除管理，保留管理設定</h3><p>在 InstanceHarbor 管理 JSON 設定 removed: true。一般清單與全部操作略過；之後透過「匯入已有伺服器」讀回設定才恢復。</p>${button('移除管理，保留管理設定','remove-keep',`data-id="${esc(id)}" ${blocked?'disabled':''}`)}</section><section class="management-choice"><h3>清除管理資料</h3><p>清除該實例的 InstanceHarbor 管理 JSON 與專用偏好。之後只能重新匯入，無法自動還原這些設定。</p>${button('清除管理資料','remove-clear',`data-id="${esc(id)}" class="danger" ${blocked?'disabled':''}`)}</section>`,button('關閉','close-dialog'));d.id='remove-dialog';}
function confirmRemoval(id,mode){const r=rowBy(id);if(!state.connected||!managedRow(r)||r.status!=='stopped'||state.tasks.some(t=>!t.done&&t.ids.includes(id)))return messageBox('無法移除','實例仍在運作或有工作進行中，請先正常停止並核對。');
 const label=mode==='keep'?'移除管理，保留管理設定':'清除管理資料';confirmDialog(label+'？',`<p>${esc(r.name)}</p><p>${mode==='keep'?'保留管理 JSON，設定 removed: true；重新加入會沿用原設定。':'只清除 InstanceHarbor 專用管理資料，重新匯入會建立新設定。'}</p><p>世界、玩家、插件與伺服器設定全部保留。</p>`,label,()=>{
  if(!state.connected||!managedRow(r)||r.status!=='stopped'||state.tasks.some(t=>!t.done&&t.ids.includes(id)))return messageBox('狀態已改變','沒有移除管理資料。請重新核對。');
  if(mode==='keep'){r.removed=true;r.hasManagementData=true;}else{r.removed=false;r.hasManagementData=false;r.managed=false;r.pin=null;delete state.instanceSettings[id];const legacy=state.legacyCandidates.find(x=>x.id===id);if(legacy)legacy.connected=false;}
  const record=newRecord(label,[id],'已完成');record.detail='只有示範管理資料改變；世界、玩家、插件、伺服器設定與原位置保留。';closeDialogs();state.selected.delete(id);state.route='instances';render();announce(mode==='keep'?'已標記移除管理。請透過「匯入已有伺服器」恢復原設定。':'已清除示範管理資料。伺服器檔案保留，請透過「匯入已有伺服器」建立新管理設定。');
 });}
function importExisting(id){const r=rowBy(id);if(!r)return;closeDialogs();beginWizard('import');Object.assign(state.wizard,{name:r.name,path:r.path,port:r.port,core:r.core,version:r.version,existingId:id,restoreExisting:!!r.removed&&r.hasManagementData!==false});renderWizard();}
function legacyOverview(){const d=mountDialog('辨識 1.0 資料 · 唯讀預覽',`<p>先辨識再確認接入。不停服、不搬世界、不改原 Host 或 policy。</p><div class="table-wrap"><table><thead><tr><th>候選</th><th>目前控制權</th><th>操作</th></tr></thead><tbody>${state.legacyCandidates.map(r=>`<tr><td>${esc(r.name)}<small class="policy-source">${esc(r.path)}</small></td><td>${managedRow(rowBy(r.id))?'已建立 2.0 示範登記':rowBy(r.id)?.removed?'已移除管理 · 設定保留':r.hostIdle&&r.status==='stopped'?'已停止 · 原 Host 閒置（示範）':'1.0 仍持有運作中程序'}</td><td>${managedRow(rowBy(r.id))?(r.connected?button('還原本次接入','legacy-rollback',`data-id="${r.id}"`):button('查看已登記實例','view-registered',`data-id="${r.id}"`)):rowBy(r.id)?.removed?button('透過匯入恢復','import-existing',`data-id="${r.id}"`):button(r.hostIdle?'預覽接入':'查看交接條件','legacy-preview',`data-id="${r.id}"`)}</td></tr>`).join('')}</tbody></table></div><p class="hint">這是固定離線資料，不是實際 Host 狀態。相同實例不由兩個版本同時控制。</p>`,button('關閉','close-dialog'));d.id='legacy-dialog';}
function legacyPreview(id){const r=state.legacyCandidates.find(x=>x.id===id);if(!r)return;if(managedRow(rowBy(id)))return messageBox('已納入管理','請查看既有實例，不重複接入。');if(!state.connected||!r.hostIdle||r.status!=='stopped')return messageBox('尚不能接入','原 1.0 仍持有運作中實例。請保留原控制方式；正常停止、確認 Host 閒置與控制權釋放後，再重新辨識。本頁不會替妳停止或接管。');
 const d=mountDialog('預覽舊版接入 · '+esc(r.name),`<p>只轉入可對應的管理設定，原 Host、policy 與全部伺服器檔案保留。</p><table><thead><tr><th>管理設定</th><th>原值</th><th>接入後</th></tr></thead><tbody><tr><td>Java</td><td class="mono">${esc(r.settings.java)}</td><td>一次複製，獨立儲存</td></tr><tr><td>最大記憶體</td><td>${esc(r.settings.xmx)}</td><td>${esc(r.settings.xmx)}</td></tr><tr><td>舊版專用欄位</td><td>原資料保留</td><td>不支援的欄位不猜值、不覆寫</td></tr></tbody></table><details><summary>檢視完整管理設定快照（示範）</summary><pre>${esc(JSON.stringify(r.settings,null,2))}</pre></details><p>先保存接入前的管理資料快照。接入完成保持停止；可在原流程還原本次接入。</p><p class="hint">真實 Rust 接入、控制權證據與回復尚待隔離驗證。</p>`,button('取消','close-dialog')+button('確認接入','confirm-legacy','class="primary"'));d.id='legacy-preview-dialog';d.querySelector('[data-action=confirm-legacy]').addEventListener('click',()=>{
  if(!state.connected||r.connected||!r.hostIdle||r.status!=='stopped')return messageBox('無法接入','原控制權狀態已改變。請重新辨識。');
  state.legacyBackups[id]=structuredClone(r);let row=rowBy(id);if(!row){row={...structuredClone(r),project:'舊版接入',kind:'Human',build:18,latest:18,pin:null};state.rows.push(row);}Object.assign(row,{removed:false,managed:true,hasManagementData:true});state.instanceSettings[id]={...r.settings};r.connected=true;newRecord('舊版接入',[id],'示範已接入').detail='原管理資料快照保留；沒有改動原 Host、policy 或伺服器檔案。';closeDialogs();state.route='instances';state.selected=new Set([id]);state.query='';state.filter='all';render();announce('示範登記已接入，保持停止；可從舊版接入入口還原。');
 });}
function legacyRollback(id){const r=state.legacyCandidates.find(x=>x.id===id),row=rowBy(id);if(!r?.connected||!state.legacyBackups[id])return messageBox('無法還原','找不到本次接入快照。');if(!state.connected||row.status!=='stopped'||state.tasks.some(t=>!t.done&&t.ids.includes(id)))return messageBox('請先正常停止','本次接入的實例仍在運作或有工作進行中，不會自動停止。');confirmDialog('還原本次接入？','<p>只撤回本次 2.0 管理登記。原 1.0 資料、Host、policy 與伺服器檔案不變，不自動啟動。</p>','還原本次接入',()=>{if(!state.connected||row.status!=='stopped'||state.tasks.some(t=>!t.done&&t.ids.includes(id)))return messageBox('無法還原','狀態已改變，請重新核對。');row.managed=false;row.hasManagementData=false;delete state.instanceSettings[id];r.connected=false;newRecord('還原舊版接入',[id],'已完成').detail='原管理資料與伺服器檔案保留。';closeDialogs();state.selected.clear();render();announce('本次示範接入已還原，原管理資料與伺服器檔案保留。');});}

const {runLifecycle,lifecycleDiagram,lifecycleDiagramBody}=createLifecycle({
 getState:()=>state,
 resetScenario:()=>{state.scenario='normal';$('#scenario').value='normal';},
 newRecord,rowBy,managedRow,probeJava,timer,refreshTaskUI,readableTime,announce,esc,
});

// Browser checks use the same interaction assertions as the preserved comparison page.
if(import.meta.env.DEV){
 Object.assign(window,{titleCatalog,demoBase,seed,marks,defaults,javaRecords,dedupeJava,javaChoices,popups,rowBy,badge,button,setConnection,applyTheme,navigate,resolveDemoPath,scanIncludes,visibleRows,renderInstances,renderRows,renderDetail,selectRow,closeDialogs,triggerOperation,allTargets,triggerAll,runOperation,recordDialog,coreDialog,backupsDialog,terminalWindow,settingsPage,javaPicker,wizardNext,beginWizard,onboarding,enhanceScrolls,managedRow,removalDialog});
 Object.defineProperty(window,'state',{get:()=>state});
}
