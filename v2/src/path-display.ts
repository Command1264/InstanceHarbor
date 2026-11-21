export type PathSeparator = 'forward' | 'native'

// This is a presentation boundary. Unix permits literal backslashes in names.
export function displayPath(path: string, separator: PathSeparator = 'forward', platform = typeof navigator !== 'undefined' && /Win/i.test(navigator.platform) ? 'windows' : 'unix'): string {
  if (platform !== 'windows') return path
  if (/^\\\\[?.]\\(?![a-z]:\\|UNC\\)/i.test(path)) return path
  let value = path.replace(/^\\\\\?\\UNC\\/i, '//').replace(/^\\\\\?\\(?=[a-z]:\\)/i, '')
  value = value.replaceAll('\\', '/')
  return separator === 'native' ? value.replaceAll('/', '\\') : value
}
