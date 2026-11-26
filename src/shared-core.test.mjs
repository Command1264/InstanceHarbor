import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('#62: the detail panel says how many instances share the core file', () => {
  assert.equal(translate('zh-TW', 'instances.sharedCore', { count: 3 }), '共用核心：3 台使用同一份')
  assert.match(translate('en', 'instances.sharedCore', { count: 3 }), /^Shared core: 3 /)
  assert.match(read('./InstancesPage.vue'), /core\?\.sharedUsers/)
  assert.match(read('../src-tauri/src/presentation.rs'), /core\["sharedUsers"\]/)
})

test('#62: Clear cache lists unused shared cores under their own heading', () => {
  assert.match(read('./components/CachePanel.vue'), /id: 'sharedCore', label: 'components\.cache\.sharedCore'/)
  assert.match(translate('zh-TW', 'components.cache.sharedCore'), /共用核心/)
  const backend = read('../src-tauri/src/cache_cleanup.rs')
  const categories = backend.slice(backend.indexOf('enum Category {'), backend.indexOf('}', backend.indexOf('enum Category {')))
  assert.match(categories, /SharedCore,/)
})

test('#62: the old shared-core.enabled experiment is gone', () => {
  assert.doesNotMatch(read('../src-tauri/src/windows.rs'), /shared-core\.enabled|verified_launch_jar/)
})
