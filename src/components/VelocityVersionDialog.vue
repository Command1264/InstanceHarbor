<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, manage, manageCoreWithProgress } from '../manager'
import { coreVersionText } from '../core-display'
import type { CoreUpdateProgress } from '../core-progress'
import CoreUpdateProgressBar from './CoreUpdateProgressBar.vue'
import OperationButton from './OperationButton.vue'

// Moving a Velocity proxy to another Velocity version (#41). The first step
// chooses the version; the second shows "current -> target" and the risks and
// needs an explicit acknowledgement before the change starts.
type Profile = { family: string; minecraft: string; build: string; sha256: string }
const props = defineProps<{ name: string }>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useAppLanguage()
const dialog = ref<HTMLDialogElement>()
const step = ref<'choose' | 'summary' | 'running' | 'done'>('choose')
const loading = ref(false)
const error = ref('')
const current = ref<Profile>()
const versions = ref<{ version: string; snapshot: boolean }[]>([])
const chosen = ref('')
const target = ref<{ target: Profile; channel: string }>()
const acknowledged = ref(false)
const progress = ref<CoreUpdateProgress>()
const result = ref('')
const busy = computed(() => loading.value || step.value === 'running')

async function open() {
  step.value = 'choose'
  error.value = ''
  result.value = ''
  chosen.value = ''
  target.value = undefined
  acknowledged.value = false
  versions.value = []
  dialog.value?.showModal()
  loading.value = true
  try {
    const reply = await manage<{ current: Profile; versions: { version: string; snapshot: boolean }[] }>(['core-velocity-versions', props.name])
    current.value = reply.current
    versions.value = reply.versions
  } catch (failure) { error.value = errorText(failure) }
  finally { loading.value = false }
}
async function next() {
  if (!chosen.value || busy.value) return
  loading.value = true
  error.value = ''
  try {
    const reply = await manage<{ current: Profile; target: Profile; channel: string }>(['core-velocity-target', props.name, chosen.value])
    current.value = reply.current
    target.value = { target: reply.target, channel: reply.channel }
    acknowledged.value = false
    step.value = 'summary'
  } catch (failure) { error.value = errorText(failure) }
  finally { loading.value = false }
}
async function apply() {
  const before = current.value
  const after = target.value?.target
  if (!before || !after || !acknowledged.value || busy.value) return
  step.value = 'running'
  error.value = ''
  progress.value = undefined
  try {
    const outcome = await manageCoreWithProgress<{ outcome: string }>(
      ['core-install-version', props.name, crypto.randomUUID(), before.sha256, after.minecraft, after.sha256],
      update => { progress.value = update })
    result.value = outcome.outcome === 'applied' ? t('components.coreMaintenance.verified') : t('components.coreMaintenance.notReplaced')
    emit('changed')
  } catch (failure) { error.value = errorText(failure) }
  finally { step.value = 'done' }
}
function close() { if (!busy.value) dialog.value?.close() }
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" aria-labelledby="velocity-version-title" @cancel.prevent="close">
    <h2 id="velocity-version-title">{{ t('components.velocityVersion.title', { name }) }}</h2>
    <template v-if="step === 'choose'">
      <p>{{ t('components.velocityVersion.chooseHint') }}</p>
      <p v-if="current">{{ t('components.velocityVersion.current', { version: coreVersionText(current) }) }}</p>
      <p v-if="loading" role="status">{{ t('components.velocityVersion.loading') }}</p>
      <p v-else-if="!error && !versions.length" role="status">{{ t('components.velocityVersion.none') }}</p>
      <fieldset v-if="versions.length" class="velocity-versions">
        <legend>{{ t('components.velocityVersion.target') }}</legend>
        <label v-for="row in versions" :key="row.version">
          <input v-model="chosen" type="radio" name="velocity-version" :value="row.version">
          <bdi translate="no">{{ row.version }}</bdi>
          <span v-if="row.snapshot" class="hint">{{ t('components.velocityVersion.snapshot') }}</span>
        </label>
      </fieldset>
    </template>
    <template v-else-if="target && current">
      <dl class="import-preview">
        <dt>{{ t('components.velocityVersion.change') }}</dt>
        <dd><bdi translate="no">{{ coreVersionText(current) }}</bdi> → <bdi translate="no">{{ coreVersionText(target.target) }}</bdi></dd>
        <dt>{{ t('components.velocityVersion.channel') }}</dt>
        <dd>{{ target.channel === 'STABLE' ? t('components.velocityVersion.stable') : t('components.velocityVersion.beta') }}</dd>
      </dl>
      <ul class="velocity-risks">
        <li>{{ t('components.velocityVersion.riskPlugins') }}</li>
        <li>{{ t('components.velocityVersion.riskPluginData') }}</li>
        <li>{{ t('components.velocityVersion.backupFiles') }}</li>
        <li>{{ t('components.velocityVersion.stopped') }}</li>
      </ul>
      <label v-if="step === 'summary'" class="velocity-acknowledge">
        <input v-model="acknowledged" type="checkbox"> {{ t('components.velocityVersion.acknowledge') }}
      </label>
      <CoreUpdateProgressBar v-if="step === 'running'" :progress="progress" fallback="components.coreMaintenance.progressLabel" />
      <p v-if="result" role="status">{{ result }}</p>
    </template>
    <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    <div class="dialog-actions">
      <button v-if="step === 'summary'" type="button" @click="step = 'choose'">{{ t('components.velocityVersion.back') }}</button>
      <button type="button" autofocus :disabled="busy" @click="close">{{ t(step === 'done' ? 'action.close' : 'action.cancel') }}</button>
      <OperationButton v-if="step === 'choose'" class="primary" :label="t('components.velocityVersion.next')" :busy-label="t('components.velocityVersion.loading')"
        :busy="loading && versions.length > 0" :disabled="!chosen || loading" @click="next" />
      <button v-if="step === 'summary'" type="button" class="primary" :disabled="!acknowledged" @click="apply">{{ t('components.velocityVersion.confirm') }}</button>
    </div>
  </dialog>
</template>

<style scoped>
.velocity-versions { display: grid; gap: .25rem; border: 0; padding: 0; margin: .5rem 0; max-block-size: 14rem; overflow: auto; }
.velocity-versions label, .velocity-acknowledge { display: flex; gap: .5rem; align-items: center; }
.velocity-risks { margin: .75rem 0; padding-inline-start: 1.25rem; }
.velocity-acknowledge { margin-block: .75rem; font-weight: 600; }
</style>
