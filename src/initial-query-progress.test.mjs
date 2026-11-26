import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

// Guards issue #93: full-page lists center their first-read progress in the visible scroll
// area; dialogs keep it left-aligned. The DevTools measurements live in the PR for #93.

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8')
const uses = (source) => [...source.matchAll(/<InitialQueryProgress\b[^>]*>/g)].map((match) => match[0])

test('full-page lists use the centered progress', async () => {
  for (const page of ['./InstancesPage.vue', './ActivityPage.vue']) {
    const found = uses(await read(page))
    assert.ok(found.length, `${page} shows InitialQueryProgress`)
    for (const tag of found) assert.match(tag, /\scentered[\s/>]/, `${page}: ${tag}`)
  }
})

test('dialogs keep the left-aligned progress', async () => {
  for (const source of ['./App.vue', './components/RemovedInstancesDialog.vue', './components/CreateCorePicker.vue']) {
    for (const tag of uses(await read(source))) assert.doesNotMatch(tag, /\scentered[\s/>]/, `${source}: ${tag}`)
  }
})

test('centered progress is positioned against the non-scrolling shell', async () => {
  const component = await read('./components/InitialQueryProgress.vue')
  const rule = component.match(/\.initial-query-progress\.centered\s*\{([^}]*)\}/)
  assert.ok(rule, 'centered rule exists')
  // Out of the scrolled content, so horizontal scrolling cannot move it off the visible area.
  assert.match(rule[1], /position:\s*absolute/)
  assert.match(rule[1], /place-items:\s*center/)
  // OverlayScroll's shell is the positioned ancestor that does not scroll.
  const overlay = await read('./components/OverlayScroll.vue')
  assert.match(overlay, /\.scroll-shell\s*\{[^}]*position:relative/)
})
