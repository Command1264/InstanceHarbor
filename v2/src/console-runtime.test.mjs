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

test('a 1.0 package and a release build refuse to ship without the tier-1 console', () => {
  assert.match(read('../pack-windows.ps1'), /\$consoleNames\.Count -ne 2 -and \$Version -notlike '1\.0\.0-rc\*'/)
  assert.match(read('../build-product.ps1'), /elseif\(\$Release\)\{\s*throw/)
})

test('the retired workspace switch file is gone', () => {
  for (const path of ['../src-tauri/src/windows.rs', '../src-tauri/src/presentation.rs']) {
    assert.doesNotMatch(read(path), /native-console\.enabled"\)\.(is_file|exists)\(\)|console_marker/, path)
  }
})
