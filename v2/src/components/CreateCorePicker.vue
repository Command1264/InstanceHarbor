<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import InitialQueryProgress from './InitialQueryProgress.vue'
import OperationButton from './OperationButton.vue'
import type { CatalogBuild, CatalogEntry, CatalogFamily } from '../core-catalog'

const family = defineModel<'' | CatalogFamily>('family', { required: true })
const version = defineModel<string>('version', { required: true })
const props = defineProps<{
  disabled: boolean
  versions?: CatalogEntry<string[]>
  build?: CatalogEntry<CatalogBuild | null>
}>()
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
    <label for="create-core-source">核心來源</label>
    <select id="create-core-source" name="core-source" :value="family" :disabled="disabled" @change="changeFamily">
      <option value="">本機 JAR</option>
      <option value="PAPER">Paper 官方下載</option>
      <option value="FOLIA">Folia 官方下載</option>
    </select>
    <template v-if="family">
      <label for="create-core-version">Minecraft 版本</label>
      <select id="create-core-version" v-model="version" name="core-version" :aria-busy="coldVersions || undefined"
        required :disabled="disabled || !versions?.value?.length">
        <option value="">選擇版本</option>
        <option v-if="selectedUnavailable" :value="version" disabled>{{ version }}</option>
        <option v-for="candidate in versions?.value" :key="candidate" :value="candidate">{{ candidate }}</option>
      </select>
      <InitialQueryProgress v-if="visibleProgress === 'versions'" id="catalog-version-progress" label="正在讀取版本清單" />
      <p v-else-if="versions?.value?.length === 0" class="hint" role="status">目前沒有可選版本。</p>
      <p v-else-if="selectedUnavailable" class="form-error" role="status">這個版本已不在官方清單中，請另選版本。</p>
      <div v-if="versions?.failed" class="catalog-error">
        <p class="form-error" role="status">無法讀取官方版本清單。</p>
        <OperationButton label="重試讀取版本" busy-label="正在讀取版本" :busy="versions.manual" :disabled="disabled"
          @click="emit('retryVersions')" />
      </div>
      <template v-if="version && !selectedUnavailable">
        <InitialQueryProgress v-if="visibleProgress === 'build'" id="catalog-build-progress" label="正在核對穩定建置" />
        <p v-else-if="build?.value" class="catalog-build">最新穩定建置 <strong>Build {{ build.value.build }}</strong></p>
        <p v-else-if="build?.value === null" class="hint" role="status">這個版本沒有可下載的穩定建置，請另選版本。</p>
        <div v-if="build?.failed" class="catalog-error">
          <p class="form-error" role="status">無法核對官方建置。</p>
          <OperationButton label="重試核對建置" busy-label="正在核對建置" :busy="build.manual" :disabled="disabled"
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
