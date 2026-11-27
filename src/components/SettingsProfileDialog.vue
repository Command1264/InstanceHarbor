<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage } from '../manager'
import { draftFrom, emptySettingsProfiles, fieldLabel, PROFILE_PROPERTIES, PROFILE_RULES, reasonText, valueCount, valuesFrom, valueText,
  type ProfileApplyResult, type ProfileDraft, type ProfilePreview, type SettingsProfiles } from '../settings-profiles'
import OperationButton from './OperationButton.vue'

// Settings profiles (#36): manage profiles, then apply one to chosen servers once.
// Applying always shows the per-server preview first and needs a confirmation.
type Step = 'list' | 'edit' | 'choose' | 'preview' | 'done'
const emit = defineEmits<{ applied: [] }>()
const { t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const step = ref<Step>('list')
const data = ref<SettingsProfiles>(emptySettingsProfiles)
const busy = ref(false)
const error = ref('')
const editing = ref<string | null>(null)
const draft = ref<ProfileDraft>(draftFrom())
const names = ref<string[]>([])
const group = ref<{ id: string; name: string } | null>(null)
const chosen = ref('')
const preview = ref<ProfilePreview>()
const result = ref<ProfileApplyResult>()
const deleting = ref<string | null>(null)

const remembered = computed(() => group.value ? data.value.groupProfiles[group.value.id] ?? '' : '')
const changeCount = computed(() => preview.value?.items.reduce((sum, item) => sum + item.changes.length, 0) ?? 0)
const title = computed(() => step.value === 'list' ? t('profiles.title')
  : step.value === 'edit' ? (editing.value ? t('profiles.editTitle') : t('profiles.createTitle'))
  : group.value ? t('profiles.applyGroupTitle', { name: group.value.name }) : t('profiles.applyTitle', { count: names.value.length }))

async function load() {
  data.value = await manage<SettingsProfiles>(['settings-profiles'])
}
async function run(work: () => Promise<void>) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try { await work() } catch (failure) { error.value = errorText(failure) } finally { busy.value = false }
}
function reset() {
  error.value = ''; preview.value = undefined; result.value = undefined; deleting.value = null
}
/** Opens the profile list, or the apply flow for `targets`. */
function open(options: { names?: string[]; group?: { id: string; name: string } } = {}) {
  reset()
  names.value = options.names ?? []
  group.value = options.group ?? null
  step.value = names.value.length ? 'choose' : 'list'
  dialog.value?.showModal()
  void run(async () => {
    await load()
    chosen.value = remembered.value || data.value.profiles[0]?.id || ''
  })
}
function edit(id: string | null) {
  editing.value = id
  draft.value = draftFrom(data.value.profiles.find(profile => profile.id === id))
  error.value = ''
  step.value = 'edit'
}
function save() {
  void run(async () => {
    let values
    try { values = valuesFrom(draft.value) } catch { throw new Error(t('profiles.invalidNumber')) }
    const edit = { op: 'save', name: draft.value.name, values, ...(editing.value ? { id: editing.value } : {}) }
    data.value = await manage<SettingsProfiles>(['edit-settings-profiles', JSON.stringify(edit)])
    step.value = names.value.length ? 'choose' : 'list'
  })
}
function remove(id: string) {
  void run(async () => {
    data.value = await manage<SettingsProfiles>(['edit-settings-profiles', JSON.stringify({ op: 'delete', id })])
    deleting.value = null
  })
}
function remember() {
  const target = group.value
  if (!target) return
  void run(async () => {
    data.value = await manage<SettingsProfiles>(['edit-settings-profiles', JSON.stringify({ op: 'link', group: target.id, id: chosen.value || null })])
  })
}
function showPreview() {
  if (!chosen.value) return
  void run(async () => {
    preview.value = await manage<ProfilePreview>(['settings-profile-preview', chosen.value, JSON.stringify(names.value)])
    step.value = 'preview'
  })
}
function apply() {
  const current = preview.value
  if (!current) return
  void run(async () => {
    result.value = await manage<ProfileApplyResult>(['settings-profile-apply', crypto.randomUUID(), current.profile.id, JSON.stringify(names.value), current.token])
    step.value = 'done'
    emit('applied')
  })
}
function close() { if (!busy.value) dialog.value?.close() }
const running = (state?: string) => !!state && state !== 'Stopped'
const stateText = (state: string) => t(state === 'Completed' ? 'activity.state.Completed' : state === 'Failed' ? 'activity.state.Failed' : 'activity.state.Skipped')
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="settings-profile-dialog" aria-labelledby="settings-profile-title" @cancel.prevent="close">
    <h2 id="settings-profile-title">{{ title }}</h2>

    <template v-if="step === 'list'">
      <p v-if="!data.profiles.length && !busy" class="profile-empty">{{ t('profiles.empty') }}</p>
      <ul v-else class="profile-list">
        <li v-for="profile in data.profiles" :key="profile.id">
          <span><strong>{{ profile.name }}</strong> <span class="profile-count">{{ t('profiles.count', { count: valueCount(profile.values) }) }}</span></span>
          <span class="actions">
            <template v-if="deleting === profile.id">
              <span>{{ t('profiles.deleteConfirm') }}</span>
              <button type="button" :disabled="busy" @click="deleting = null">{{ t('action.cancel') }}</button>
              <button type="button" class="danger" :disabled="busy" @click="remove(profile.id)">{{ t('profiles.delete') }}</button>
            </template>
            <template v-else>
              <button type="button" :disabled="busy" @click="edit(profile.id)">{{ t('profiles.edit') }}</button>
              <button type="button" class="danger" :disabled="busy" @click="deleting = profile.id">{{ t('profiles.delete') }}</button>
            </template>
          </span>
        </li>
      </ul>
    </template>

    <form v-else-if="step === 'edit'" class="profile-editor" @submit.prevent="save">
      <label class="profile-name">{{ t('profiles.name') }}<input v-model="draft.name" name="profile-name" maxlength="40" autocomplete="off" required></label>
      <fieldset>
        <legend><label><input v-model="draft.java.include" type="checkbox"> {{ t('profiles.field.java') }}</label></legend>
        <template v-if="draft.java.include">
          <label><input v-model="draft.java.automatic" type="radio" :value="true" name="profile-java"> {{ t('profiles.javaAutomatic') }}</label>
          <label><input v-model="draft.java.automatic" type="radio" :value="false" name="profile-java"> {{ t('profiles.javaPath') }}</label>
          <input v-if="!draft.java.automatic" v-model="draft.java.path" name="profile-java-path" :aria-label="t('profiles.javaPath')" placeholder="C:/…/bin/java.exe">
        </template>
      </fieldset>
      <fieldset>
        <legend><label><input v-model="draft.jvm.include" type="checkbox"> {{ t('profiles.field.jvm') }}</label></legend>
        <div v-if="draft.jvm.include" class="profile-grid">
          <label>{{ t('profiles.jvmMin') }}<input v-model="draft.jvm.minMib" inputmode="numeric" name="profile-jvm-min"></label>
          <label>{{ t('profiles.jvmMax') }}<input v-model="draft.jvm.maxMib" inputmode="numeric" name="profile-jvm-max"></label>
          <label>{{ t('profiles.jvmGc') }}<select v-model="draft.jvm.gc" name="profile-jvm-gc"><option value="">{{ t('profiles.jvmGcDefault') }}</option><option value="g1">G1</option><option value="parallel">Parallel</option><option value="serial">Serial</option></select></label>
          <label class="wide">{{ t('settingsPage.jvmFlags') }}<textarea v-model="draft.jvm.advanced" rows="2" name="profile-jvm-flags"></textarea></label>
        </div>
      </fieldset>
      <fieldset>
        <legend><label><input v-model="draft.timing.include" type="checkbox"> {{ t('profiles.field.timing') }}</label></legend>
        <div v-if="draft.timing.include" class="profile-grid">
          <label>{{ t('profiles.stopSeconds') }}<input v-model="draft.timing.stopSeconds" inputmode="numeric" name="profile-stop"></label>
          <label>{{ t('profiles.readySeconds') }}<input v-model="draft.timing.readySeconds" inputmode="numeric" name="profile-ready"></label>
          <label>{{ t('profiles.restartDelayMs') }}<input v-model="draft.timing.restartDelayMs" inputmode="numeric" name="profile-restart-delay"></label>
        </div>
      </fieldset>
      <fieldset>
        <legend><label><input v-model="draft.autoRestart.include" type="checkbox"> {{ t('profiles.field.autoRestart') }}</label></legend>
        <div v-if="draft.autoRestart.include" class="profile-grid">
          <label class="check"><input v-model="draft.autoRestart.onCrash" type="checkbox"> {{ t('profiles.onCrash') }}</label>
          <label class="check"><input v-model="draft.autoRestart.onUnexpectedExit" type="checkbox"> {{ t('profiles.onUnexpectedExit') }}</label>
          <label>{{ t('profiles.delaySeconds') }}<input v-model="draft.autoRestart.delaySeconds" inputmode="numeric" name="profile-ar-delay"></label>
          <label>{{ t('profiles.maxAttempts') }}<input v-model="draft.autoRestart.maxAttempts" inputmode="numeric" name="profile-ar-attempts"></label>
        </div>
      </fieldset>
      <fieldset v-for="section in [{ id: 'properties', rows: PROFILE_PROPERTIES }, { id: 'rules', rows: PROFILE_RULES }]" :key="section.id">
        <legend>{{ section.id === 'properties' ? t('profiles.properties') : t('profiles.rules') }}</legend>
        <p v-if="section.id === 'rules'" class="hint">{{ t('profiles.rulesHint') }}</p>
        <div v-for="row in section.rows" :key="row.key" class="profile-field">
          <label><input v-model="draft.fields[row.key]!.include" type="checkbox"> {{ fieldLabel(row.key) }}</label>
          <template v-if="draft.fields[row.key]!.include">
            <select v-if="row.field.kind === 'bool' || row.field.kind === 'choice'" v-model="draft.fields[row.key]!.value" :aria-label="fieldLabel(row.key)">
              <option v-for="choice in row.field.kind === 'bool' ? ['true', 'false'] : row.field.choices" :key="choice" :value="choice">{{ valueText(choice) }}</option>
            </select>
            <input v-else v-model="draft.fields[row.key]!.value" :aria-label="fieldLabel(row.key)" :inputmode="row.field.kind === 'int' ? 'numeric' : undefined" :maxlength="row.field.kind === 'text' ? row.field.max : undefined">
          </template>
        </div>
      </fieldset>
      <button type="submit" hidden></button>
    </form>

    <template v-else-if="step === 'choose'">
      <p v-if="!data.profiles.length && !busy" class="profile-empty">{{ t('profiles.emptyApply') }}</p>
      <label v-else class="profile-name">{{ t('profiles.choose') }}
        <select v-model="chosen" name="profile-choice">
          <option v-for="profile in data.profiles" :key="profile.id" :value="profile.id">{{ profile.name }}{{ profile.id === remembered ? ' · ' + t('profiles.rememberedMark') : '' }}</option>
        </select>
      </label>
      <p class="hint">{{ t('profiles.applyHint') }}</p>
      <div v-if="group" class="actions">
        <button type="button" :disabled="busy || !chosen || chosen === remembered" @click="remember">{{ t('profiles.remember') }}</button>
        <button v-if="remembered" type="button" :disabled="busy" @click="chosen = ''; remember()">{{ t('profiles.forget') }}</button>
      </div>
    </template>

    <template v-else-if="step === 'preview' && preview">
      <p>{{ t('profiles.previewSummary', { profile: preview.profile.name, count: changeCount }) }}</p>
      <div class="profile-preview">
        <section v-for="item in preview.items" :key="item.name">
          <h3>{{ item.name }}<span v-if="running(item.state)" class="profile-running"> · {{ t('profiles.runningNote') }}</span></h3>
          <p v-if="item.error" class="form-error">{{ errorText(item.error) }}</p>
          <p v-else-if="!item.changes.length" class="profile-note">{{ t('profiles.noChanges') }}</p>
          <ul v-if="item.changes.length">
            <li v-for="change in item.changes" :key="change.key">{{ t('profiles.changeLine', { field: fieldLabel(change.key), before: valueText(change.before), after: valueText(change.after) }) }}<span v-if="change.deferred" class="profile-note"> · {{ t('profiles.nextStart') }}</span></li>
          </ul>
          <ul v-if="item.skipped.length" class="profile-skipped">
            <li v-for="skip in item.skipped" :key="skip.key">{{ t('profiles.skipped', { field: fieldLabel(skip.key), reason: reasonText(skip.reason) }) }}</li>
          </ul>
        </section>
      </div>
    </template>

    <template v-else-if="step === 'done' && result">
      <p role="status">{{ t('profiles.doneSummary') }}</p>
      <ul class="profile-results">
        <li v-for="item in result.items" :key="item.name">
          <strong>{{ item.name }}</strong> · {{ stateText(item.state) }}
          <span v-if="item.applied.length"> · {{ t('profiles.appliedCount', { count: item.applied.length }) }}</span>
          <span v-if="item.deferred.length"> · {{ t('profiles.deferredCount', { count: item.deferred.length }) }}</span>
          <span v-if="item.skipped.length"> · {{ t('profiles.skippedCount', { count: item.skipped.length }) }}</span>
          <span v-if="item.error" class="form-error"> · {{ errorText(item.error) }}</span>
        </li>
      </ul>
    </template>

    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <div class="dialog-actions">
      <button v-if="step === 'edit'" type="button" :disabled="busy" @click="step = names.length ? 'choose' : 'list'">{{ t('profiles.back') }}</button>
      <button v-if="step === 'preview'" type="button" :disabled="busy" @click="step = 'choose'">{{ t('profiles.back') }}</button>
      <button v-if="step === 'list' || step === 'choose'" type="button" :disabled="busy" @click="edit(null)">{{ t('profiles.create') }}</button>
      <button type="button" :disabled="busy" @click="close">{{ t(step === 'done' || step === 'list' ? 'action.close' : 'action.cancel') }}</button>
      <OperationButton v-if="step === 'edit'" class="primary" :label="t('profiles.save')" :busy-label="t('profiles.saving')" :busy="busy" :disabled="!draft.name.trim()" @click="save" />
      <OperationButton v-if="step === 'choose'" class="primary" :label="t('profiles.preview')" :busy-label="t('profiles.previewing')" :busy="busy" :disabled="!chosen" @click="showPreview" />
      <OperationButton v-if="step === 'preview'" class="primary" :label="t('profiles.confirm', { count: preview?.items.length ?? 0 })" :busy-label="t('profiles.applying')" :busy="busy" :disabled="!changeCount" @click="apply" />
    </div>
  </dialog>
