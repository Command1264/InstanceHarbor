export type ManagementChoice = { kind: 'none' } | { kind: 'folder'; path: string }

/** Value of the "no management folder" option in the creation confirmation. */
export const NO_MANAGEMENT_FOLDER = ''

/**
 * Turns the selected management folder into the draft field. The backend lists the
 * target folder first and its parent second; the parent is the default and needs no field.
 */
export function managementChoice(selected: string, options: readonly string[]): ManagementChoice | undefined {
  if (selected === NO_MANAGEMENT_FOLDER) return { kind: 'none' }
  if (selected === options[1]) return undefined
  return { kind: 'folder', path: selected }
}
