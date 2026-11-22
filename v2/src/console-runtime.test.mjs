import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
const manifest = JSON.parse(read('../console-runtime.json'))

// native_console.rs checks the same pins through a Rust test.
test('packaging and installation pin the console runtime in console-runtime.json', () => {
  for (const script of ['../pack-windows.ps1', '../install-windows.ps1']) {
    const source = read(script)
    for (const [name, { sha256 }] of Object.entries(manifest.files)) {
      assert.ok(source.includes(`'${name}' = '${sha256}'`), `${script} does not pin ${name}`)
    }
  }
})

test('the runtime record names the official package and its x64 entries', () => {
  const { source, files } = manifest
  assert.match(source.url, /^https:\/\/api\.nuget\.org\/v3-flatcontainer\/microsoft\.windows\.console\.conpty\/1\.25\.260930003\//)
  assert.match(source.sha256, /^[0-9a-f]{64}$/)
  assert.ok(Number.isInteger(source.bytes) && source.bytes > 0)
  assert.equal(files['conpty.dll'].entry, 'runtimes/win-x64/native/conpty.dll')
  assert.equal(files['OpenConsole.exe'].entry, 'build/native/runtimes/x64/OpenConsole.exe')
  const docs = read('../../docs/windows-distribution-licenses.md')
  for (const value of [source.url, source.sha256]) assert.ok(docs.includes(value), `the distribution guide lacks ${value}`)
})

test('stable versions are distributions for the ZIP and Setup', () => {
  const pack = read('../pack-windows.ps1')
  assert.ok(pack.includes(String.raw`$distribution = $Version -ceq '1.0.0-rc2' -or $Version -cmatch '^\d+\.\d+\.\d+$'`))
  const setup = read('../pack-windows-setup.ps1')
  assert.doesNotMatch(setup, /\/DVERSION=1\.0\.0-rc2/)
  assert.match(setup, /!\$prepared\.nativeConsoleRuntime/)
})

test('a 1.0 package and a release build refuse to ship without the tier-1 console', () => {
  assert.match(read('../pack-windows.ps1'), /\$consoleNames\.Count -ne 2 -and \$Version -notlike '1\.0\.0-rc\*'/)
  assert.match(read('../build-product.ps1'), /elseif\(\$Release\)\{\s*throw/)
})

test('the retired workspace switch file is gone', () => {
  for (const path of ['../src-tauri/src/windows.rs', '../src-tauri/src/presentation.rs']) {
    assert.doesNotMatch(read(path), /native-console\.enabled"\)\.(is_file|exists)\(\)|console_marker/, path)
  }
})

test('stable executables need no Visual C++ Redistributable', () => {
  assert.match(read('../.cargo/config.toml'), /\[target\.x86_64-pc-windows-msvc\]\s*rustflags = \["-C", "target-feature=\+crt-static"\]/)
  const pack = read('../pack-windows.ps1')
  assert.ok(pack.includes(String.raw`Where-Object { $_ -match '^(vcruntime|msvcp)\d' }`))
  assert.doesNotMatch(read('../windows-setup.nsi'), /VC_RUNTIME|vc_redist/i)
})

test('Setup checks WebView2 on every run and compares the installed version', () => {
  const setup = read('../windows-setup.nsi')
  const init = setup.slice(setup.indexOf('Function .onInit'), setup.indexOf('FunctionEnd', setup.indexOf('Function .onInit')))
  // Before any installed-version branch, so an update or repair also checks it.
  assert.ok(init.indexOf('Call RequireWebView2') < init.indexOf('ReadRegStr $0 HKCU "${UNINSTALL_KEY}"'))
  assert.match(init, /Call CompareVersions/)
  assert.match(init, /\$\(DowngradeAsk\)" \/SD IDNO/)
  for (const name of ['WebView2Missing', 'UpdateAsk', 'RepairAsk', 'DowngradeAsk', 'InUse']) {
    for (const language of ['LANG_TRADCHINESE', 'LANG_ENGLISH']) {
      assert.ok(setup.includes(`LangString ${name} ${'$'}{${language}}`), `${name} ${language}`)
    }
  }
  assert.ok(setup.includes('linkid=2124701'))
  assert.ok(read('../src-tauri/src/webview_runtime.rs').includes('linkid=2124701'))
})

test('the GUI checks WebView2 before it takes the single-instance lock', () => {
  const main = read('../src-tauri/src/main.rs')
  const check = main.indexOf('webview_runtime::ensure_available()')
  assert.ok(check > 0 && check < main.indexOf('gui_singleton::acquire()'))
})
