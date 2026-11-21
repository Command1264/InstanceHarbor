<script setup lang="ts">
import { computed, ref } from 'vue'
import { manage, stateName } from '../manager'
import OverlayScroll from './OverlayScroll.vue'

type FieldState = 'Match' | 'Different' | 'NoTarget' | 'Unsupported' | 'Unverified'
type Field = {
  key: string
  label: string
  source: string
  current?: string | null
  target?: string | null
  targetSource?: string | null
  status: FieldState
  reason: string
}
type Instance = {
  name: string
  profile?: { family: string; version: string } | null
  state?: string | null
  pending?: boolean | null
  error?: string
  fields: Field[]
}
type Preview = { instances: Instance[]; sourceRevision: string }

const dialog = ref<HTMLDialogElement>()
const names = ref<string[]>([])
const result = ref<Preview>()
const marked = ref(new Map<string, Set<string>>())
const differencesOnly = ref(false)
const loading = ref(false)
const error = ref('')
let request = 0
const statusLabels: Record<FieldState, string> = {
  Match: '已一致',
  Different: '有差異',
  NoTarget: '沒有目標值',
  Unsupported: '不支援',
  Unverified: '尚未核對',
}
const sourceLabels: Record<string, string> = {
  VersionOverride: '核心版本覆寫',
  WorkspaceDefault: '工作區預設',
  BuiltInDefault: '內建預設',
}
const markedCount = computed(() => [...marked.value.values()].reduce((total, keys) => total + keys.size, 0))

function differences(instance: Instance) {
  return instance.fields.filter(field => field.status === 'Different')
}

function visibleFields(instance: Instance) {
  return differencesOnly.value ? differences(instance) : instance.fields
}

function isMarked(name: string, key: string) {
  return marked.value.get(name)?.has(key) ?? false
}

function markedFor(instance: Instance) {
  return differences(instance).filter(field => isMarked(instance.name, field.key)).length
}

function markFields(name: string, fields: Field[], checked: boolean) {
  const keys = new Set(marked.value.get(name))
  for (const field of fields) {
    if (checked) keys.add(field.key)
    else keys.delete(field.key)
  }
  if (keys.size) marked.value.set(name, keys)
  else marked.value.delete(name)
}

function retainMatchingMarks(next: Preview) {
  const previous = new Map(result.value?.instances.map(instance => [instance.name, instance]))
  const retained = new Map<string, Set<string>>()
  for (const instance of next.instances) {
    const previousFields = new Map(previous.get(instance.name)?.fields.map(field => [field.key, field]))
    const keys = differences(instance).filter(field =>
      isMarked(instance.name, field.key) && JSON.stringify(previousFields.get(field.key)) === JSON.stringify(field),
    ).map(field => field.key)
    if (keys.length) retained.set(instance.name, new Set(keys))
  }
  marked.value = retained
}

async function refresh() {
  if (loading.value || !names.value.length || names.value.length > 32) return
  const turn = ++request
  loading.value = true
  error.value = ''
  try {
    const next = await manage<Preview>(['world-batch-preview', JSON.stringify(names.value)], 'manual')
    if (turn !== request || !dialog.value?.open) return
    if (next.instances.length !== names.value.length ||
        next.instances.some((instance, index) => instance.name !== names.value[index])) {
      throw new Error('Unexpected world batch preview targets')
    }
    retainMatchingMarks(next)
    result.value = next
  } catch {
    if (turn === request && dialog.value?.open) {
      error.value = '無法讀取批次差異。請核對實例與工作區預設後重試。'
    }
  } finally {
    if (turn === request) loading.value = false
  }
}

function reset() {
  request++
  names.value = []
  result.value = undefined
  marked.value.clear()
  differencesOnly.value = false
  loading.value = false
  error.value = ''
}

function open(selected: string[]) {
  if (!selected.length || dialog.value?.open) return
  reset()
  names.value = [...selected]
  dialog.value?.showModal()
  if (selected.length > 32) {
    error.value = '每次最多預覽 32 台實例。請縮小選取範圍後再試。'
    return
  }
  void refresh()
}

defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="world-batch-dialog" aria-labelledby="world-batch-title" @close="reset">
    <h2 id="world-batch-title">世界設定差異預覽</h2>
    <p>比較所選實例與工作區預設。標記只供這次檢視。</p>
    <div class="preview-toolbar">
      <label><input v-model="differencesOnly" type="checkbox"> 只顯示有差異的欄位</label>
      <span role="status">{{ names.length }} 台實例 · 已標記 {{ markedCount }} 欄</span>
    </div>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <p v-if="loading && !result" role="status">讀取所選實例設定中</p>
    <OverlayScroll v-if="result" class="preview-content" label="逐台世界設定差異">
      <section v-for="instance in result.instances" :key="instance.name" class="preview-instance">
        <h3>{{ instance.name }}</h3>
        <p class="hint">
          {{ instance.profile ? instance.profile.family + ' ' + instance.profile.version : '核心尚未核對' }} ·
          {{ instance.state ? stateName(instance.state) : '狀態尚未核對' }}
          <template v-if="instance.pending"> · 有設定待處理</template>
        </p>
        <p v-if="instance.error" class="form-error">{{ instance.error }}</p>
        <template v-else>
          <label v-if="differences(instance).length" class="mark-instance">
            <input type="checkbox"
                   :checked="markedFor(instance) === differences(instance).length"
                   :indeterminate="markedFor(instance) > 0 && markedFor(instance) < differences(instance).length"
                   :aria-label="'標記 ' + instance.name + ' 的所有差異'"
                   @change="markFields(instance.name, differences(instance), ($event.target as HTMLInputElement).checked)">
            標記本台所有差異
          </label>
          <p v-if="!visibleFields(instance).length">沒有已核對的差異</p>
          <ul v-else class="preview-fields">
            <li v-for="field in visibleFields(instance)" :key="field.key" :data-field="field.key">
              <div class="field-label">
                <template v-if="field.status === 'Different'">
                  <input :id="'world-preview-' + instance.name + ':' + field.key"
                         type="checkbox" :checked="isMarked(instance.name, field.key)"
                         :aria-label="'標記 ' + instance.name + ' 的' + field.label"
                         @change="markFields(instance.name, [field], ($event.target as HTMLInputElement).checked)">
                  <label :for="'world-preview-' + instance.name + ':' + field.key">{{ field.label }}</label>
                </template>
                <span v-else>{{ field.label }}</span>
                <small v-if="field.source" class="hint">{{ field.source }}</small>
              </div>
              <dl class="value-pair">
                <div><dt>目前值</dt><dd>{{ field.current === '' ? '空白' : field.current ?? '尚無' }}</dd></div>
                <div><dt>目標值</dt><dd>{{ field.target === '' ? '空白' : field.target ?? '尚無' }}</dd></div>
              </dl>
              <div>
                <strong>{{ statusLabels[field.status] }}</strong>
                <small v-if="field.targetSource" class="hint">{{ sourceLabels[field.targetSource] }}</small>
                <small v-if="field.reason" class="hint">{{ field.reason }}</small>
              </div>
            </li>
          </ul>
        </template>
      </section>
    </OverlayScroll>
    <div class="dialog-actions">
      <button type="button" :aria-disabled="loading || names.length > 32" @click="refresh">{{ loading ? '讀取中' : '重新讀取' }}</button>
      <button type="button" autofocus @click="dialog?.close()">關閉</button>
    </div>
  </dialog>
</template>

<style scoped>
.world-batch-dialog { width: min(56rem, calc(100vw - 2rem)); }
.preview-toolbar { display: flex; flex-wrap: wrap; justify-content: space-between; gap: .5rem 1rem; }
.preview-content { max-height: min(55vh, 31rem); margin-block-start: .75rem; }
.preview-instance + .preview-instance { margin-block-start: 1.25rem; }
.preview-instance h3 { margin-block: .5rem .25rem; }
.mark-instance { display: inline-flex; align-items: center; gap: .3rem; }
.preview-fields { list-style: none; padding: 0; }
.preview-fields li {
  display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) minmax(0, .8fr);
  gap: .5rem 1rem; padding-block: .65rem; border-block-start: 1px solid var(--border);
  overflow-wrap: anywhere;
}
.field-label {
  display: grid; grid-template-columns: 1rem minmax(0, 1fr);
  column-gap: .5rem; align-content: start;
}
.field-label > input { grid-column: 1; grid-row: 1; place-self: center; margin: 0; }
.field-label > :not(input) { grid-column: 2; }
.field-label > label { font-weight: 700; }
.preview-fields small { display: block; }
.value-pair { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: .5rem; margin: 0; }
.value-pair dd { margin: 0; white-space: pre-wrap; }
@media (max-width: 640px) { .preview-fields li { grid-template-columns: minmax(0, 1fr); } }
</style>
