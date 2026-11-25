import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { test } from 'node:test'
import { translate } from './i18n.ts'
import { coreBuildText, coreFamilyName, coreReasonId, coreVersionText } from './core-display.ts'

test('every reason the backend gives for an unknown core is translated', () => {
  const source = readFileSync(new URL('../src-tauri/src/presentation.rs', import.meta.url), 'utf8')
  const body = source.slice(source.indexOf('fn current_core('), source.indexOf('/// `nativeConsole`'))
  const codes = [...new Set([...body.matchAll(/"reason":"(\w+)"/g)].map(match => match[1]))]
  assert.deepEqual(codes.sort(), ['CorePathUnverified', 'CoreProfileUnverified', 'CoreUnreadable'])
  for (const code of codes) {
    const id = coreReasonId(code)
    assert.notEqual(id, 'instances.coreReason.other', code)
    assert.match(translate('zh-TW', id), /\p{Script=Han}/u, code)
    assert.match(translate('en', id), /^[A-Z]/, code)
  }
})

test('an unknown core reason falls back to a general message and no reason shows nothing', () => {
  assert.equal(coreReasonId('SomethingNew'), 'instances.coreReason.other')
  assert.equal(coreReasonId(undefined), undefined)
  assert.equal(coreReasonId(''), undefined)
})

test('an offline reading of the core version is labelled briefly and explained once', () => {
  assert.equal(translate('zh-TW', 'components.coreMaintenance.offline'), '（離線辨識）')
  assert.equal(translate('en', 'components.coreMaintenance.offline'), ' (read offline)')
  // The explanation says what was not compared, not that a connection failed.
  assert.match(translate('zh-TW', 'instances.coreOffline'), /核心檔.*未與內建清單或官方來源比對/)
  assert.match(translate('en', 'instances.coreOffline'), /read from the core file.*built-in list or the official source/)
})

test('core families are shown with an initial capital whatever the source wrote', () => {
  for (const [stored, shown] of [['PAPER', 'Paper'], ['Paper', 'Paper'], ['folia', 'Folia'], ['SPIGOT', 'Spigot'],
    ['VELOCITY', 'Velocity'], ['PURPUR', 'Purpur']]) assert.equal(coreFamilyName(stored), shown, stored)
})

test('an unknown core family is shown as written and never as blank', () => {
  assert.equal(coreFamilyName('LeafMC'), 'LeafMC')
  assert.equal(coreFamilyName('CUSTOM'), 'CUSTOM')
  assert.equal(coreFamilyName(''), '')
  assert.equal(coreFamilyName(undefined), '')
})

test('a core version reads family, Minecraft version and Build in one form', () => {
  assert.equal(coreBuildText('134'), 'Build 134')
  assert.equal(coreVersionText({ family: 'PAPER', minecraft: '26.2', build: '134' }), 'Paper 26.2 · Build 134')
  assert.equal(coreVersionText({ family: 'FOLIA', minecraft: '26.1.2', build: '8' }), 'Folia 26.1.2 · Build 8')
  assert.equal(coreVersionText({ family: 'VELOCITY', minecraft: '4.2.1-SNAPSHOT', build: '' }), 'Velocity 4.2.1-SNAPSHOT')
  assert.equal(coreVersionText({ family: 'LeafMC', minecraft: '1.21.4', build: null }), 'LeafMC 1.21.4')
})

test('templates show core families and builds only through core-display', () => {
  const root = new URL('./', import.meta.url)
  const files = ['App.vue', 'InstancesPage.vue', 'SettingsPage.vue', ...readdirSync(new URL('components/', root)).map(name => `components/${name}`)]
    .filter(name => name.endsWith('.vue'))
  for (const name of files) {
    const source = readFileSync(new URL(name, root), 'utf8')
    for (const match of source.matchAll(/\{\{([^}]*)\}\}/g)) {
      assert.doesNotMatch(match[1], /\.family\b(?![^(]*\))/, `${name}: ${match[0]}`)
    }
    assert.doesNotMatch(source, />\s*Build \{\{|'build',/, `${name} writes Build by hand`)
  }
  for (const id of ['core.batch.updateTo', 'components.coreMaintenance.targetBuild']) {
    for (const locale of ['zh-TW', 'en']) assert.match(translate(locale, id, { build: '134' }), /Build 134/, `${locale} ${id}`)
  }
})
