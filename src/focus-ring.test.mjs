import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'

// Guards issue #27: a focused field flush with a dialog's scroll area must not lose its focus ring
// to the viewport's overflow clip. The ring extends --focus-ring-width + --focus-ring-offset past
// the field, so every edge of a dialog scroll viewport has to reserve at least that much.

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8')
const appCss = await read('./app.css')
const overlay = await read('./components/OverlayScroll.vue')
const overlayStyle = overlay.slice(overlay.indexOf('<style'))

function rules(css) {
  const found = []
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '')
  for (const match of stripped.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    found.push({ selector: match[1].trim(), body: match[2] })
  }
  return found
}
function declarations(body) {
  return Object.fromEntries(body.split(';').map((part) => part.split(/:(.*)/s).map((v) => v?.trim()))
    .filter(([name, value]) => name && value).map(([name, value]) => [name, value]))
}
function pixels(css, name) {
  const match = css.match(new RegExp(`${name}:\\s*(\\d+(?:\\.\\d+)?)px`))
  assert.ok(match, `${name} must be a px token`)
  return Number(match[1])
}

const ringWidth = pixels(appCss, '--focus-ring-width')
const ringOffset = pixels(appCss, '--focus-ring-offset')
const extent = ringWidth + ringOffset

// A padding side is safe when it reserves the ring token itself or a literal px value at least that wide.
function reservesRing(value) {
  if (/var\(--focus-ring-extent\)/.test(value) && !/-\s*1\s*\*|-var/.test(value)) return true
  const px = value.match(/^(\d+(?:\.\d+)?)px$/)
  return Boolean(px) && Number(px[1]) >= extent
}
// Splits a shorthand on top-level whitespace, keeping calc(... var(...)) together.
function splitValues(value) {
  const parts = []
  let depth = 0
  let current = ''
  for (const char of value) {
    if (char === '(') depth++
    if (char === ')') depth--
    if (/\s/.test(char) && depth === 0) {
      if (current) parts.push(current)
      current = ''
    } else current += char
  }
  if (current) parts.push(current)
  return parts
}
function paddingSides(decls) {
  const sides = {}
  if (decls.padding) {
    const parts = splitValues(decls.padding)
    const [top, right = top, bottom = top, left = right] = parts
    Object.assign(sides, { top, right, bottom, left })
  }
  for (const side of ['top', 'right', 'bottom', 'left']) if (decls[`padding-${side}`]) sides[side] = decls[`padding-${side}`]
  return sides
}

test('the global focus ring is drawn from the shared ring tokens', () => {
  const focus = rules(appCss).find((rule) => rule.selector === ':focus-visible')
  assert.ok(focus, ':focus-visible rule must exist')
  const decls = declarations(focus.body)
  assert.match(decls.outline, /var\(--focus-ring-width\)/)
  assert.equal(decls['outline-offset'], 'var(--focus-ring-offset)')
  assert.match(appCss, /--focus-ring-extent:\s*calc\(var\(--focus-ring-width\)\s*\+\s*var\(--focus-ring-offset\)\)/)
})

test('dialog scroll viewports reserve the ring on the top and left edges without moving content', () => {
  const shell = rules(overlayStyle).find((rule) => rule.selector === ':where(dialog) .scroll-shell')
  assert.ok(shell, 'OverlayScroll must let its dialog shell show the reserved edge')
  assert.equal(declarations(shell.body).overflow, 'visible')
  const viewport = rules(overlayStyle).find((rule) => rule.selector === ':where(dialog) .overlay-viewport')
  assert.ok(viewport, 'OverlayScroll must reserve the ring inside dialogs')
  const decls = declarations(viewport.body)
  for (const side of ['top', 'left']) {
    assert.equal(decls[`padding-${side}`], 'var(--focus-ring-extent)', `padding-${side}`)
    assert.equal(decls[`margin-${side}`], 'calc(-1 * var(--focus-ring-extent))', `margin-${side} keeps content in place`)
  }
  // :global() would compile to a bare `dialog` selector and restyle every dialog box.
  assert.doesNotMatch(overlayStyle, /:global\(/)
})

test('base overlay viewport keeps room for the ring on its right and bottom edges', () => {
  const base = rules(overlayStyle).find((rule) => rule.selector === '.overlay-viewport')
  const sides = paddingSides(declarations(base.body))
  for (const side of ['right', 'bottom']) assert.ok(reservesRing(sides[side]), `base padding-${side} ${sides[side]}`)
})

test('no overlay viewport override shrinks an edge below the focus ring extent', async () => {
  const sources = [['app.css', appCss]]
  for (const dir of ['./', './components/']) {
    for (const name of await readdir(new URL(dir, import.meta.url))) {
      if (!name.endsWith('.vue')) continue
      const text = await read(`${dir}${name}`)
      const styles = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((match) => match[1]).join('\n')
      sources.push([name, styles])
    }
  }
  let checked = 0
  for (const [file, css] of sources) {
    for (const rule of rules(css)) {
      if (!/overlay-viewport/.test(rule.selector) || /::/.test(rule.selector)) continue
      for (const [side, value] of Object.entries(paddingSides(declarations(rule.body)))) {
        checked++
        assert.ok(reservesRing(value), `${file}: ${rule.selector} padding-${side} is ${value}, below the ${extent}px focus ring`)
      }
    }
  }
  assert.ok(checked > 0, 'expected overlay viewport padding rules to check')
})

test('scroll viewport is a tab stop only while it can scroll', () => {
  const viewport = overlay.match(/<div[^>]*class="overlay-viewport"[^>]*>/)
  assert.ok(viewport, 'OverlayScroll must render an overlay-viewport')
  assert.doesNotMatch(viewport[0], /\stabindex="0"/, 'a fixed tabindex makes every non-scrolling area a tab stop')
  assert.match(viewport[0], /:tabindex="scrollable \? 0 : undefined"/)
  assert.match(overlay, /const scrollable = computed\(\(\) => needed\.value \|\| maximumX\.value > 1\)/)
})
