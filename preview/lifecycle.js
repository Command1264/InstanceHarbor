// Keeps the preview scheduler independent of the page and modal components.
export function createLifecycle({getState, resetScenario, newRecord, rowBy, managedRow, probeJava, timer, refreshTaskUI, readableTime, announce, esc}) {
// Timed memory-only state model. Durations and start concurrency are demo values.
function wakeSharedStarts(){for(const pump of [...getState().startPumps])pump();}
function runLifecycle(action,ids,label,scope){
 const state=getState();
 const outcome=['failure','timeout','unknown'].includes(state.scenario)?state.scenario:'normal';if(outcome!=='normal'){resetScenario();}
 const record=newRecord(label,ids);record.scope=scope;
 const task={id:record.id,ids:[...ids],action,label,scope,lifecycle:true,done:false,progress:0,record,results:[],stages:{},stopSent:[],startSent:[],maxStarting:0,forceStop:new Map()};state.tasks.push(task);
 const queue=[];let activeStarts=0,booting=false;
 function finish(r,result){if(task.stages[r.id]==='完成')return;task.forceStop.delete(r.id);task.stages[r.id]='完成';task.results.push({id:r.id,name:r.name,result});r.last=`${label}：${result}`;task.progress=Math.round(task.results.length/ids.length*100);record.detail=task.results.map(x=>`${x.name}：${x.result}`).join('\n');
  if(task.results.length===ids.length){task.done=true;state.startPumps.delete(pumpStarts);record.result=task.results.some(x=>/失敗|逾時|未確認|身分|衝突/.test(x.result))?'需要處理':task.results.some(x=>x.result.includes('取消'))?'部分取消':'已完成';announce(label+record.result);}refreshTaskUI();}
 function conflict(r){return state.tasks.some(t=>t!==task&&!t.done&&t.ids.includes(r.id));}
 function enqueue(r){if(task.cancelled){finish(r,'已停止；已取消尚未開始的啟動');return;}task.stages[r.id]='等待啟動名額';queue.push(r);if(!booting)pumpStarts();}
 function pumpStarts(){while(state.activeStartCount<2&&queue.length){const r=queue.shift();if(task.cancelled){finish(r,'已取消尚未開始的啟動');continue;}if(!state.connected){finish(r,'失敗：管理服務未連線，沒有啟動');continue;}if(!managedRow(r)){finish(r,'失敗：不在管理範圍，沒有啟動');continue;}if(conflict(r)){finish(r,'略過：有衝突工作');continue;}
   const java=probeJava(state.instanceSettings[r.id]?.java);if(java.status!=='ready'){finish(r,'失敗：'+java.message);continue;}
   activeStarts++;state.activeStartCount++;task.maxStarting=Math.max(task.maxStarting,activeStarts);task.startSent.push(r.id);task.stages[r.id]='啟動中';r.status='starting';r.last=label+'：啟動中';refreshTaskUI();
   timer(()=>{if(outcome==='failure'){r.status='failed';activeStarts--;state.activeStartCount--;finish(r,'失敗：示範啟動未通過核對');wakeSharedStarts();return;}if(outcome==='unknown'){r.status='unknown';activeStarts--;state.activeStartCount--;finish(r,'結果未確認，請唯讀核對');wakeSharedStarts();return;}
    timer(()=>{r.status='running';r.checked=readableTime();activeStarts--;state.activeStartCount--;finish(r,'成功');wakeSharedStarts();},450);
   },500);
  }}
 // Dispatch every eligible stop before waiting for any one server to finish.
 booting=true;state.startPumps.add(pumpStarts);
 for(const id of ids){const r=rowBy(id);if(!r)continue;task.stages[id]='核對';
  if(!managedRow(r)){finish(r,'略過：不在管理範圍');continue;}
  if(conflict(r)||['starting','stopping'].includes(r.status)){finish(r,'略過：正在啟停或有衝突工作');continue;}
  if(r.status==='unknown'){finish(r,'略過：尚未核對身分');continue;}
  if(action==='start'&&r.status==='running'||action==='stop'&&r.status==='stopped'){finish(r,'已略過');continue;}
  if(action==='start'||action==='restart'&&r.status==='stopped'){enqueue(r);continue;}
  const before=r.status;task.stopSent.push(id);task.stages[id]='已送停止，等待存檔與退出';r.status='stopping';r.last=label+'：等待正常停止';
  const force=()=>{
   if(task.stages[id]==='完成'||r.status!=='stopping')return false;
   task.forceStop.delete(id);
   r.status='stopped';r.checked=readableTime();
   const receipt=newRecord('強制停止',[id],'已完成');receipt.detail='示範程序已停止，沒有啟動新工作階段。';
   finish(r,'已由緊急停止接手');return true;
  };
  task.forceStop.set(id,force);
  timer(()=>{if(task.stages[id]==='完成')return;if(outcome==='timeout'){r.status=before;finish(r,'停止逾時，尚未確認停止');return;}if(outcome==='unknown'){r.status='unknown';finish(r,'結果未確認，請唯讀核對');return;}if(outcome==='failure'){r.status=before;finish(r,'失敗：示範停止核對未通過');return;}
   timer(()=>{if(task.stages[id]==='完成')return;r.status='stopped';r.checked=readableTime();if(action==='restart'){enqueue(r);refreshTaskUI();}else finish(r,'成功');},450);
  },500);
 }
 task.cancelPendingStarts=()=>{while(queue.length)finish(queue.shift(),'已取消尚未開始的啟動');};
 booting=false;pumpStarts();refreshTaskUI();announce(`${label}已受理。停止分別送出並獨立等候；只有啟動排隊。進度與結果保留在操作紀錄。`);return task;
}
function lifecycleDiagram(task){return `<section class="schedule-demo" aria-label="停止與啟動排程示意">${lifecycleDiagramBody(task)}</section>`;}
function lifecycleDiagramBody(task){return `<h3>停止並行、啟動排隊</h3><div class="schedule-lanes"><div><strong>停止要求：獨立送出</strong><p>已送出 ${task.stopSent.length} 台，不等前台完成才通知下一台。</p></div><div><strong>各自等待存檔與退出</strong><p>停止逾時不強制終止；退出與資源釋放確認後才能重開。</p></div><div><strong>啟動：有限併發佇列</strong><p>所有工作共用最多 2 台同時啟動，尚非正式資源上限。</p></div></div><ul>${task.ids.map(id=>`<li>${esc(rowBy(id)?.name||id)}：${esc(task.stages[id]||'尚未開始')}</li>`).join('')}</ul><p class="hint">取消剩餘只阻止尚未開始的啟動，不能撤銷已送出的正常停止。所有時間與狀態都是離線模擬。</p>`;}

 return {runLifecycle,lifecycleDiagram,lifecycleDiagramBody};
}
