import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { provideAppLanguage, tr } from './i18n.ts'

// tr() serves code outside components (manager.ts, policy-settings.ts); it must follow the provided app language.
test('tr follows the language chosen through provideAppLanguage', async () => {
  let seen = ''
  const app = createSSRApp({
    setup() {
      provideAppLanguage(() => 'en', () => undefined)
      seen = tr('activity.state.Running')
      return () => h('div')
    },
  })
  await renderToString(app)
  assert.equal(seen, 'In progress')
})
