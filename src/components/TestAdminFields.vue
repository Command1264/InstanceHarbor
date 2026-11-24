<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage } from '../manager'
import { clearedLuckPerms, LOOKUP_DEBOUNCE_MS, MAX_TEST_ADMINISTRATORS, shouldLookup, type PermissionKind } from '../test-admins'
import OperationButton from './OperationButton.vue'

export type TestAdministrator = { name: string; uuid: string }
export type LuckPermsArtifact = { platform: 'bukkit' | 'bukkit-java8' | 'velocity'; fileName: string; version: string; sha256: string; state: 'Ready' | 'Missing' | 'Mismatch' }

const props = defineProps<{ artifacts?: LuckPermsArtifact[]; busy: boolean; fetching: boolean; connected: boolean }>()
const rows = defineModel<TestAdministrator[]>('administrators', { required: true })
const luckperms = defineModel<boolean>('luckperms', { required: true })
const op = defineModel<boolean>('op', { required: true })
const kind = defineModel<PermissionKind>('kind', { required: true })
const nodes = defineModel<string>('nodes', { required: true })
const group = defineModel<string>('group', { required: true })
const emit = defineEmits<{ fetch: [] }>()
const { t } = useAppLanguage()

const platformNames = {
  bukkit: 'components.testAdmins.platformBukkit',
  'bukkit-java8': 'components.testAdmins.platformJava8',
  velocity: 'components.testAdmins.platformVelocity',
} as const
const stateNames = {
  Ready: 'components.testAdmins.stateReady',
  Missing: 'components.testAdmins.stateMissing',
  Mismatch: 'components.testAdmins.stateMismatch',
} as const
const allReady = computed(() => !!props.artifacts?.length && props.artifacts.every(row => row.state === 'Ready'))
const luckpermsOff = computed(() => props.busy || !luckperms.value)
// The nodes and group stay when LuckPerms is turned off; only this button removes them.
const hasLuckPermsContent = computed(() => kind.value !== 'all' || nodes.value !== '' || group.value !== '')
function clearLuckPerms() {
  const cleared = clearedLuckPerms()
  kind.value = cleared.kind
  nodes.value = cleared.nodes
  group.value = cleared.group
}

// Rows whose UUID came from a lookup may be looked up again; a hand-typed or saved UUID never is.
const autoFilled = new WeakMap<TestAdministrator, string>()
const timers = new Map<TestAdministrator, ReturnType<typeof setTimeout>>()
const looking = ref<TestAdministrator>()
const lookupError = ref('')

function add() {
  if (rows.value.length < MAX_TEST_ADMINISTRATORS) rows.value = [...rows.value, { name: '', uuid: '' }]
}
function remove(index: number) {
  const row = rows.value[index]
  clearTimeout(timers.get(row))
  timers.delete(row)
  if (looking.value === row) looking.value = undefined
  rows.value = rows.value.filter((_, position) => position !== index)
}
function nameChanged(row: TestAdministrator) {
  clearTimeout(timers.get(row))
  lookupError.value = ''
  // A UUID filled in for another name must not stay beside a different name.
  const filled = autoFilled.get(row)
  if (filled !== undefined && filled.toLowerCase() !== row.name.trim().toLowerCase()) {
    row.uuid = ''
    autoFilled.delete(row)
  }
  if (!shouldLookup(row, autoFilled.has(row))) return
  timers.set(row, setTimeout(() => void lookup(row), LOOKUP_DEBOUNCE_MS))
}
function uuidEdited(row: TestAdministrator) {
  autoFilled.delete(row)
}
async function lookup(row: TestAdministrator) {
  timers.delete(row)
  const requested = row.name.trim()
  if (!props.connected || !rows.value.includes(row) || !shouldLookup(row, autoFilled.has(row))) return
  looking.value = row
  lookupError.value = ''
  try {
    const found = await manage<{ name: string; uuid: string }>(['test-admin-lookup', requested], 'background')
    // A newer edit wins: apply the answer only to the name and UUID state it was asked for.
    if (!rows.value.includes(row) || row.name.trim() !== requested || !shouldLookup(row, autoFilled.has(row))) return
    row.name = found.name
    row.uuid = found.uuid
    autoFilled.set(row, found.name)
  } catch (error) {
    if (rows.value.includes(row) && row.name.trim() === requested) {
      lookupError.value = t('components.testAdmins.lookupFailed', { row: rows.value.indexOf(row) + 1, reason: errorText(error) })
    }
  } finally {
    if (looking.value === row) looking.value = undefined
  }
}
onBeforeUnmount(() => timers.forEach(timer => clearTimeout(timer)))
</script>

