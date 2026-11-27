import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import test from 'node:test'

// #97: build and packaging scripts live in packaging/, launch scripts in scripts/,
// acceptance scripts in tests/acceptance/. The root keeps project files only.
const root = new URL('../', import.meta.url)

test('the repository root holds no scripts', () => {
  const scripts = readdirSync(root).filter(name => /\.(ps1|py|mjs|nsi|nsh)$/i.test(name))
  assert.deepEqual(scripts, [])
})

test('the automatic development workspace still hashes the checkout root', () => {
  const launcher = readFileSync(new URL('scripts/launch-development.ps1', root), 'utf8')
  assert.match(launcher, /\$checkout = \(Resolve-Path -LiteralPath \(Split-Path -Parent \$PSScriptRoot\)\)/)
})
