// Helpers for restoring an instance that was removed from management.

/** A request ID the backend `adopt` receipt accepts: lowercase letters, digits and hyphens, at most 64 characters. */
export function restoreRequestId(now = Date.now(), random = Math.random()): string {
  return `restore-${now.toString(36)}-${Math.floor(random * 36 ** 6).toString(36).padStart(6, '0')}`
}

/** Java executables offered for a restore: the saved setting first, then the discovered ones, without duplicates. */
export function restoreJavaChoices(saved: string | null, discovered: string[]): string[] {
  const key = (path: string) => path.replaceAll('\\', '/').toLowerCase()
  const seen = new Set<string>()
  const result: string[] = []
  for (const path of [saved, ...discovered]) {
    if (!path || seen.has(key(path))) continue
    seen.add(key(path))
    result.push(path)
  }
  return result
}
