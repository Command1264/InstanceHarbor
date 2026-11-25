// Cold-start marks for the GUI diagnostics log (see src-tauri/src/startup_timing.rs).
// Each mark is a wall-clock time in Unix milliseconds; the native side makes it
// relative to process creation and records one `gui-startup` event.
import { invoke } from '@tauri-apps/api/core'

export type StartupMark =
  | 'page-navigation-start'
  | 'page-script-start'
  | 'page-mounted'
  | 'page-preferences-read'
  | 'page-window-presented'
  | 'page-first-list-read'
  | 'page-first-list-rendered'
  | 'page-first-list-painted'

const marks: Partial<Record<StartupMark, number>> = {}
let reported = false
let paintWaitOver = false
/** A hidden window may not paint; report without the paint mark after this wait. */
const PAINT_WAIT_MS = 2000

function now(): number {
  return performance.timeOrigin + performance.now()
}

export function startupMark(name: StartupMark, at: number = now()): void {
  if (marks[name] === undefined) marks[name] = at
}

export function hasStartupMark(name: StartupMark): boolean {
  return marks[name] !== undefined
}

function reportWhenComplete(): void {
  if (reported || !hasStartupMark('page-window-presented') || !hasStartupMark('page-first-list-rendered')) return
  if (!hasStartupMark('page-first-list-painted') && !paintWaitOver) return
  reported = true
  void invoke('report_startup_timing', { marks }).catch(error => console.error('Could not record the startup timing', error))
}

/** The window is on screen. */
export function startupWindowPresented(): void {
  startupMark('page-window-presented')
  reportWhenComplete()
}

/** The first instance list reply was applied; call after the DOM update that shows it. */
export function startupListRendered(): void {
  if (hasStartupMark('page-first-list-rendered')) return
  startupMark('page-first-list-rendered')
  requestAnimationFrame(() => {
    startupMark('page-first-list-painted')
    reportWhenComplete()
  })
  setTimeout(() => {
    paintWaitOver = true
    reportWhenComplete()
  }, PAINT_WAIT_MS)
  reportWhenComplete()
}
