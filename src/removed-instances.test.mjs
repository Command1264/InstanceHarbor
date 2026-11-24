import assert from 'node:assert/strict'
import { test } from 'node:test'
import { restoreJavaChoices, restoreRequestId } from './removed-instances.ts'

test('restore request IDs match the backend token rule', () => {
  for (const [now, random] of [[0, 0], [Date.now(), 0.999999], [8.64e15, 0.5]]) {
    const id = restoreRequestId(now, random)
    assert.match(id, /^[a-z0-9][a-z0-9-]{0,63}$/)
  }
  assert.notEqual(restoreRequestId(1, 0.1), restoreRequestId(1, 0.2))
})

test('the saved Java comes first and duplicates are dropped regardless of slashes or case', () => {
  assert.deepEqual(
    restoreJavaChoices('C:/Java/bin/java.exe', ['c:\\java\\bin\\java.exe', 'D:/jdk/bin/java.exe']),
    ['C:/Java/bin/java.exe', 'D:/jdk/bin/java.exe'],
  )
  assert.deepEqual(restoreJavaChoices(null, ['D:/jdk/bin/java.exe']), ['D:/jdk/bin/java.exe'])
  assert.deepEqual(restoreJavaChoices(null, []), [])
})
