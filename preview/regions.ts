import { h, render, type VNode } from 'vue'
import InstanceDetail from './InstanceDetail.vue'
import JavaPicker from './JavaPicker.vue'
import CoreMaintenance from './CoreMaintenance.vue'
import SettingsPage from './SettingsPage.vue'
import ActivityPage from './ActivityPage.vue'
import InstanceRows, { type InstanceRow } from './InstanceRows.vue'
import InstancePage, { type InstancePageState } from './InstancePage.vue'
import DialogContent from './DialogContent.vue'

export function renderInstanceRows(
  target: Element, rows: InstanceRow[], selected: Set<string>, anchor: string | null,
) {
  render(h(InstanceRows, { rows, selected, anchor }), target)
}

export function renderDialogContent(
  target: HTMLDialogElement, titleId: string, title: string, body: string, actions: string,
) {
  render(h(DialogContent, { titleId, title, body, actions }), target)
}

export function renderInstanceDetail(target: Element, props: InstanceType<typeof InstanceDetail>['$props']) {
  render(h(InstanceDetail, props), target)
}

const pages = new WeakMap<Element, string>()

function renderPage(target: Element, name: string, node: VNode) {
  if (pages.get(target) !== name) clearPreviewPage(target)
  render(node, target)
  pages.set(target, name)
}

export function renderInstancePage(target: Element, state: InstancePageState, statusNames: Record<string, string>) {
  renderPage(target, 'instances', h(InstancePage, {
    state: {
      query: state.query, filter: state.filter, connected: state.connected,
      scenario: state.scenario, coreQueryLabel: state.coreQueryLabel,
      sort: state.sort, direction: state.direction,
    },
    statusNames,
  }))
}

export function renderSettingsPage(target: Element, props: InstanceType<typeof SettingsPage>['$props']) {
  renderPage(target, 'settings', h(SettingsPage, props))
}

export function renderActivityPage(target: Element, props: InstanceType<typeof ActivityPage>['$props']) {
  renderPage(target, 'activity', h(ActivityPage, props))
}

export function clearPreviewPage(target: Element) {
  const rows = target.querySelector('#instance-rows')
  if (rows) render(null, rows)
  const detail = target.querySelector('#detail')
  if (detail) render(null, detail)
  render(null, target)
  target.replaceChildren()
  pages.delete(target)
}

export function renderJavaPicker(target: Element, props: InstanceType<typeof JavaPicker>['$props']) {
  render(h(JavaPicker, props), target)
}

export function renderCoreMaintenance(target: Element, props: InstanceType<typeof CoreMaintenance>['$props']) {
  render(h(CoreMaintenance, props), target)
}
