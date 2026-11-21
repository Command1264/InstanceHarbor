import { invoke } from '@tauri-apps/api/core'

export function pickManagementFolder(current: string): Promise<string | null> {
  return invoke<string | null>('pick_management_folder', { current })
}

export type FilePickerKind = 'jar' | 'java' | 'json' | 'any' | 'recovery' | 'save-recovery'

export function pickFile(current: string, kind: FilePickerKind): Promise<string | null> {
  return invoke<string | null>('pick_file', { current, kind })
}

export type ManagementPathKind = 'allow' | 'exclude'

export function managementPathError(raw: string) {
  const match = /\bInvalidManagementPath (allow|exclude) (\d+) ([A-Za-z]+)\b/.exec(raw)
  if (!match) return undefined
  return { kind: match[1] as ManagementPathKind, index: Number(match[2]), reason: match[3] }
}
