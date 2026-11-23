import test from 'node:test'
import assert from 'node:assert/strict'
import { displayPath } from './path-display.ts'
test('Windows drive and extended UNC display use the selected separator',()=>{
  assert.equal(displayPath('\\\\?\\C:\\Java\\bin\\java.exe','forward','windows'),'C:/Java/bin/java.exe')
  assert.equal(displayPath('\\\\?\\UNC\\host\\share\\java.exe','forward','windows'),'//host/share/java.exe')
  assert.equal(displayPath('C:/Java/bin/java.exe','native','windows'),'C:\\Java\\bin\\java.exe')
  assert.equal(displayPath('\\\\.\\pipe\\host','forward','windows'),'\\\\.\\pipe\\host')
})
test('Unix and macOS retain literal backslashes rather than guessing a different file',()=>{
  assert.equal(displayPath('/home/a\\b','native','linux'),'/home/a\\b')
  assert.equal(displayPath('/Users/a\\b','forward','macos'),'/Users/a\\b')
})
