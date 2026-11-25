<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import InitialQueryProgress from './InitialQueryProgress.vue'
import OperationButton from './OperationButton.vue'
import { useAppLanguage } from '../i18n'
import { coreBuildText } from '../core-display'
import type { CatalogBuild, CatalogEntry, CatalogFamily } from '../core-catalog'

const family = defineModel<'' | CatalogFamily>('family', { required: true })
const version = defineModel<string>('version', { required: true })
const props = defineProps<{
  disabled: boolean
  versions?: CatalogEntry<string[]>
  build?: CatalogEntry<CatalogBuild | null>
}>()
const { t } = useAppLanguage()
const emit = defineEmits<{ retryVersions: []; retryBuild: [] }>()
const selectedUnavailable = computed(() => !!version.value && !!props.versions?.value
  && !props.versions.value.includes(version.value))
const coldVersions = computed(() => !!props.versions?.pending && props.versions.value === undefined)
const coldBuild = computed(() => !!props.build?.pending && props.build.value === undefined)
const visibleProgress = ref('')
watch(() => coldVersions.value ? 'versions' : coldBuild.value ? 'build' : '', (stage, _previous, cleanup) => {
  visibleProgress.value = ''
  if (!stage) return
  const timer = setTimeout(() => { visibleProgress.value = stage }, 180)
  cleanup(() => clearTimeout(timer))
}, { immediate: true })
function changeFamily(event: Event) {
  version.value = ''
  family.value = (event.target as HTMLSelectElement).value as '' | CatalogFamily
}
</script>

<template>
  <div class="core-picker">
    <label for="create-core-source">{{ t('components.createCore.source') }}</label>
    <select id="create-core-source" name="core-source" :value="family" :disabled="disabled" @change="changeFamily">
      <option value="">{{ t('components.createCore.localJar') }}</option>
      <option value="PAPER">{{ t('components.createCore.paper') }}</option>
      <option value="FOLIA">{{ t('components.createCore.folia') }}</option>
    </select>
    <template v-if="family">
      <label for="create-core-version">{{ t('components.createCore.minecraftVersion') }}</label>
      <select id="create-core-version" v-model="version" name="core-version" :aria-busy="coldVersions || undefined"
        required :disabled="disabled || !versions?.value?.length">
        <option value="">{{ t('components.createCore.chooseVersion') }}</option>
        <option v-if="selectedUnavailable" :value="version" disabled>{{ version }}</option>
        <option v-for="candidate in versions?.value" :key="candidate" :value="candidate">{{ candidate }}</option>
      </select>
      <InitialQueryProgress v-if="visibleProgress === 'versions'" id="catalog-version-progress" :label="t('components.createCore.loadingVersions')" />
      <p v-else-if="versions?.value?.length === 0" class="hint" role="status">{{ t('components.createCore.noVersions') }}</p>
      <p v-else-if="selectedUnavailable" class="form-error" role="status">{{ t('components.createCore.versionGone') }}</p>
      <div v-if="versions?.failed" class="catalog-error">
        <p class="form-error" role="status">{{ t('components.createCore.versionsFailed') }}</p>
        <OperationButton :label="t('components.createCore.retryVersions')" :busy-label="t('components.createCore.retryingVersions')" :busy="versions.manual" :disabled="disabled"
          @click="emit('retryVersions')" />
      </div>
      <template v-if="version && !selectedUnavailable">
        <InitialQueryProgress v-if="visibleProgress === 'build'" id="catalog-build-progress" :label="t('components.createCore.checkingBuild')" />
        <p v-else-if="build?.value" class="catalog-build">{{ t('components.createCore.latestBuild') }} <strong>{{ coreBuildText(build.value.build) }}</strong></p>
        <p v-else-if="build?.value === null" class="hint" role="status">{{ t('components.createCore.noStableBuild') }}</p>
        <div v-if="build?.failed" class="catalog-error">
          <p class="form-error" role="status">{{ t('components.createCore.buildFailed') }}</p>
          <OperationButton :label="t('components.createCore.retryBuild')" :busy-label="t('components.createCore.retryingBuild')" :busy="build.manual" :disabled="disabled"
            @click="emit('retryBuild')" />
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.core-picker { display: grid; gap: .5rem; margin-block: 1rem; font-size: .875rem; }
.core-picker label { display: block; }
.catalog-build, .catalog-error p { margin: 0; }
.catalog-build strong { margin-inline-start: .25rem; }
</style>
