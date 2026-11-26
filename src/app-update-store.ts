// One update status shared by the rail hint, the settings tab and the About section (#65).
import { invoke } from '@tauri-apps/api/core'
import { ref, shallowRef } from 'vue'
import { errorText, manage, nativeBridge } from './manager.ts'
import type { AppUpdateStatus, UpdateChannel } from './app-update.ts'

export const updateStatus = shallowRef<AppUpdateStatus>()
/** Only a check the user started shows progress; automatic checks stay quiet. */
export const updateChecking = ref(false)
/** The result of the last manual check or open, for its status line. */
export const updateFeedback = ref<'' | 'done' | 'failed'>('')
export const updateActionError = ref('')
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

export async function openUpdateRelease(tag: string) {
  updateActionError.value = ''
  try {
    await invoke('open_update_release', { tag })
  } catch (error) {
    updateActionError.value = errorText(error)
  }
}
