export type InstanceNameIssue = '' | 'required' | 'tooLong' | 'start' | 'characters' | 'trailingPeriod' | 'reserved'

const reserved = /^(?:con|prn|aux|nul|(?:com|lpt)[1-9])$/

/** Mirrors `instance_name` in `windows.rs`: inner periods are allowed, a trailing one is not. */
export function instanceNameIssue(name: string): InstanceNameIssue {
  if (!name) return 'required'
  if (name.length > 64) return 'tooLong'
  if (!/^[a-z0-9]/.test(name)) return 'start'
  if (!/^[a-z0-9.-]+$/.test(name)) return 'characters'
  if (name.endsWith('.')) return 'trailingPeriod'
  // Windows opens the device for a reserved name even when an extension follows, such as `con.1`.
  if (reserved.test(name.split('.')[0])) return 'reserved'
  return ''
}

export function isInstanceName(name: unknown): name is string {
  return typeof name === 'string' && instanceNameIssue(name) === ''
}
