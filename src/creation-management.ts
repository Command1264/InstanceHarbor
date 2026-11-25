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

/** The option a draft field selects; the inverse of `managementChoice`. */
export function selectedManagementFolder(choice: ManagementChoice | undefined, options: readonly string[]): string {
  if (!choice) return options[1] ?? NO_MANAGEMENT_FOLDER
  return choice.kind === 'none' ? NO_MANAGEMENT_FOLDER : choice.path
}

/**
 * The choice to show after a preview for a newly selected option fails: the one the
 * preview still on screen was made for, so the selection matches what "Create" sends.
 */
export function previewedManagementChoice(previewed: { managementFolder?: ManagementChoice } | undefined): ManagementChoice | undefined {
  return previewed?.managementFolder
}
