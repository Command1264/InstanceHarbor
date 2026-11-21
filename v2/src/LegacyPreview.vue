<script setup lang="ts">
import { ref } from 'vue'
import { manage } from './manager'
import OverlayScroll from './components/OverlayScroll.vue'

type Observation = { state: 'stopped' | 'managed' | 'external' | 'unknown'; reason: string; pid?: number; hostPid?: number; runId?: string; process?: string }
interface LegacyRow {
  name: string; project: string; kind: string; path: string; port: number | null
  core: { state: string; identity?: { version: string }; source: string }
  java: { source: string; alias?: string; path?: string; fileVerified?: boolean }
  observation: Observation
}
interface Preview { source: string; policy: string; skippedLinks: number; rows: LegacyRow[] }
const dialog = ref<HTMLDialogElement>()
const source = ref(''), policy = ref(''), host = ref('')
const preview = ref<Preview>(), busy = ref(false), error = ref(''), status = ref('')
let request = 0
let opener: HTMLElement | null = null
const states = { stopped: '已停止', managed: '1.0 Host 正在管理', external: '外部程序占用', unknown: '狀態未確認' }
const reasons: Record<string, string> = {
  ConfigUnreadable: '缺少必要設定，或設定無法讀取',
  ConfigInvalid: '實例身分、連接埠或設定不符合已支援的 1.0 格式',
  NoHostOrListener: '核對時沒有觀察到 Host 或 TCP 監聽',
  IndependentListener: '監聽程序未由此 1.0 實例管理',
  VerifiedLegacyHost: '已核對 Host、啟動紀錄與子程序身分',
  HostUnconfirmed: '無法確認 Host 身分或啟動紀錄。若實例仍在執行，請核對 Host 執行檔路徑',
  LegacyTicketUnreadable: '無法讀取啟動紀錄',
  LegacyDirectoryUnreadable: '無法讀取啟動紀錄資料夾',
  DuplicatePort: '掃描範圍內有多台實例設定相同的連接埠',
  ListenerUnconfirmed: '無法核對 Windows 的 TCP 監聽資料',
  ListenerMismatch: '監聽程序與 Host 回報不一致',
  ListenerAmbiguous: '監聽程序有多個或身分不明',
  LockUnconfirmed: '無法核對舊 Host 的實例鎖',
  ObservationChanged: '核對期間狀態有變化，請重新核對',
  PreviewDeadline: '本輪無法核對此實例，請重新核對',
  PipeNameUnsupported: '此路徑的字元超出本版能核對的 Host 名稱範圍',
}
const errors: Record<string, string> = {
  LegacyLocalPathRequired: '請填寫本機固定磁碟上的完整路徑。這版不支援網路路徑或路徑轉址。',
  LegacyInstancesUnreadable: '找不到可讀取的 instances 資料夾。請選擇包含 instances 的 1.0 資料根目錄。',
  LegacyPolicyUnreadable: '無法讀取掃描 policy。請確認 managed-scan.json 的位置、目錄與讀取權限。',
  LegacyPolicyUnsupported: '掃描 policy 格式不受支援。請使用 schemaVersion 1 的 managed-scan.json。',
  LegacyPolicyOutsideSource: '掃描 policy 指向來源外部。若使用隔離副本，請提供與該副本相符的 policy。',
  LegacyHostExecutableRequired: '無法核對 Host 執行檔。請確認填的是 1.0 Host 的本機 exe 路徑，或留空。',
  LegacyScanLimit: '掃描範圍超過本版上限。請指定範圍較小的掃描 policy 再核對。',
  LegacyDirectoryUnreadable: '掃描範圍內有目錄無法讀取。請核對 policy 和讀取權限。',
}
const javaSources: Record<string, string> = { instance:'實例設定', global:'全域設定', pin:'舊版固定值', registry:'舊版 Java 登記表', unknown:'尚未核對' }
function open(trigger?: HTMLElement) {
  opener = trigger ?? null
  dialog.value?.showModal()
}
function close() { request++; busy.value = false; status.value = ''; dialog.value?.close() }
function restoreFocus() { opener?.focus({ preventScroll:true }); opener = null }
defineExpose({ open })
async function check() {
  if (busy.value) return
  const id = ++request
  busy.value = true
  error.value = ''
  status.value = '正在核對所選來源'
  try {
    const next = await manage<Preview>(['legacy-preview', source.value.trim(), policy.value.trim(), host.value.trim()])
    if (request !== id) return
    preview.value = next
    status.value = next.rows.length ? `已核對 ${next.rows.length} 台實例` : '掃描範圍內沒有實例'
  } catch (e) {
    if (request !== id) return
    status.value = ''
    error.value = (errors[String(e)] ?? '無法完成唯讀預覽。請確認來源後重新核對。') +
      (preview.value ? ' 下方保留上次核對結果。' : '')
  } finally {
    if (request === id) busy.value = false
  }
}
</script>

