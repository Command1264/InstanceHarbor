import { translate, type Locale } from './i18n.ts'

/** `diagnostic_status` reply. `folder` is shown as `%LOCALAPPDATA%/…`, never the user profile path. */
export interface DiagnosticStatus { available: boolean; error?: string | null; folder?: string }

// The 1.0 folder, for an older native side that does not report its folder.
const fallbackFolder = '%LOCALAPPDATA%/InstanceHarbor/logs'

/** The warning for a log that could not be written, naming the folder of this channel and workspace. */
export function diagnosticWarning(locale: Locale, status: DiagnosticStatus): string {
  const folder = status.folder || fallbackFolder
  if (status.error === 'DiagnosticDirectoryBudgetReached') return translate(locale, 'app.diagnosticBudgetWarning', { folder })
  return translate(locale, 'app.diagnosticWarning', { folder, error: status.error || 'DiagnosticWriteFailed' })
}
