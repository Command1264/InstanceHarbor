<script setup lang="ts">
import { computed } from 'vue'
import { useAppLanguage } from '../i18n'
import OperationButton from './OperationButton.vue'

export type TestAdministrator = { name: string; uuid: string }
export type LuckPermsArtifact = { platform: 'bukkit' | 'bukkit-java8' | 'velocity'; fileName: string; version: string; sha256: string; state: 'Ready' | 'Missing' | 'Mismatch' }
const MAX_TEST_ADMINISTRATORS = 10

const props = defineProps<{ artifacts?: LuckPermsArtifact[]; busy: boolean; fetching: boolean; connected: boolean }>()
const rows = defineModel<TestAdministrator[]>('administrators', { required: true })
const mode = defineModel<'all' | 'group'>('mode', { required: true })
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

function add() {
  if (rows.value.length < MAX_TEST_ADMINISTRATORS) rows.value = [...rows.value, { name: '', uuid: '' }]
}
function remove(index: number) {
  rows.value = rows.value.filter((_, position) => position !== index)
}
</script>

<template>
  <section class="test-admins" aria-labelledby="test-admins-identities">
    <h3 id="test-admins-identities" class="sr-only">{{ t('components.testAdmins.identities') }}</h3>
    <table class="identities">
      <thead><tr><th scope="col">{{ t('components.testAdmins.name') }}</th><th scope="col">UUID</th><th scope="col"><span class="sr-only">{{ t('components.testAdmins.actions') }}</span></th></tr></thead>
      <tbody>
        <tr v-for="(row, index) in rows" :key="index">
          <td><input v-model.trim="row.name" :name="`test-admin-name-${index}`" :aria-label="t('components.testAdmins.nameRow', { row: index + 1 })" autocomplete="off" spellcheck="false" maxlength="16" :disabled="busy"></td>
          <td><input v-model.trim="row.uuid" class="mono" :name="`test-admin-uuid-${index}`" :aria-label="t('components.testAdmins.uuidRow', { row: index + 1 })" autocomplete="off" spellcheck="false" maxlength="36" :disabled="busy"></td>
          <td><button type="button" :disabled="busy" @click="remove(index)">{{ t('components.testAdmins.remove') }}</button></td>
        </tr>
        <tr v-if="!rows.length"><td colspan="3" class="empty-row">{{ t('components.testAdmins.none') }}</td></tr>
      </tbody>
    </table>
    <div class="actions"><button type="button" :disabled="busy || rows.length >= MAX_TEST_ADMINISTRATORS" @click="add">{{ t('components.testAdmins.add') }}</button></div>
    <div class="field">
      <label for="test-admin-mode">{{ t('components.testAdmins.mode') }}</label>
      <div class="editor"><select id="test-admin-mode" v-model="mode" :disabled="busy"><option value="all">{{ t('components.testAdmins.modeAll') }}</option><option value="group">{{ t('components.testAdmins.modeGroup') }}</option></select></div>
    </div>
    <div class="field">
      <label for="test-admin-group">{{ t('components.testAdmins.group') }}</label>
      <div class="editor"><input id="test-admin-group" v-model.trim="group" name="test-admin-group" autocomplete="off" spellcheck="false" maxlength="36" :disabled="busy || mode !== 'group'"></div>
    </div>
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
</style>