<template>
  <dialog ref="dialog" class="legacy-preview" aria-labelledby="legacy-preview-title" @cancel.prevent="close" @close="restoreFocus">
    <h2 id="legacy-preview-title">辨識 1.0 資料</h2>
    <p class="hint">只讀取舊版資料並核對目前狀態。預覽不會遷移實例或接管程序。</p>
    <form id="legacy-preview-form" @submit.prevent="check">
      <label>1.0 資料根目錄
        <input v-model="source" name="legacy-data-root" autofocus required autocomplete="off" spellcheck="false" :readonly="busy">
      </label>
      <details>
        <summary>指定掃描 policy 或 Host</summary>
        <label>掃描 policy 路徑
          <input v-model="policy" name="legacy-scan-policy" autocomplete="off" spellcheck="false" :readonly="busy" aria-describedby="legacy-policy-hint">
        </label>
        <p id="legacy-policy-hint" class="hint">留空時讀取來源目錄下的 config/managed-scan.json。</p>
        <label>1.0 Host 執行檔路徑
          <input v-model="host" name="legacy-host-executable" autocomplete="off" spellcheck="false" :readonly="busy" aria-describedby="legacy-host-hint">
        </label>
        <p id="legacy-host-hint" class="hint">只填入你確認的 1.0 Host。留空時不向任何 Host 查詢狀態。</p>
      </details>
    </form>
    <p class="legacy-status" role="status">{{ status }}</p>
    <p v-if="error" role="alert" class="form-error">{{ error }}</p>
    <OverlayScroll v-if="preview" class="legacy-results" label="舊版實例辨識結果">
      <p class="hint selectable">已核對來源：{{ preview.source }}<br>掃描 policy：{{ preview.policy }}</p>
      <p v-if="preview.skippedLinks" class="hint">已略過 {{ preview.skippedLinks }} 個連結目錄。</p>
      <article v-for="row in preview.rows" :key="row.path" class="legacy-row">
        <div class="legacy-row-title"><strong>{{ row.name }}</strong><span>{{ states[row.observation.state] }}</span></div>
        <p class="hint">{{ row.kind }}<template v-if="row.project"> / {{ row.project }}</template> · Port {{ row.port ?? '尚未核對' }}</p>
        <p class="hint">{{ reasons[row.observation.reason] ?? '本次無法確認，請核對來源或重新查詢' }}<template v-if="row.observation.pid"> · {{ row.observation.process ? row.observation.process + ' · ' : '' }}PID {{ row.observation.pid }}</template></p>
        <p v-if="row.observation.runId" class="hint selectable">Host PID {{ row.observation.hostPid }} · Run {{ row.observation.runId }}</p>
        <p class="hint">核心：{{ row.core.identity ? 'Paper ' + row.core.identity.version : '版本尚未辨識' }}<template v-if="row.core.source"> · {{ row.core.source }}</template></p>
        <p class="hint selectable">Java：{{ row.java.path ?? row.java.alias ?? '尚未核對' }}<template v-if="row.java.source !== 'unknown'"> · {{ javaSources[row.java.source] }}</template><template v-if="row.java.path"> · {{ row.java.fileVerified ? '檔案存在，未執行' : '檔案未確認' }}</template></p>
        <p class="hint selectable">{{ row.path }}</p>
      </article>
    </OverlayScroll>
    <div class="dialog-actions">
      <button type="button" @click="close">關閉</button>
      <button class="primary" form="legacy-preview-form" :aria-disabled="busy">{{ preview ? '重新核對' : '核對來源' }}</button>
    </div>
  </dialog>
</template>

<style scoped>
.legacy-preview { width: min(46rem, calc(100vw - 2rem)); overscroll-behavior: contain; }
.legacy-preview[open] { display: flex; flex-direction: column; }
.legacy-preview form { display: grid; gap: .7rem; }
.legacy-preview details label { margin-top: .6rem; display: grid; gap: .3rem; }
.legacy-preview input { width: 100%; min-width: 0; }
.legacy-preview summary { cursor: pointer; }
.legacy-status { min-height: 1.2em; margin: .5rem 0; }
.legacy-results { flex: 1 1 auto; min-height: 4rem; max-height: 39vh; overflow-wrap: anywhere; }
.legacy-row { border-top: 1px solid var(--border); padding: .6rem 0; content-visibility: auto; contain-intrinsic-size: auto 9rem; }
.legacy-row p { margin: .18rem 0; }
.legacy-row-title { display: flex; justify-content: space-between; gap: .7rem; flex-wrap: wrap; }
</style>