<template>
  <section class="test-admins" aria-labelledby="test-admins-identities">
    <h3 id="test-admins-identities" class="sr-only">{{ t('components.testAdmins.identities') }}</h3>
    <table class="identities">
      <thead><tr><th scope="col">{{ t('components.testAdmins.name') }}</th><th scope="col">UUID</th><th scope="col"><span class="sr-only">{{ t('components.testAdmins.actions') }}</span></th></tr></thead>
      <tbody>
        <tr v-for="(row, index) in rows" :key="index">
          <td><input v-model.trim="row.name" :name="`test-admin-name-${index}`" :aria-label="t('components.testAdmins.nameRow', { row: index + 1 })" autocomplete="off" spellcheck="false" maxlength="16" :disabled="busy" @change="nameChanged(row)"></td>
          <td><input v-model.trim="row.uuid" class="mono" :name="`test-admin-uuid-${index}`" :aria-label="t('components.testAdmins.uuidRow', { row: index + 1 })" :aria-busy="looking === row" :placeholder="looking === row ? t('components.testAdmins.lookingUp') : ''" autocomplete="off" spellcheck="false" maxlength="36" :disabled="busy" @input="uuidEdited(row)"></td>
          <td><button type="button" :disabled="busy" @click="remove(index)">{{ t('components.testAdmins.remove') }}</button></td>
        </tr>
        <tr v-if="!rows.length"><td colspan="3" class="empty-row">{{ t('components.testAdmins.none') }}</td></tr>
      </tbody>
    </table>
    <p v-if="lookupError" class="form-error" role="alert">{{ lookupError }}</p>
    <div class="actions"><button type="button" :disabled="busy || rows.length >= MAX_TEST_ADMINISTRATORS" @click="add">{{ t('components.testAdmins.add') }}</button></div>
    <div class="toggles">
      <label class="check"><input v-model="luckperms" type="checkbox" name="test-admin-luckperms" :disabled="busy"> {{ t('components.testAdmins.luckperms') }}</label>
      <label class="check"><input v-model="op" type="checkbox" name="test-admin-op" :disabled="busy"> {{ t('components.testAdmins.op') }}</label>
    </div>
    <div class="field">
      <label for="test-admin-kind">{{ t('components.testAdmins.grant') }}</label>
      <div class="editor"><select id="test-admin-kind" v-model="kind" :disabled="luckpermsOff"><option value="all">{{ t('components.testAdmins.grantAll') }}</option><option value="custom">{{ t('components.testAdmins.grantCustom') }}</option></select></div>
    </div>
    <div class="field">
      <label for="test-admin-nodes">{{ t('components.testAdmins.nodes') }}</label>
      <div class="editor"><textarea id="test-admin-nodes" v-model="nodes" name="test-admin-nodes" rows="4" autocomplete="off" spellcheck="false" :placeholder="t('components.testAdmins.nodesPlaceholder')" :disabled="luckpermsOff || kind !== 'custom'"></textarea></div>
    </div>
    <div class="field">
      <label for="test-admin-group">{{ t('components.testAdmins.group') }}</label>
      <div class="editor"><input id="test-admin-group" v-model.trim="group" name="test-admin-group" autocomplete="off" spellcheck="false" maxlength="36" :disabled="luckpermsOff"></div>
    </div>
    <div class="actions"><button type="button" :disabled="busy || !hasLuckPermsContent" @click="clearLuckPerms">{{ t('components.testAdmins.clearLuckPerms') }}</button></div>
  </section>
  <section class="test-admins" aria-labelledby="test-admins-luckperms">
    <h3 id="test-admins-luckperms">LuckPerms</h3>
    <table>
      <thead><tr><th scope="col">{{ t('components.testAdmins.platform') }}</th><th scope="col">{{ t('components.testAdmins.file') }}</th><th scope="col">{{ t('components.testAdmins.state') }}</th></tr></thead>
      <tbody>
        <tr v-for="row in artifacts ?? []" :key="row.platform">
          <td>{{ t(platformNames[row.platform]) }}</td>
          <td class="mono" translate="no">{{ row.fileName }}</td>
          <td :class="{ 'form-error': row.state === 'Mismatch' }">{{ t(stateNames[row.state]) }}</td>
        </tr>
        <tr v-if="!artifacts"><td>--</td><td>--</td><td>--</td></tr>
      </tbody>
    </table>
    <div class="actions">
      <OperationButton :label="t('components.testAdmins.fetch')" :busy-label="t('components.testAdmins.fetching')" :busy="fetching" :disabled="busy || !connected || !artifacts || allReady" @click="emit('fetch')" />
    </div>
    <p class="hint">{{ t('components.testAdmins.existingHelp') }}</p>
  </section>
</template>

<style scoped>
.test-admins { margin-block-start: 1rem; }
.test-admins h3 { margin-block: 0 .5rem; }
.test-admins td input { width: 100%; }
.identities td:last-child { width: 1%; white-space: nowrap; }
.test-admins .actions { margin-block: .5rem; }
.empty-row { color: var(--muted); }
.toggles { display: flex; flex-wrap: wrap; gap: .25rem 1.5rem; margin-block: .5rem; }
</style>
