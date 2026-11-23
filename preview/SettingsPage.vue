<script setup lang="ts">
import { settingsPresentation } from './settings-presentation'

defineProps<{
  instance?: string
  tabs: { key: string; label: string }[]
  tab: string
  title: string
  subtitle: string
  heading: string
  templates: string
  fields: string
  fieldRevision: number
}>()
</script>

<template>
  <div class="settings-scroll">
    <header class="page-head"><div><h1>{{ title }}</h1><p>{{ subtitle }}</p></div><div class="actions"></div></header>
    <nav v-if="instance" class="context-breadcrumb" aria-label="目前位置">
      <a href="#instances" data-nav="instances">實例</a><span aria-hidden="true">／</span>
      <span>{{ instance }}</span><span aria-hidden="true">／</span><span>設定</span>
    </nav>
    <div class="settings-layout">
      <nav class="settings-nav" aria-label="設定分類">
        <button v-for="item in tabs" :key="item.key" type="button" :data-tab="item.key"
                :aria-current="tab === item.key ? 'page' : undefined">{{ item.label }}</button>
      </nav>
      <section class="sheet">
        <h2>{{ heading }}</h2>
        <div v-if="templates" class="preview-fragment" v-html="templates"></div>
        <form id="settings-form" novalidate>
          <div id="settings-content" :key="fieldRevision" v-html="fields"></div>
          <div id="form-error" class="form-error" role="alert">{{ settingsPresentation.error }}</div>
        </form>
      </section>
    </div>
  </div>
  <div class="settings-footer" aria-label="設定操作">
    <span id="setting-state" class="setting-message" role="status">{{ settingsPresentation.message }}</span>
    <div class="actions">
      <button type="button" data-action="reset-draft">取消</button>
      <button type="submit" class="primary" form="settings-form">{{ tab === 'world' ? '套用' : '儲存設定' }}</button>
    </div>
  </div>
</template>
