import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

// Interface text lives in i18n.ts and messages/*.ts so both languages stay complete.
// List a line here only when the Chinese text is data, not interface text.
const ALLOWED = [
  // A language's own name stays the same in every interface language.
  'components/LanguageSetting.vue: <option value="zh-TW" lang="zh-TW">繁體中文</option>',
  // Policy categories are IDs sent by policy_catalog.rs; labels come from messages.
  "components/PolicyFields.vue: const category = ref('伺服器')",
  "components/PolicyFields.vue: const categories = ['伺服器', 'Spigot／Bukkit', 'Paper', '世界', '遊戲規則']",
  "components/PolicyFields.vue: '伺服器': 'components.policy.category.server', 'Spigot／Bukkit': 'components.policy.category.spigot', Paper: 'components.policy.category.paper',",
  "components/PolicyFields.vue: '世界': 'components.policy.category.world', '遊戲規則': 'components.policy.category.gamerules'",
  "components/PolicyFields.vue: if (!choices.includes(category.value)) category.value = choices[0] ?? '伺服器'",
  "policy-settings.ts: return row.category === '遊戲規則' && row.key !== 'initialTime'",
]

const root = new URL('.', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')
const sources = [
  ...readdirSync(root).filter(name => /\.(vue|ts|js)$/.test(name) && name !== 'i18n.ts'),
  ...readdirSync(join(root, 'components')).filter(name => name.endsWith('.vue')).map(name => `components/${name}`),
]
const cjk = /[\u3400-\u9fff\uff01-\uff5e]/

function withoutComments(source) {
  return source
    .replace(/<!--[\s\S]*?-->/g, match => match.replace(/[^\n]/g, ' '))
    .replace(/\/\*[\s\S]*?\*\//g, match => match.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, (match, lead) => lead + ' '.repeat(match.length - lead.length))
}

test('interface text comes from i18n messages', () => {
  const found = []
  for (const name of sources) {
    const lines = withoutComments(readFileSync(join(root, name), 'utf8')).split(/\r?\n/)
    for (const line of lines) {
      const entry = `${name}: ${line.trim()}`
      if (cjk.test(line) && !ALLOWED.includes(entry)) found.push(entry)
    }
  }
  assert.deepEqual(found, [])
})
