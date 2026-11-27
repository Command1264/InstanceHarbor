import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import test from 'node:test'

// #116: src-tauri/icons/channels.json is the only place that pairs a channel with its icon
// files. Every consumer reads it: build.rs and the Rust code through app_icon.rs, the sidebar
// through vite.config.ts, the build checks through verify-app-icon.ps1 and the generator of
// the red files. These tests fail when a new place names an icon file or picks one itself.
const root = new URL('../', import.meta.url)
const MANIFEST = 'src-tauri/icons/channels.json'
const manifest = JSON.parse(readFileSync(new URL(MANIFEST, root), 'utf8'))
const CHANNELS = ['product', 'development']
const read = path => readFileSync(new URL(path, root), 'utf8').replace(/\r\n/g, '\n')

// Setup is built for the product channel only and #38 is rewriting its scripts; it keeps
// naming the blue icon until it can take the path from pack-windows-setup.ps1.
const NAMES_ICON_FILES = new Set([MANIFEST, 'src/channel-icons.test.mjs', 'packaging/windows-setup.nsi'])

function trackedTextFiles() {
  return execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
    .split('\0')
    .filter(path => path && !path.endsWith('.md') && existsSync(new URL(path, root)))
    .filter(path => !/\.(ico|png|jpe?g|gif|webp|woff2?|ttf|jar|zip|exe|dll|lock)$/i.test(path))
    .filter(path => !path.startsWith('packaging/windows-upstream-notices/') && !path.startsWith('src/vendor/'))
    .filter(path => !path.includes('package-lock.json'))
}

test('each channel has its own existing icon files', () => {
  assert.equal(manifest.resource, 32512, 'tauri-build always embeds the application icon as resource 32512')
  for (const kind of ['app', 'brand']) {
    const paths = CHANNELS.map(channel => manifest[channel][kind])
    assert.equal(new Set(paths).size, 2, `${kind}: the channels use different files`)
    for (const path of paths) {
      assert.ok(existsSync(new URL(path, root)), `${path} exists`)
    }
  }
})

test('only the manifest names the icon files or the icon resource', () => {
  const names = CHANNELS.flatMap(channel => Object.values(manifest[channel]))
    .map(path => path.split('/').pop())
  const offenders = []
  for (const path of trackedTextFiles()) {
    if (NAMES_ICON_FILES.has(path)) continue
    const text = readFileSync(new URL(path, root), 'utf8')
    for (const name of names) {
      if (text.includes(name)) offenders.push(`${path}: ${name}`)
    }
    if (text.includes(String(manifest.resource))) offenders.push(`${path}: ${manifest.resource}`)
  }
  assert.deepEqual(offenders, [], `read ${MANIFEST} instead`)
})

test('every consumer reads the manifest', () => {
  for (const path of [
    'src-tauri/src/app_icon.rs',
    'vite.config.ts',
    'packaging/verify-app-icon.ps1',
    'packaging/generate-development-icons.py',
  ]) {
    assert.match(read(path), /channels\.json/, `${path} reads the manifest`)
  }
  // build.rs and build-product.ps1 go through app_icon.rs and verify-app-icon.ps1.
  assert.match(read('src-tauri/build.rs'), /app_icon::for_channel\(development\)/)
  assert.match(read('packaging/build-product.ps1'), /verify-app-icon\.ps1'\) -Binary \$executable -Channel \$channel/)
})

test('Rust code takes window and tray icons from app_icon only', () => {
  // Every window gets Tauri's default window icon, which build.rs points at the channel
  // icon. One generate_context! keeps the GUI and the console viewer on the same icon.
  const files = execFileSync('git', ['ls-files', '-z', 'src-tauri/src', 'src-tauri/build.rs'], { cwd: root, encoding: 'utf8' })
    .split('\0').filter(path => path.endsWith('.rs'))
  const contexts = []
  const iconCalls = []
  for (const path of files) {
    const text = read(path)
    for (const match of text.matchAll(/tauri::generate_context!\(/g)) contexts.push(path)
    for (const match of text.matchAll(/(?:\.icon|set_icon|with_icon|set_default_window_icon|from_resource|from_path|from_bytes)\([^\n]*/g)) {
      iconCalls.push(`${path}: ${match[0].trim()}`)
    }
  }
  assert.deepEqual(contexts, ['src-tauri/src/main.rs'])
  assert.deepEqual(iconCalls, [
    'src-tauri/src/manager_tray.rs: from_resource(crate::app_icon::resource_id(), None) {',
    'src-tauri/src/manager_tray.rs: with_icon(icon),',
  ])
})

test('vite picks the sidebar image of its channel from the manifest', async () => {
  const { default: config } = await import(new URL('vite.config.ts', root))
  for (const [mode, channel] of [['production', 'product'], ['development-channel', 'development']]) {
    const resolved = (typeof config === 'function' ? config({ mode, command: 'build' }) : config).resolve.alias[0].replacement
    assert.ok(resolved.replace(/\\/g, '/').endsWith(manifest[channel].brand), `${mode} uses ${manifest[channel].brand}`)
  }
})
