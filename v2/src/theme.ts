import { invoke } from '@tauri-apps/api/core'

export type ThemePreference = 'system' | 'light' | 'dark'
export type SelectionColorPreference = 'system' | 'product' | 'blue' | 'red' | 'green' | 'magenta' | 'yellow' | 'cyan' | 'purple'
export const defaultSelectionOpacity = 25

export function resolveSelectionOpacity(value?: number): number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100
    ? value : defaultSelectionOpacity
}

const selectionColors = {
  blue: '#2457c5', red: '#bd3547', green: '#168052', magenta: '#ad278e',
  yellow: '#e8c547', cyan: '#35bfc9', purple: '#7750be',
} as const

export function resolveSelectionColor(preference?: SelectionColorPreference): SelectionColorPreference {
  // Old saved "product" preferences resolve to the blue choice.
  return preference === 'product' ? 'blue' : preference ?? 'blue'
}

function rgb(hex: string): [number, number, number] | undefined {
  const match = /^#([\da-f]{3}|[\da-f]{6})$/i.exec(hex)
  if (!match) return undefined
  const digits = match[1]
  const number = Number.parseInt(digits.length === 3 ? digits.replace(/./g, '$&$&') : digits, 16)
  return [(number >> 16) & 255, (number >> 8) & 255, number & 255]
}

export function relativeLuminance(hex: string): number | undefined {
  const channels = rgb(hex)
  if (!channels) return undefined
  const [red, green, blue] = channels.map(value => {
    const normalized = value / 255
    return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/** Pick the higher contrast of black and white for an opaque sRGB background. */
export function readableTextColor(background: string): '#000000' | '#ffffff' {
  const luminance = relativeLuminance(background)
  return luminance === undefined || luminance > Math.sqrt(0.0525) - 0.05 ? '#000000' : '#ffffff'
}

function contrast(first: string, second: string): number {
  const a = relativeLuminance(first)
  const b = relativeLuminance(second)
  return a === undefined || b === undefined ? 0 : (Math.max(a,b) + .05) / (Math.min(a,b) + .05)
}

const statusTokens = ['ok', 'warn', 'error', 'muted'] as const

export function refreshSelectionTheme(): void {
  const root = document.documentElement
  const style = getComputedStyle(root)
  const accent = (root.style.getPropertyValue('--selection') || style.getPropertyValue('--selection')).trim()
  const surface = style.getPropertyValue('--surface').trim()
  const foreground = rgb(accent)
  const base = rgb(surface)
  if (!foreground || !base) return
  const opacity = resolveSelectionOpacity(Number(root.dataset.selectionOpacity ?? defaultSelectionOpacity)) / 100
  const mixed = '#' + foreground.map((channel, i) =>
    Math.round(channel * opacity + base[i] * (1-opacity)).toString(16).padStart(2,'0')).join('')
  const text = readableTextColor(mixed)
  const edge = contrast(accent, surface) >= 3 ? accent : readableTextColor(surface)
  const neutral = opacity >= .8 && contrast('#ffffff', mixed) >= 4.5 ? '#ffffff' : text
  const badges = statusTokens.map(token => {
    const original = style.getPropertyValue('--'+token).trim()
    if (opacity < .8) {
      if (contrast(original, mixed) >= 4.5) return { color: original, background: 'transparent' }
      if (contrast(original, surface) >= 4.5) return { color: original, background: surface }
    }
    return { color: neutral, background: 'transparent' }
  })
  root.style.setProperty('--row-selection', mixed)
  root.style.setProperty('--row-selection-ink', text)
  root.style.setProperty('--row-selection-edge', edge)
  statusTokens.forEach((token, i) => {
    root.style.setProperty('--row-selection-'+token, badges[i].color)
    root.style.setProperty('--row-selection-'+token+'-bg', badges[i].background)
  })
}

export function applySelectionOpacity(value?: number): void {
  document.documentElement.dataset.selectionOpacity = String(resolveSelectionOpacity(value))
  refreshSelectionTheme()
}

export function applySelectionColor(preference?: SelectionColorPreference, systemAccent?: string, opacity = defaultSelectionOpacity): void {
  const selected = resolveSelectionColor(preference)
  const background = selected === 'system'
    ? (rgb(systemAccent ?? '') ? systemAccent! : selectionColors.blue)
    : selectionColors[selected === 'product' ? 'blue' : selected]
  const foreground = readableTextColor(background)
  const root = document.documentElement
  root.dataset.selectionColor = selected
  root.style.setProperty('--selection', background)
  root.style.setProperty('--selection-ink', foreground)
  root.style.setProperty('--selection-edge', foreground)
  applySelectionOpacity(opacity)
}

let cachedSystemAccent: string | undefined
let accentEpoch = 0

export async function refreshSelectionColor(preference: SelectionColorPreference | undefined, nativeBridge: boolean, opacity = defaultSelectionOpacity): Promise<void> {
  const epoch = ++accentEpoch
  const selected = resolveSelectionColor(preference)
  applySelectionColor(selected, cachedSystemAccent, opacity)
  if (selected !== 'system' || !nativeBridge) return
  try {
    const accent = await invoke<string>('system_accent_color')
    if (!rgb(accent)) throw new Error('InvalidSystemAccent')
    cachedSystemAccent = accent
    // An older response must not replace a newer palette selection.
    const root = document.documentElement
    if (epoch === accentEpoch && root.dataset.selectionColor === 'system')
      applySelectionColor('system', accent, resolveSelectionOpacity(Number(root.dataset.selectionOpacity)))
  } catch (error) {
    console.error('無法讀取 Windows 系統強調色', error)
  }
}

export function resolveTheme(theme: ThemePreference, prefersDark: boolean): 'light' | 'dark' {
  return theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme
}

export function shouldRespondToSystemTheme(theme: ThemePreference, settingsPreview: boolean): boolean {
  return theme === 'system' && !settingsPreview
}
