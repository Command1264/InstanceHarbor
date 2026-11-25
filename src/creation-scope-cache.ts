/**
 * Previews of one creation confirmation, keyed by the selected management folder option.
 * `reset` starts a new confirmation; replies for an older confirmation are dropped.
 */
export class ScopeCache<T> {
  private entries = new Map<string, T>()
  private current = 0

  get generation() { return this.current }

  reset(): number {
    this.entries.clear()
    return ++this.current
  }

  get(key: string): T | undefined {
    return this.entries.get(key)
  }

  /** Stores a reply only while its confirmation is still the current one. */
  set(generation: number, key: string, value: T): boolean {
    if (generation !== this.current) return false
    this.entries.set(key, value)
    return true
  }
}
