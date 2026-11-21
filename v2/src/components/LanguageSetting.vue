<script setup lang="ts">
import { useAppLanguage, type LanguagePreference } from '../i18n'

const language = defineModel<LanguagePreference>({ required: true })
const { t } = useAppLanguage()
</script>

<template>
  <div class="field">
    <label for="app-language">{{ t('appearance.language') }}</label>
    <div class="editor">
      <select id="app-language" v-model="language" name="language" aria-describedby="app-language-help">
        <option value="zh-TW" lang="zh-TW">繁體中文</option>
        <option value="en" lang="en">English</option>
        <option value="system">{{ t('appearance.systemLanguage') }}</option>
      </select>
      <small id="app-language-help" class="hint">{{ t('appearance.languageHelp') }}</small>
    </div>
  </div>
</template>
