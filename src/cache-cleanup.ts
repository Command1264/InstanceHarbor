import type { MessageId } from './i18n'

export type CacheCategory = 'download' | 'legacyWebView' | 'webView' | 'sharedCore'
export type CacheState = 'ready' | 'inUse' | 'scheduled' | 'unavailable'
export type CacheItem = { category: CacheCategory; path: string; bytes: number; complete: boolean; state: CacheState; reason?: string }
export type CachePreview = { items: CacheItem[]; revision: string }
export type CacheResult = { category: CacheCategory; path: string; bytes: number; outcome: 'deleted' | 'scheduled' | 'failed'; reason?: string }

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB']

/** Binary units, one decimal from KiB up, for example `1.5 MiB`. */
export function formatBytes(bytes: number, locale?: string) {
  let value = Math.max(0, bytes)
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit++
  }
  const digits = unit === 0 ? 0 : 1
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value)} ${UNITS[unit]}`
}

const REASONS: Record<string, MessageId> = {
  FileInUse: 'components.cache.reason.fileInUse',
  AccessDenied: 'components.cache.reason.accessDenied',
  InUse: 'components.cache.reason.inUse',
  CacheEntryChanged: 'components.cache.reason.changed',
  WebViewCacheCustomFolder: 'components.cache.reason.customFolder',
}

/** The message for a reason the cache commands report, or undefined for an unknown reason. */
export function cacheReasonId(reason: string): MessageId | undefined {
  return Object.hasOwn(REASONS, reason) ? REASONS[reason] : undefined
}
