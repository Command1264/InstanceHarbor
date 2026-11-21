interface Receipt {
  id: string
  action: string
  ids: string[]
  targets?: string[]
  time: string
  result: string
  detail: string
}

const key = 'instanceharbor-vue-preview-activity-v1'

function isReceipt(item: unknown): item is Receipt {
  if (!item || typeof item !== 'object') return false
  const record = item as Partial<Receipt>
  return [record.id, record.action, record.time, record.result, record.detail].every(value => typeof value === 'string')
    && /^demo-\d+$/.test(record.id ?? '')
    && Array.isArray(record.ids) && record.ids.every(value => typeof value === 'string')
    && (record.targets === undefined || Array.isArray(record.targets) && record.targets.every(value => typeof value === 'string'))
}

export function readActivity(): Receipt[] {
  try {
    const data: unknown = JSON.parse(sessionStorage.getItem(key) ?? '[]')
    return Array.isArray(data) ? data.filter(isReceipt) : []
  } catch (error) {
    console.warn('無法讀取示範操作紀錄', error)
    return []
  }
}

export function saveActivity(records: Receipt[], tasks: { id: string; done: boolean }[]) {
  const pending = new Set(tasks.filter(task => !task.done).map(task => task.id))
  const snapshot = records.map(record => pending.has(record.id) || record.result === '進行中'
    ? { ...record, result: '結果未確認', detail: '重開前的示範工作未確認。請使用唯讀核對，系統不會重新送出命令。' }
    : record)
  try {
    sessionStorage.setItem(key, JSON.stringify(snapshot))
  } catch (error) {
    console.warn('無法保留示範操作紀錄', error)
  }
}

export function exportActivity(records: Receipt[]) {
  const rows = records.map(({ id, action, targets, ids, time, result }) => ({ id, action, targets: targets ?? ids, time, result }))
  const blob = new Blob([JSON.stringify({ schemaVersion: 1, records: rows }, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = 'instanceharbor-demo-activity.json'
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
