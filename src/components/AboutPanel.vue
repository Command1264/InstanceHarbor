<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { onMounted, ref } from 'vue'
import { useAppLanguage } from '../i18n'
import { errorText, nativeBridge } from '../manager'
import { aboutCommitText, type AboutInfo, type AboutLink } from '../about'
import type { TimeFormat } from '../i18n'
import AppUpdatePanel from './AppUpdatePanel.vue'

defineProps<{ timeFormat?: TimeFormat }>()

const { t } = useAppLanguage()
const info = ref<AboutInfo>()
const readError = ref('')
const openError = ref('')

onMounted(async () => {
  if (!nativeBridge) return
  try {
    info.value = await invoke<AboutInfo>('about_info')
  } catch (error) {
    readError.value = t('components.about.readFailed', { reason: errorText(error) })
  }
})

async function open(link: AboutLink) {
  openError.value = ''
  try {
    await invoke('open_about_link', { link })
  } catch (error) {
    openError.value = t('components.about.openFailed', { reason: errorText(error) })
  }
}
</script>

<template>
  <section aria-labelledby="about-heading">
    <h2 id="about-heading">{{ t('components.about.heading') }}</h2>
    <div class="field"><span class="about-label">{{ t('components.about.version') }}</span><span class="about-value">{{ info?.version ?? '--' }}</span></div>
    <AppUpdatePanel :time-format="timeFormat" :show-version="false" />
    <div class="field"><span class="about-label">{{ t('components.about.commit') }}</span><span class="about-value mono">{{ info ? aboutCommitText(info.commit) : '--' }}</span></div>
    <div v-if="info?.workspace" class="field"><span class="about-label">{{ t('components.about.workspace') }}</span><span class="about-value mono">{{ info.workspace }}</span></div>
    <div class="field"><span class="about-label">{{ t('components.about.author') }}</span><span class="about-value">{{ info?.author ?? '--' }}</span></div>
    <div class="field"><span class="about-label">{{ t('components.about.links') }}</span><div class="actions">
      <button type="button" :disabled="!info" @click="open('repository')">{{ t('components.about.repository') }}</button>
      <button type="button" :disabled="!info" @click="open('issues')">{{ t('components.about.issues') }}</button>
    </div></div>
    <div class="field"><span class="about-label">{{ t('components.about.license') }}</span><div class="editor">
      <span>MIT</span>
      <div class="actions">
        <button type="button" :disabled="!info?.license" @click="open('license')">{{ t('components.about.licenseFile') }}</button>
        <button type="button" :disabled="!info?.notices" @click="open('notices')">{{ t('components.about.notices') }}</button>
      </div>
      <small v-if="info && (!info.license || !info.notices)" class="hint">{{ t('components.about.noticesMissing') }}</small>
    </div></div>
    <p v-if="readError || openError" role="alert" class="form-error">{{ readError || openError }}</p>
  </section>
</template>

<style scoped>
.about-label { font-size: .875rem; font-weight: 500; }
.about-value { overflow-wrap: anywhere; }
.field:last-of-type { border-bottom: 0; }
</style>
