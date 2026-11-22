import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

// UI rule: no explanatory subtitles; keep only text a user needs to decide or act.
// Always-visible hint and notice lines must be listed below on purpose.
const ALLOWED_ALWAYS_VISIBLE = [
  'InstancesPage.vue: 選一台查看資訊與操作，或按 Ctrl／Shift 選取多台。',
  "SettingsPage.vue: {{ t('appearance.closeToTrayHelp') }}",
  "SettingsPage.vue: {{ t('settings.javaCheckHelp') }}",
  "SettingsPage.vue: {{ t('settings.restartScheduleHelp') }}",
  'SettingsPage.vue: 每行一個旗標，可用 ActiveProcessorCount、ParallelGCThreads、ConcGCThreads、MaxGCPauseMillis、InitiatingHeapOccupancyPercent、G1ReservePercent。例如 -XX:MaxGCPauseMillis=100。',
  "SettingsPage.vue: {{ world.profile ? world.profile.family + ' ' + world.profile.version : '核心與 Minecraft 版本待核對' }}",
  'SettingsPage.vue: 設定檔變更在正常停止後套用；遊戲規則即時套用。',
  'SettingsPage.vue: 內容有變更的檔案',
  'SettingsPage.vue: 可用完整路徑或 ./instances 等相對路徑。',
  "App.vue: {{ selectedLog?.path ?? logs?.path ?? '尚未取得紀錄路徑' }}",
  "components/DisplayPreferencesFields.vue: {{ t('appearance.preview') }} · {{ formatDisplayTime(sampleTime, locale, timeFormat) }}",
  "components/InstanceDiagnosticsPanel.vue: {{ t('diagnostics.jarHelp') }}",
  "components/InstanceDiagnosticsPanel.vue: {{ t('diagnostics.source', { source: sourceText(signal.source) }) }}",
  "components/JavaSearchScope.vue: {{ t('management.javaSearchHelp') }}",
  "components/ManagementFolderDialog.vue: {{ t('management.summary', { selected: selectedCount, excluded: excluded.length, blocked: pendingRows.length, }) }}",
  'components/WorldLiveControls.vue: 尚未核對目前核心與版本是否支援調整世界時間。 伺服器就緒後可即時調整世界時間。 確認後即時調整目前運行中的世界。請輸入 0 到 23999 刻。',
]

const root = new URL('.', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')
const files = [
  ...readdirSync(root).filter(name => name.endsWith('.vue')),
  ...readdirSync(join(root, 'components')).filter(name => name.endsWith('.vue')).map(name => `components/${name}`),
]
const templates = files.map(name => {
  const source = readFileSync(join(root, name), 'utf8')
  return { name, template: source.slice(Math.max(0, source.indexOf('<template'))) }
})

test('page headings have no subtitle paragraph', () => {
  const offenders = templates
    .filter(({ template }) => /<\/h1>\s*<p\b/.test(template))
    .map(({ name }) => name)
  assert.deepEqual(offenders, [])
})

test('always-visible hint and notice text is an explicit decision', () => {
  const found = []
  const element = /<(p|small|div|span)\b([^>]*class="[^"]*\b(?:hint|notice)\b[^"]*"[^>]*)>([\s\S]*?)<\/\1>/g
  for (const { name, template } of templates) {
    for (const match of template.matchAll(element)) {
      if (/\bv-(if|else-if|else|show)\b/.test(match[2])) continue
      const text = match[3].replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim()
      // Data such as paths or times uses the same style; only worded text is a decision.
      if (/[一-鿿]|\bt\('/.test(text)) found.push(`${name}: ${text}`)
    }
  }
  assert.deepEqual(found.sort(), [...ALLOWED_ALWAYS_VISIBLE].sort())
})
