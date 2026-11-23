import type { MessageId } from './i18n'

// Start preflight codes for a registered Java that is no longer usable.
// Both send the user to Instance settings → "Java and JVM" to choose another one.
export function javaStartErrorId(raw: string): MessageId | undefined {
  if (/\bJavaExecutableMissing\b/.test(raw)) return 'manager.error.javaExecutableMissing'
  if (/\bJavaExecutableUnreadable\b/.test(raw)) return 'manager.error.javaExecutableUnreadable'
  return undefined
}
