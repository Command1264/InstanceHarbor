import type { MessageId, tr } from './i18n.ts'

// What `app-update-status` and `app-update-check` report (src-tauri/src/app_update.rs, #65).
export type UpdateChannel = 'stable' | 'prerelease'

export interface UpdateAsset {
  name: string
  url: string
  size: number
  digest: string | null
}

export interface AvailableUpdate {
  version: string
  /** The release tag; the page sends only this to open the release page. */
  tag: string
  prerelease: boolean
  publishedAt: string | null
  asset: UpdateAsset | null
  problem: 'NoMatchingAsset' | 'UntrustedAsset' | null
}

export interface AppUpdateStatus {
  currentVersion: string
  channel: UpdateChannel
  autoCheck: boolean
  /** For example `windows-x64`. */
  platform: string
  install: 'setup' | 'portable'
  /** Unix seconds. */
  lastAttempt: number | null
  lastSuccess: number | null
  lastError: string | null
  /** An automatic check is allowed now (on, and none in the last 24 hours). */
  due: boolean
  update: AvailableUpdate | null
}

type Translate = typeof tr

const reasons = new Set(['UpdateCheckRateLimited', 'UpdateCheckNotFound', 'UpdateCheckHttpError', 'UpdateCheckTimeout',
  'UpdateCheckOffline', 'UpdateCheckInvalidResponse', 'UpdateCheckUnavailable'])

/** Why the last check failed; unknown codes fall back to a general sentence. */
export function updateFailureText(code: string, t: Translate) {
  return reasons.has(code) ? t(`components.update.reason.${code}` as MessageId) : t('components.update.reason.UpdateCheckFailed')
}

/**
 * The "latest version" line. A failed check never claims the build is current:
 * without a successful result since the failure, the line stays `--`.
 */
export function updateResultText(status: AppUpdateStatus | undefined, t: Translate) {
  if (!status) return '--'
  const update = status.update
  if (update) return t(update.prerelease ? 'components.update.availablePrerelease' : 'components.update.available', { version: update.version })
  if (status.lastSuccess === null || status.lastError) return '--'
  return t('components.update.current')
}

/** Why the file of an available update cannot be used, or '' when it can. */
export function updateAssetText(status: AppUpdateStatus | undefined, t: Translate) {
  const problem = status?.update?.problem
  if (problem === 'NoMatchingAsset') return t('components.update.noAsset', { platform: status!.platform })
  if (problem === 'UntrustedAsset') return t('components.update.untrustedAsset')
  return ''
}
