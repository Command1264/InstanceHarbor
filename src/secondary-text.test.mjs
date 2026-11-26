import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

// UI rule: no explanatory subtitles; keep only text a user needs to decide or act.
// Always-visible hint and notice lines must be listed below on purpose.
const ALLOWED_ALWAYS_VISIBLE = [
  "InstancesPage.vue: {{ t('instances.selectHint') }}",
  "SettingsPage.vue: {{ t('settings.javaCheckHelp') }}",
  "SettingsPage.vue: {{ t('settings.restartScheduleHelp') }}",
  "SettingsPage.vue: {{ t('settingsPage.jvmFlagsHelp') }}",
  "SettingsPage.vue: {{ world.profile ? coreFamilyName(world.profile.family) + ' ' + world.profile.version : t('settingsPage.worldProfilePending') }}",
  "SettingsPage.vue: {{ t('settingsPage.worldApplyNotice') }}",
  "SettingsPage.vue: {{ t('settingsPage.yamlChangedFiles') }}",
  "SettingsPage.vue: {{ t('settingsPage.scanHelp') }}",
  // Saving a new default must not suggest that existing servers move.
  "SettingsPage.vue: {{ t('settingsPage.creationParentHelp') }}",
  // Fast open trades memory for speed (#56); the measured cost must be seen before turning it on.
  "SettingsPage.vue: {{ t('appearance.fastOpenHint') }}",
  "App.vue: {{ selectedLog?.path ?? logs?.path ?? t('app.logs.noPath') }}",
  // Starting in a console window needs this consequence stated before the user decides.
  "App.vue: {{ t('console.confirm.warning') }}",
  // Each removal option states what it keeps and what it deletes (#26); the other two hints are computed.
  "App.vue: {{ t('app.remove.retainHint') }}",
  // Windows may offer permanent deletion; the user needs to know that No cancels before choosing.
  "App.vue: {{ t('app.remove.nukeWarning') }}",
  // The exact folder that deleting server data moves to the Recycle Bin.
  "App.vue: {{ t('app.remove.folder',{path:removeMeta?.path?displayPath(removeMeta.path,visiblePreferences.pathSeparator):'--'}) }}",
  // Installing waits for running servers (#61, #65); the user needs that before confirming.
  "components/AppUpdatePanel.vue: {{ t('components.update.installServers') }}",
  "components/DisplayPreferencesFields.vue: {{ t('appearance.preview') }} · {{ formatDisplayTime(sampleTime, locale, timeFormat) }}",
  "components/InstanceDiagnosticsPanel.vue: {{ t('diagnostics.jarHelp') }}",
  "components/InstanceDiagnosticsPanel.vue: {{ t('diagnostics.source', { source: sourceText(signal.source) }) }}",
  "components/JavaSearchScope.vue: {{ t('management.javaSearchHelp') }}",
  // Existing instances are never granted automatically; the manual steps must stay visible.
  "components/TestAdminFields.vue: {{ t('components.testAdmins.existingHelp') }}",
  "components/ManagementFolderDialog.vue: {{ t('management.summary', { selected: selectedCount, excluded: excluded.length, blocked: pendingRows.length, }) }}",
  "components/WorldLiveControls.vue: {{ t('components.worldTime.unsupportedHelp') }} {{ t('components.worldTime.notReadyHelp') }} {{ t('components.worldTime.readyHelp') }}",
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
