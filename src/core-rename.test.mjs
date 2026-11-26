import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('#112: the confirmations name the new core file', () => {
  for (const locale of ['zh-TW', 'en']) {
    assert.match(translate(locale, 'components.coreMaintenance.renameTo', { name: 'velocity-4.3.1-SNAPSHOT-52.jar' }), /velocity-4\.3\.1-SNAPSHOT-52\.jar/)
    assert.match(translate(locale, 'components.coreMaintenance.restoreName', { name: 'velocity-4.2.1-SNAPSHOT-38.jar' }), /velocity-4\.2\.1-SNAPSHOT-38\.jar/)
  }
  assert.match(translate('zh-TW', 'components.coreMaintenance.renameTo', { name: 'x.jar' }), /^檔名會改為 x\.jar/)
  assert.match(translate('zh-TW', 'components.coreMaintenance.restoreName', { name: 'x.jar' }), /^檔名會改回 x\.jar/)
  // Single update and restore, the cross-version summary and the batch preview.
  assert.match(read('./components/CoreMaintenanceDialog.vue'), /choice\?\.renameTo/)
  assert.match(read('./components/VelocityVersionDialog.vue'), /target\.renameTo/)
  assert.match(read('./components/BatchCoreDialog.vue'), /row\.source\?\.renameTo/)
})

test('#112: the backend states the new name where the confirmations read it', () => {
  const source = read('../src-tauri/src/core_source.rs')
  assert.match(source, /row\["renameTo"\]/)
  assert.match(source, /"renameTo":rename/)
  assert.match(read('../src-tauri/src/core_maintenance.rs'), /"renameTo": file/)
})
