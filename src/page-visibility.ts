// Fast open (#56) hides the main window with X and makes its WebView2 invisible, so the
// page's visibilityState becomes 'hidden'. Background polls skip their work then, and the
// page reads everything once when the window is shown again.
interface VisibilitySource {
  readonly visibilityState: string
  addEventListener(type: 'visibilitychange', listener: () => void): void
  removeEventListener(type: 'visibilitychange', listener: () => void): void
}

const page = (): VisibilitySource | undefined => typeof document === 'undefined' ? undefined : document

export function pageHidden(source: VisibilitySource | undefined = page()): boolean {
  return source?.visibilityState === 'hidden'
}

/** Calls `shown` each time the page turns visible again; returns the function that stops it. */
export function onPageShown(shown: () => void, source: VisibilitySource | undefined = page()): () => void {
  if (!source) return () => {}
  let hidden = pageHidden(source)
  const listener = () => {
    const now = pageHidden(source)
    if (hidden && !now) shown()
    hidden = now
  }
  source.addEventListener('visibilitychange', listener)
  return () => source.removeEventListener('visibilitychange', listener)
}
