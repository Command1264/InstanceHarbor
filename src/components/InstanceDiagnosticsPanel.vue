<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { formatDisplayTime, useAppLanguage, type MessageId, type TimeFormat } from '../i18n'
import { manage, type Instance } from '../manager'
import OperationButton from './OperationButton.vue'

interface Signal {
  state: string
  source?: string | null
  reason?: string | null
}

interface Diagnostics {
  instance: string
  checkedAt: number
  configuration: Signal & { proxy?: string | null; route?: string | null }
  runtime: Signal & { proxyRun?: string | null; backendRun?: string | null; forwarding?: string | null }
  plugin: Signal & { name?: string | null }
  player: Signal
}

const props = defineProps<{
  instance: string
  rows: Instance[]
  proxies: string[]
  connected: boolean
  timeFormat?: TimeFormat
}>()
const { locale, t } = useAppLanguage()
const proxy = ref('')
const route = ref('')
const plugin = ref('')
const pending = ref(false)
const failure = ref<MessageId>()
const result = ref<Diagnostics>()
let revision = 0

const isProxy = computed(() => props.proxies.includes(props.instance))
const proxyOptions = computed(() => props.proxies.filter(name => name !== props.instance))
const target = computed(() => JSON.stringify([props.instance, proxy.value, route.value, plugin.value]))
const evidence = computed(() => {
  const proxies = proxy.value ? [proxy.value] : props.proxies
  return JSON.stringify(props.rows
    .filter(row => row.name === props.instance || (!isProxy.value && proxies.includes(row.name)))
    .map(row => [row.name, row.run?.id ?? null, row.state])
    .sort((a, b) => String(a[0]).localeCompare(String(b[0]))))
})

watch(proxyOptions, names => {
  if (proxy.value && !names.includes(proxy.value)) proxy.value = ''
})
watch(target, () => {
  revision++
  pending.value = false
  failure.value = undefined
  result.value = undefined
}, { flush: 'sync' })
watch(evidence, () => {
  if (!pending.value) return
  revision++
  pending.value = false
  failure.value = 'diagnostics.runChanged'
}, { flush: 'sync' })
onBeforeUnmount(() => { revision++ })

async function refresh() {
  if (pending.value || !props.connected) return
  if (route.value && !/^[a-z0-9][a-z0-9._-]{0,63}$/i.test(route.value)) {
    failure.value = 'diagnostics.invalidRoute'
    return
  }
  if (plugin.value && (!/^[a-z0-9][a-z0-9 .+_-]*\.jar$/i.test(plugin.value) ||
      plugin.value.includes('..') || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])\./i.test(plugin.value))) {
    failure.value = 'diagnostics.invalidJar'
    return
  }
  const instance = props.instance
  const options = { proxy: proxy.value || undefined, route: route.value || undefined, plugin: plugin.value || undefined }
  const args = ['instance-diagnostics', instance]
  if (options.proxy || options.route || options.plugin) args.push(JSON.stringify(options))
  const request = ++revision
  pending.value = true
  failure.value = undefined
  try {
    const report = await manage<Diagnostics>(args)
    if (request !== revision) return
    if (report.instance !== instance) {
      failure.value = 'diagnostics.readError'
      return
    }
    result.value = report
  } catch {
    if (request === revision) failure.value = 'diagnostics.readError'
  } finally {
    if (request === revision) pending.value = false
  }
}

const stateLabels: Record<string, MessageId> = {
  Match: 'diagnostics.match',
  Mismatch: 'diagnostics.mismatch',
  NotConfigured: 'diagnostics.notConfigured',
  NotApplicable: 'diagnostics.notApplicable',
  Ready: 'diagnostics.ready',
  Stopped: 'diagnostics.stopped',
  Starting: 'diagnostics.starting',
  Present: 'diagnostics.present',
  Enabling: 'diagnostics.enabling',
  Enabled: 'diagnostics.enabled',
  Failed: 'diagnostics.failed',
  Disabled: 'diagnostics.disabled',
  Observed: 'diagnostics.observed',
  Unverified: 'diagnostics.unverified',
}
function stateText(state: string) {
  return t(Object.hasOwn(stateLabels, state) ? stateLabels[state] : 'diagnostics.unconfirmed')
}