</template>

<style scoped>
.profile-list, .profile-results, .profile-preview ul { list-style: none; margin: .5rem 0; padding: 0; display: grid; gap: .4rem; }
.profile-list li { display: flex; justify-content: space-between; align-items: center; gap: .75rem; border-bottom: 1px solid var(--border); padding-block: .35rem; }
.profile-count, .profile-note, .profile-running { color: var(--muted); font-size: .8125rem; }
.profile-editor { display: grid; gap: .5rem; }
.profile-editor fieldset { border: 1px solid var(--border); border-radius: 6px; padding: .5rem .75rem; margin: 0; display: grid; gap: .4rem; }
.profile-editor legend label, .profile-field label, .check { display: inline-flex; gap: .4rem; align-items: center; font-weight: 500; }
.profile-name { display: grid; gap: .35rem; font-size: .875rem; }
.profile-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr)); gap: .5rem; }
.profile-grid label:not(.check) { display: grid; gap: .25rem; font-size: .8125rem; }
.profile-grid .wide { grid-column: 1 / -1; }
.profile-field { display: grid; grid-template-columns: minmax(12rem, 1fr) minmax(8rem, 1fr); gap: .5rem; align-items: center; font-size: .875rem; }
.profile-preview { display: grid; gap: .75rem; max-block-size: 50vh; overflow: auto; }
.profile-preview h3 { font-size: .9375rem; margin: 0 0 .25rem; }
.profile-preview li { font-size: .875rem; overflow-wrap: anywhere; }
.profile-skipped li { color: var(--muted); }
.profile-empty { color: var(--muted); }
</style>
