/** Scrollback plus a display-only margin beyond the live prompt. */
export function attachConsoleScroll(terminal, surface, screen, panelId, label) {
  const rail = document.createElement('div')
  const thumb = document.createElement('div')
  rail.className = 'console-rail'
  rail.tabIndex = 0
  rail.setAttribute('role', 'scrollbar')
  rail.setAttribute('aria-controls', panelId)
  rail.setAttribute('aria-orientation', 'vertical')
  rail.setAttribute('aria-valuemin', '0')
  rail.setAttribute('aria-label', label)
  thumb.className = 'console-thumb'
  rail.append(thumb)
  surface.append(rail)

  let extra = 0
  let remainder = 0
  let frame = 0
  let drag = null

  function bounds() {
    const buffer = terminal.buffer.active
    let last = 0
    if (buffer.type === 'normal') {
      for (let index = terminal.rows - 1; index >= 0; index--) {
        if (buffer.getLine(buffer.baseY + index)?.translateToString(true)) {
          last = index
          break
        }
      }
    }
    return { buffer, limit: buffer.type === 'normal' ? last : 0 }
  }

  function cellHeight() {
    return (screen.querySelector('.xterm-screen')?.offsetHeight ?? 0) / terminal.rows
  }

  function paint() {
    frame = 0
    const { buffer, limit } = bounds()
    extra = buffer.viewportY === buffer.baseY ? Math.min(extra, limit) : 0
    const value = buffer.viewportY + extra
    const max = buffer.baseY + limit
    rail.hidden = max === 0
    const height = rail.clientHeight
    // The transform changes only the viewport. xterm and ConPTY keep their own rows.
    screen.style.transform = extra ? `translateY(-${extra * cellHeight()}px)` : ''
    rail.setAttribute('aria-valuemax', String(max))
    rail.setAttribute('aria-valuenow', String(value))
    if (height && max) {
      const size = Math.min(height, Math.max(24, height * terminal.rows / (terminal.rows + max)))
      thumb.style.height = `${size}px`
      thumb.style.transform = `translateY(${(height - size) * value / max}px)`
    }
  }

  function schedule() {
    if (!frame) frame = requestAnimationFrame(paint)
  }

  function scrollTo(target) {
    const { buffer, limit } = bounds()
    const row = Math.max(0, Math.min(buffer.baseY + limit, Math.round(target)))
    extra = Math.max(0, row - buffer.baseY)
    terminal.scrollToLine(Math.min(row, buffer.baseY))
    schedule()
  }

  function live() {
    extra = 0
    remainder = 0
    screen.style.transform = ''
    terminal.scrollToBottom()
    schedule()
  }

  surface.addEventListener('wheel', event => {
    if (!event.deltaY || event.ctrlKey || terminal.modes.mouseTrackingMode !== 'none' ||
        terminal.buffer.active.type !== 'normal') return
    const height = cellHeight()
    if (!height) return
    const factor = event.deltaMode === 1 ? 1 : event.deltaMode === 2 ? terminal.rows : 1 / height
    if (Math.sign(event.deltaY) !== Math.sign(remainder)) remainder = 0
    remainder += event.deltaY * factor
    const lines = Math.trunc(remainder)
    remainder -= lines
    if (lines) scrollTo(terminal.buffer.active.viewportY + extra + lines)
    event.preventDefault()
    event.stopPropagation()
  }, { capture: true, passive: false })

  surface.addEventListener('pointerdown', event => {
    if (event.target !== surface || event.button !== 0) return
    event.preventDefault()
    terminal.focus()
  })
  rail.addEventListener('pointerdown', event => {
    if (event.button !== 0) return
    event.preventDefault()
    terminal.focus()
    rail.setPointerCapture(event.pointerId)
    const handle = thumb.getBoundingClientRect()
    drag = { id: event.pointerId, offset: event.target === thumb ? event.clientY - handle.top : handle.height / 2 }
    move(event)
  })
  function move(event) {
    if (drag?.id !== event.pointerId) return
    const travel = rail.clientHeight - thumb.getBoundingClientRect().height
    if (travel > 0) {
      const { buffer, limit } = bounds()
      scrollTo((event.clientY - rail.getBoundingClientRect().top - drag.offset) / travel * (buffer.baseY + limit))
    }
  }
  rail.addEventListener('pointermove', move)
  rail.addEventListener('lostpointercapture', () => { drag = null })
  rail.addEventListener('keydown', event => {
    const { buffer, limit } = bounds()
    const current = buffer.viewportY + extra
    const destination = { ArrowUp: current - 1, ArrowDown: current + 1, PageUp: current - terminal.rows,
      PageDown: current + terminal.rows, Home: 0, End: buffer.baseY + limit }[event.key]
    if (destination === undefined) return
    event.preventDefault()
    scrollTo(destination)
  })

  screen.addEventListener('keydown', event => {
    if (event.shiftKey && !event.ctrlKey && !event.altKey && ['PageUp', 'PageDown'].includes(event.key) &&
        terminal.buffer.active.type === 'normal' && terminal.modes.mouseTrackingMode === 'none') {
      event.preventDefault()
      event.stopPropagation()
      scrollTo(terminal.buffer.active.viewportY + extra + (event.key === 'PageUp' ? -terminal.rows : terminal.rows))
      return
    }
    if (extra && !event.isComposing && !event.ctrlKey && !event.metaKey && !event.altKey &&
        (event.key.length === 1 || ['Enter', 'Backspace', 'Delete'].includes(event.key))) live()
  }, true)
  for (const event of ['compositionstart', 'beforeinput', 'input', 'paste']) {
    screen.addEventListener(event, () => { if (extra) live() }, true)
  }
  terminal.onScroll(schedule)
  terminal.onWriteParsed(schedule)
  terminal.onResize(schedule)
  schedule()
  return {
    refresh: schedule,
    acceptInput: data => {
      // Device replies begin with ESC and must not move a reading viewport.
      if (extra && !data.startsWith('\x1b')) live()
    },
    setLabel: value => rail.setAttribute('aria-label', value),
    dispose: () => cancelAnimationFrame(frame),
  }
}
