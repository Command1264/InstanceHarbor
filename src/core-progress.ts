import type { MessageId } from './i18n.ts'

/** A step of a core update or restore, as `CoreUpdatePhase` reports it (#60). */
export type CoreUpdateProgress =
  | { phase: 'checking' | 'reusing' | 'verifying' | 'backingUp' | 'replacing' }
  | { phase: 'downloading'; received: number; total: number | null }

const steps: Record<CoreUpdateProgress['phase'], MessageId> = {
  checking: 'core.progress.checking',
  downloading: 'core.progress.downloading',
  reusing: 'core.progress.reusing',
  verifying: 'core.progress.verifying',
  backingUp: 'core.progress.backingUp',
  replacing: 'core.progress.replacing',
}

/** The message of the reported step, or nothing for an unknown step. */
export function coreProgressStep(progress?: CoreUpdateProgress | null): MessageId | undefined {
  return progress && Object.hasOwn(steps, progress.phase) ? steps[progress.phase] : undefined
}

/** The `<progress>` value and maximum. Only a download with an official size has them. */
export function coreProgressValue(progress?: CoreUpdateProgress | null): { value: number; max: number } | undefined {
  if (progress?.phase !== 'downloading' || !progress.total || progress.total <= 0) return undefined
  return { value: Math.min(progress.received, progress.total), max: progress.total }
}

const MEGABYTE = 1024 * 1024

/**
 * The downloaded size as a message and its values, for example `23.4 / 52.7 MB`.
 * Without an official size only the downloaded amount is shown, never a percentage.
 */
export function coreProgressBytes(progress: CoreUpdateProgress | null | undefined, locale: string):
  { id: MessageId; values: Record<string, string> } | undefined {
  if (progress?.phase !== 'downloading') return undefined
  const format = new Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 })
  const megabytes = (bytes: number) => format.format(Math.floor(bytes / MEGABYTE * 10) / 10)
  const received = megabytes(progress.received)
  return progress.total && progress.total > 0
    ? { id: 'core.progress.bytes', values: { received, total: megabytes(progress.total) } }
    : { id: 'core.progress.bytesUnknownTotal', values: { received } }
}