const sourceLabels: Record<string, MessageId> = {
  VelocityConfig: 'diagnostics.velocityConfig',
  ManagedRun: 'diagnostics.managedRun',
  CurrentRunLog: 'diagnostics.currentRunLog',
  PluginJar: 'diagnostics.pluginJar',
  None: 'diagnostics.noSource',
}
function sourceText(source?: string | null) {
  return t(source && Object.hasOwn(sourceLabels, source) ? sourceLabels[source] : 'diagnostics.noSource')
}

const reasonLabels: Record<string, MessageId> = {
  SelectBackend: 'diagnostics.reason.selectBackend',
  SelectProxy: 'diagnostics.reason.selectProxy',
  ProxyUnavailable: 'diagnostics.reason.proxyUnavailable',
  NoProxy: 'diagnostics.reason.noProxy',
  BackendEndpointUnconfirmed: 'diagnostics.reason.backendEndpointUnconfirmed',
  BackendBindingChanged: 'diagnostics.reason.backendBindingChanged',
  VelocityConfigUnreadable: 'diagnostics.reason.velocityConfigUnreadable',
  ProxyBindingUnconfirmed: 'diagnostics.reason.proxyBindingUnconfirmed',
  ProxyBindingChanged: 'diagnostics.reason.proxyBindingChanged',
  RouteMissing: 'diagnostics.reason.routeMissing',
  RouteMatches: 'diagnostics.reason.routeMatches',
  RouteEndpointMismatch: 'diagnostics.reason.routeEndpointMismatch',
  RouteEndpointUnsupported: 'diagnostics.reason.routeEndpointUnsupported',
  SelectRoute: 'diagnostics.reason.selectRoute',
  NoCurrentPluginRun: 'diagnostics.reason.noCurrentPluginRun',
  PluginIdentityChanged: 'diagnostics.reason.pluginIdentityChanged',
  RunLogUnconfirmed: 'diagnostics.reason.runLogUnconfirmed',
  PluginLifecycleObserved: 'diagnostics.reason.pluginLifecycleObserved',
  PluginEventUnconfirmed: 'diagnostics.reason.pluginEventUnconfirmed',
  PluginNotApplicable: 'diagnostics.reason.pluginNotApplicable',
  SelectPlugin: 'diagnostics.reason.selectPlugin',
  PluginDescriptorUnconfirmed: 'diagnostics.reason.pluginDescriptorUnconfirmed',
  PluginJarUnavailable: 'diagnostics.reason.pluginJarUnavailable',
  EvidenceChanged: 'diagnostics.reason.evidenceChanged',
  BothRunsReady: 'diagnostics.reason.bothRunsReady',
  RunUnconfirmed: 'diagnostics.reason.runUnconfirmed',
  RunStarting: 'diagnostics.reason.runStarting',
  RunStopped: 'diagnostics.reason.runStopped',
}
function reasonText(reason?: string | null) {
  return reason && Object.hasOwn(reasonLabels, reason) ? t(reasonLabels[reason]) : ''
}

const signalLabels = {
  configuration: 'diagnostics.configuration',
  runtime: 'diagnostics.runtime',
  plugin: 'diagnostics.plugin',
  player: 'diagnostics.player',
} as const
const signals = computed(() => {
  const report = result.value
  if (!report) return []
  return (Object.keys(signalLabels) as (keyof typeof signalLabels)[]).map(key => ({
    ...report[key],
    key,
    label: signalLabels[key],
    state: key === 'player' ? 'Unverified' : report[key].state,
  }))
})
const checkedAt = computed(() => {
  const value = result.value?.checkedAt
  return value === undefined || !Number.isFinite(value) ? '' : formatDisplayTime(value, locale.value, props.timeFormat)
})
</script>

