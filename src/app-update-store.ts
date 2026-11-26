// One update status shared by the rail hint, the settings tab and the About section (#65).
import { invoke } from '@tauri-apps/api/core'
import { ref, shallowRef } from 'vue'
import { errorText, manage, nativeBridge, rawError } from './manager.ts'
import type { AppUpdateStatus, UpdateChannel } from './app-update.ts'

export const updateStatus = shallowRef<AppUpdateStatus>()
/** Only a check the user started shows progress; automatic checks stay quiet. */
export const updateChecking = ref(false)
/** The result of the last manual check or open, for its status line. */
export const updateFeedback = ref<'' | 'done' | 'failed'>('')
export const updateActionError = ref('')
/** The download, install or open the user started, while it runs. */
export const updateBusy = ref<'' | 'download' | 'install' | 'reveal'>('')
/** Set by the rail hint so the settings page opens on the update tab. */
export const updateTabRequested = ref(false)

let sequence = 0

/**
 * A check or a settings save writes `app-update.json` and replies with the state it wrote,
 * so its reply always applies. A plain read applies only if nothing else replied since it
 * started, so a stale read never replaces a newer result.
 */
async function request(arguments_: string[], origin: 'manual' | 'background', writes: boolean) {
  const id = ++sequence
  const value = await manage<AppUpdateStatus>(arguments_, origin)
  if (writes) showUpdateStatus(value)
  else if (id === sequence) updateStatus.value = value
  return value
}

export function showUpdateStatus(value: AppUpdateStatus) {
  sequence++
  updateStatus.value = value
}

export async function readUpdateStatus(origin: 'manual' | 'background' = 'background') {
  return request(['app-update-status'], origin, false)
}

/** At GUI start: checks only when due; failures are kept in the status, not shown as dialogs. */
export async function startupUpdateCheck() {
  if (!nativeBridge) return
  try {
    // Downloads of versions not newer than this build are no longer needed.
    await manage(['app-update-cleanup'], 'background')
  } catch (error) {
    console.warn('Update download cleanup failed', errorText(error))
  }
  try {
    const status = await readUpdateStatus()
    if (status.due) await request(['app-update-check', 'auto'], 'background', true)
  } catch (error) {
    console.warn('Update check at start failed', errorText(error))
  }
}

export async function checkForUpdateNow() {
  if (updateChecking.value) return
  updateChecking.value = true
  updateFeedback.value = ''
  updateActionError.value = ''
  try {
    const status = await request(['app-update-check', 'manual'], 'manual', true)
    updateFeedback.value = status.lastError ? 'failed' : 'done'
  } catch (error) {
    updateFeedback.value = 'failed'
    updateActionError.value = errorText(error)
  } finally {
    updateChecking.value = false
  }
}

export async function saveUpdateSettings(channel: UpdateChannel, autoCheck: boolean) {
  return request(['app-update-settings', channel, String(autoCheck)], 'manual', true)
}

/** Runs one action the user started; its result shows in the update row. */
async function act(kind: 'download' | 'install' | 'reveal', arguments_: string[]) {
  if (updateBusy.value) return false
  updateBusy.value = kind
  updateFeedback.value = ''
  updateActionError.value = ''
  try {
    const value = await manage<unknown>(arguments_, 'manual')
    if (kind === 'download') showUpdateStatus(value as AppUpdateStatus)
    return true
  } catch (error) {
    updateActionError.value = rawError(error)
    // A failed check may have deleted the file; show what is there now.
    readUpdateStatus().catch(failure => console.warn('Could not read the update status', failure))
    return false
  } finally {
    updateBusy.value = ''
  }
}

export const downloadUpdate = (tag: string) => act('download', ['app-update-download', tag])
/** Starts the verified Setup; it closes InstanceHarbor itself. */
export const installUpdate = (tag: string) => act('install', ['app-update-install', tag])
export const revealUpdate = (tag: string) => act('reveal', ['app-update-reveal', tag])

export async function openUpdateRelease(tag: string) {
  updateActionError.value = ''
  try {
    await invoke('open_update_release', { tag })
  } catch (error) {
    updateActionError.value = errorText(error)
  }
}
