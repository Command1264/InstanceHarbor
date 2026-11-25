import assert from 'node:assert/strict'
import { test } from 'node:test'
import { ScopeCache } from './creation-scope-cache.ts'

test('a confirmation keeps one preview per folder and drops replies for an older confirmation', () => {
  const cache = new ScopeCache()
  const first = cache.reset()
  assert.equal(cache.set(first, 'C:/ws/instances/group', 'parent'), true)
  assert.equal(cache.set(first, '', 'none'), true)
  assert.equal(cache.get('C:/ws/instances/group'), 'parent')
  assert.equal(cache.get(''), 'none')

  const second = cache.reset()
  assert.notEqual(second, first)
  assert.equal(cache.get('C:/ws/instances/group'), undefined)
  assert.equal(cache.set(first, 'C:/ws/instances', 'late'), false)
  assert.equal(cache.get('C:/ws/instances'), undefined)
  assert.equal(cache.generation, second)
})