<template>
  <section class="instance-diagnostics" aria-labelledby="instance-diagnostics-title">
    <h3 id="instance-diagnostics-title">{{ t('diagnostics.title') }}</h3>
    <form @submit.prevent="refresh">
      <label v-if="!isProxy && proxyOptions.length">
        {{ t('diagnostics.proxy') }}
        <select v-model="proxy">
          <option value="">{{ t('diagnostics.automatic') }}</option>
          <option v-for="name in proxyOptions" :key="name" :value="name">{{ name }}</option>
        </select>
      </label>
      <label v-if="!isProxy && proxyOptions.length">
        {{ t('diagnostics.route') }}
        <input v-model.trim="route" type="text" maxlength="64" spellcheck="false" autocomplete="off"
          :aria-invalid="failure === 'diagnostics.invalidRoute' || undefined"
          :aria-describedby="failure === 'diagnostics.invalidRoute' ? 'diagnostic-reply-error' : undefined">
      </label>
      <label v-if="!isProxy">
        {{ t('diagnostics.jar') }}
        <input v-model.trim="plugin" type="text" maxlength="160" spellcheck="false" autocomplete="off"
          :aria-invalid="failure === 'diagnostics.invalidJar' || undefined"
          :aria-describedby="failure === 'diagnostics.invalidJar' ? 'diagnostic-plugin-help diagnostic-reply-error' : 'diagnostic-plugin-help'">
        <small id="diagnostic-plugin-help" class="hint">{{ t('diagnostics.jarHelp') }}</small>
      </label>
      <OperationButton type="submit" :label="t('diagnostics.refresh')" :busy-label="t('action.checking')"
        :busy="pending" :disabled="!connected" />
    </form>
    <p v-if="failure" id="diagnostic-reply-error" class="form-error" role="alert">{{ t(failure) }}</p>
    <p v-if="checkedAt" class="hint" role="status">{{ t('diagnostics.checkedAt', { time: checkedAt }) }}</p>
    <ul v-if="result" class="diagnostic-results" :aria-label="t('diagnostics.results')">
      <li v-for="signal in signals" :key="signal.key">
        <h4>{{ t(signal.label) }}</h4>
        <p><span class="badge">{{ stateText(signal.state) }}</span></p>
        <p class="hint">{{ t('diagnostics.source', { source: sourceText(signal.source) }) }}</p>
        <p v-if="signal.key === 'player'" class="hint">{{ t('diagnostics.playerDetail') }}</p>
        <p v-else-if="reasonText(signal.reason)" class="hint">{{ reasonText(signal.reason) }}</p>
        <template v-if="signal.key === 'configuration'">
          <p v-if="result.configuration.proxy" class="hint">{{ t('diagnostics.proxyName', { name: result.configuration.proxy }) }}</p>
          <p v-if="result.configuration.route" class="hint">{{ t('diagnostics.routeName', { name: result.configuration.route }) }}</p>
        </template>
        <template v-else-if="signal.key === 'runtime'">
          <p v-if="result.runtime.proxyRun" class="hint selectable">{{ t('diagnostics.proxyRun', { id: result.runtime.proxyRun }) }}</p>
          <p v-if="result.runtime.backendRun" class="hint selectable">{{ t('diagnostics.backendRun', { id: result.runtime.backendRun }) }}</p>
          <p v-if="result.runtime.forwarding && signal.state !== 'NotApplicable'" class="hint">{{ t('diagnostics.forwarding', { state: stateText(result.runtime.forwarding) }) }}</p>
        </template>
        <p v-else-if="signal.key === 'plugin' && result.plugin.name" class="hint">{{ result.plugin.name }}</p>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.instance-diagnostics { margin-top: 16px; border-top: 1px solid var(--border); padding-top: 12px; }
h3, h4 { margin: 0; font-size: .875rem; font-weight: 600; }
form { display: grid; gap: 8px; justify-items: start; margin-top: 8px; }
label { display: grid; gap: 4px; width: 100%; font-size: .8125rem; }
.diagnostic-results { margin: 8px 0 0; padding: 0; list-style: none; }
.diagnostic-results li { border-top: 1px solid var(--border); padding: 8px 0; overflow-wrap: anywhere; }
.diagnostic-results p { margin: 4px 0 0; }
</style>
