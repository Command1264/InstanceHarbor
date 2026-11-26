<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage } from '../manager'
import { groupOf, type ServerGroupEdit, type ServerGroups } from '../server-groups'
import OperationButton from './OperationButton.vue'

// Creating, renaming and deleting server groups and moving servers between them (#37).
// Every change is one `edit-server-groups` call; the reply is the saved state.
type Mode = 'create' | 'rename' | 'delete' | 'assign'
const NEW_GROUP = '\u0000new'
const props = defineProps<{ data: ServerGroups }>()
const emit = defineEmits<{ saved: [data: ServerGroups] }>()
const { t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const nameInput = ref<HTMLInputElement>()
const mode = ref<Mode>('create')
const groupId = ref<string | null>(null)
const names = ref<string[]>([])
const name = ref('')
const target = ref<string>('')
const busy = ref(false)
const error = ref('')

const group = computed(() => props.data.groups.find(item => item.id === groupId.value))
const memberCount = computed(() => Object.keys(props.data.members).filter(member => groupOf(props.data, member) === groupId.value).length)
const naming = computed(() => mode.value === 'create' || mode.value === 'rename' || (mode.value === 'assign' && target.value === NEW_GROUP))
const title = computed(() => mode.value === 'create' ? t('instances.group.createTitle')
  : mode.value === 'rename' ? t('instances.group.renameTitle', { name: group.value?.name ?? '' })
  : mode.value === 'delete' ? t('instances.group.deleteTitle', { name: group.value?.name ?? '' })
  : names.value.length === 1 ? t('instances.group.assignTitleOne', { name: names.value[0] ?? '' }) : t('instances.group.assignTitle', { count: names.value.length }))
const ready = computed(() => !busy.value && (!naming.value || !!name.value.trim()) && (mode.value === 'create' || mode.value === 'assign' || !!group.value))

function open(next: Mode, options: { id?: string | null; names?: string[] } = {}) {
  mode.value = next
  groupId.value = options.id ?? null
  names.value = options.names ?? []
  name.value = next === 'rename' ? group.value?.name ?? '' : ''
  const current = names.value.length ? names.value.map(item => groupOf(props.data, item)) : []
  // Preselect the shared current group, or the first other choice.
  target.value = current.length && current.every(id => id === current[0]) ? current[0] ?? '' : ''
  error.value = ''
  dialog.value?.showModal()
  void nextTick(() => { if (naming.value) nameInput.value?.focus() })
}
async function send(edit: ServerGroupEdit): Promise<ServerGroups & { created?: string | null }> {
  const saved = await manage<ServerGroups & { created?: string | null }>(['edit-server-groups', JSON.stringify(edit)])
  emit('saved', saved)
  return saved
}
async function confirm() {
  if (!ready.value) return
  busy.value = true
  error.value = ''
  try {
    if (mode.value === 'create') await send({ op: 'create', name: name.value })
    else if (mode.value === 'rename' && groupId.value) await send({ op: 'rename', id: groupId.value, name: name.value })
    else if (mode.value === 'delete' && groupId.value) await send({ op: 'delete', id: groupId.value })
    else if (mode.value === 'assign') {
      let id: string | null = target.value || null
      if (target.value === NEW_GROUP) {
        id = (await send({ op: 'create', name: name.value })).created ?? null
        if (!id) throw new Error('ServerGroupNotFound')
      }
      await send({ op: 'assign', id, names: names.value })
    }
    dialog.value?.close()
  } catch (failure) { error.value = t('instances.group.error', { error: errorText(failure) }) }
  finally { busy.value = false }
}
function close() { if (!busy.value) dialog.value?.close() }
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="compact server-group-dialog" aria-labelledby="server-group-title" @cancel.prevent="close">
    <h2 id="server-group-title">{{ title }}</h2>
    <form @submit.prevent="confirm">
      <label v-if="mode === 'assign'">{{ t('instances.group.target') }}
        <select v-model="target" name="server-group-target">
          <option value="">{{ t('instances.group.ungrouped') }}</option>
          <option v-for="item in data.groups" :key="item.id" :value="item.id">{{ item.name }}</option>
          <option :value="NEW_GROUP">{{ t('instances.group.newGroupOption') }}</option>
        </select>
      </label>
      <label v-if="naming">{{ t('instances.group.name') }}
        <input ref="nameInput" v-model="name" name="server-group-name" maxlength="40" autocomplete="off" required>
      </label>
      <p v-if="mode === 'delete'">{{ t('instances.group.deleteHint', { count: memberCount }) }}</p>
      <p v-if="mode === 'assign'" class="hint">{{ t('instances.group.assignHint') }}</p>
      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div class="dialog-actions">
        <button type="button" :disabled="busy" @click="close">{{ t('action.cancel') }}</button>
        <OperationButton type="submit" :class="mode === 'delete' ? 'danger' : 'primary'" :label="mode === 'delete' ? t('instances.group.delete') : mode === 'assign' ? t('instances.group.move') : t('instances.group.save')"
          :busy-label="t('instances.group.saving')" :busy="busy" :disabled="!ready" />
      </div>
    </form>
  </dialog>
</template>
