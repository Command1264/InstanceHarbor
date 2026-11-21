import { invoke } from '@tauri-apps/api/core'
import '../../experiments/conpty-a-prototype/ui/vendor/xterm.css'
import { resolveLanguage, translate } from './i18n'
import { attachConsoleScroll } from './console-scroll'
import { attachConsoleClipboard, CONSOLE_INPUT_LIMIT } from './console-clipboard'
import './native-console.css'

function loadScript(url) {
  return new Promise((resolve, reject) => {
    const script = document.createElement('script')
    script.src = url
    script.onload = resolve
    script.onerror = () => reject(new Error('TerminalAssetFailed'))
    document.head.append(script)
  })
}

const tabs = document.getElementById('console-tabs')
const panels = document.getElementById('console-panels')
const viewerError = document.getElementById('console-error')
const sessions = new Map()
const encoder = new TextEncoder()
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
let active = null
function updateLayout() {
  if (active) active.fitView?.(measureLayout(active))
  active?.wakeLayout?.()
  active?.scroll.refresh()
}
const layout = new ResizeObserver(updateLayout)
layout.observe(panels)
window.addEventListener('resize', updateLayout)
let pendingTitle = Promise.resolve()
let locale = 'zh-TW'
let languagePreference
const t = (id, values) => translate(locale, id, values)
const clipboard = attachConsoleClipboard({
  currentSession: () => active,
  translate: t,
  showError: (session, text) => message(session.error, text),
  paste: (session, text) => {
    session.scroll.acceptInput(text)
    session.terminal.paste(text)
  },
})

function updateLanguage() {
  locale = resolveLanguage(languagePreference, navigator.languages)
  document.documentElement.lang = locale
  tabs.setAttribute('aria-label', t('console.tabs'))
  clipboard.updateLanguage()
  for (const session of sessions.values()) {
    labelClose(session)
    session.scroll.setLabel(t('console.scrollback'))
  }
}

function labelClose(session) {
  session.closeButton.setAttribute('aria-label', t('console.closeTab', { name: session.name }))
}

window.addEventListener('languagechange', updateLanguage)

function message(element, text = '') {
  element.textContent = text
  element.hidden = !text
}

function focusCurrent() {
  // Assign focus within this document even while Windows is activating the
  // console. This does not raise the native window or take OS focus.
  active?.terminal.focus()
}

window.addEventListener('focus', focusCurrent)
window.addEventListener('console-focus', () => requestAnimationFrame(focusCurrent))

function select(session, moveFocus = true) {
  if (active !== session) clipboard.invalidate()
  active = session
  for (const item of sessions.values()) {
    const selected = item === session
    item.panel.hidden = !selected
    item.button.setAttribute('aria-selected', String(selected))
    item.button.tabIndex = selected ? 0 : -1
    item.closeButton.tabIndex = selected ? 0 : -1
  }
  session?.wakeLayout?.()
  session?.scroll.refresh()
  session?.button.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  if (session) {
    pendingTitle = pendingTitle.then(() => active === session && invoke('terminal_title', { run: session.run }))
      .catch(error => console.warn('Could not update console title', error))
  }
  if (moveFocus) focusCurrent()
}

function adjacent(delta) {
  const ordered = [...sessions.values()]
  return ordered[(ordered.indexOf(active) + delta + ordered.length) % ordered.length]
}

tabs.addEventListener('keydown', event => {
  if (event.target.getAttribute('role') !== 'tab' || !active) return
  const ordered = [...sessions.values()]
  const next = { ArrowLeft: () => adjacent(-1), ArrowRight: () => adjacent(1),
    Home: () => ordered[0], End: () => ordered.at(-1) }[event.key]?.()
  if (!next) return
  event.preventDefault()
  select(next, false)
  next.button.focus()
})

document.addEventListener('keydown', event => {
  if (!active || event.isComposing || !event.ctrlKey || event.altKey) return
  if (!['PageUp', 'PageDown'].includes(event.key)) return
  event.preventDefault()
  event.stopPropagation()
  select(adjacent(event.key === 'PageUp' ? -1 : 1))
}, true)

async function closeSession(session) {
  if (session.closing) return false
  session.closing = true
  try {
    if (!await invoke('terminal_close', { run: session.run, version: session.version })) return false
    const ordered = [...sessions.values()]
    const index = ordered.indexOf(session)
    session.closed = true
    session.wakeLayout?.()
    sessions.delete(session.run)
    session.tab.remove()
    session.panel.remove()
    session.scroll.dispose()
    session.terminal.dispose()
    if (active === session) select(ordered[index + 1] ?? ordered[index - 1] ?? null)
    return true
  } catch {
    message(session.error, t('console.closeFailed'))
    return false
  } finally {
    session.closing = false
  }
}

function labelSession(session, title) {
  session.button.textContent = title
  session.button.title = title
}

function fittedGrid(session) {
  const { terminal, surface } = session
  const screen = surface.querySelector('.xterm-screen')
  const { width = 0, height = 0 } = screen?.getBoundingClientRect() ?? {}
  if (!width || !height || !surface.clientWidth || !surface.clientHeight) return null
  // xterm sizes this element to the rendered grid, excluding the overlay rail.
  return {
    cols: Math.max(20, Math.min(500, Math.floor((surface.clientWidth - 14) * terminal.cols / width))),
    rows: Math.max(10, Math.min(200, Math.floor(surface.clientHeight * terminal.rows / height))),
  }
}

function sameGrid(a, b) {
  return a?.cols === b?.cols && a?.rows === b?.rows
}

function measureLayout(session) {
  const grid = fittedGrid(session)
  if (!sameGrid(grid, session.layoutGrid)) {
    session.layoutGrid = grid
    session.layoutChangedAt = performance.now()
  }
  return grid
}

function applyGrid(terminal, grid) {
  if (grid && (terminal.cols !== grid.cols || terminal.rows !== grid.rows)) {
    terminal.resize(grid.cols, grid.rows)
  }
}
function canReflowBeforeOutput(terminal, grid) {
  const buffer = terminal.buffer.active
  if (buffer.type !== 'normal' || buffer.cursorX >= grid.cols) return false
  // Keep the active line and everything below it intact if unread output
  // briefly requires the old grid again.
  const cursor = buffer.baseY + buffer.cursorY
  for (let row = cursor; row < buffer.length; row++) {
    const line = buffer.getLine(row)
    const firstClippedColumn = row === cursor ? grid.cols : 0
    if (line?.translateToString(true, firstClippedColumn)) return false
  }
  return true
}
function addSession({ name, run, title }) {
  const tab = document.createElement('div')
  tab.className = 'console-tab'
  tab.setAttribute('role', 'presentation')
  const button = document.createElement('button')
  button.className = 'tab-name'
  button.id = 'tab-' + run
  button.setAttribute('role', 'tab')
  button.setAttribute('aria-controls', 'console-' + run)

  const closeButton = document.createElement('button')
  closeButton.className = 'tab-close'
  closeButton.textContent = '×'
  tab.append(button, closeButton)
  tabs.append(tab)

  const panel = document.createElement('section')
  panel.className = 'console-panel'
  panel.id = 'console-' + run
  panel.setAttribute('role', 'tabpanel')
  panel.setAttribute('aria-labelledby', button.id)
  const surface = document.createElement('div')
  surface.className = 'console-surface'
  const viewport = document.createElement('div')
  viewport.className = 'console-screen'
  surface.append(viewport)
  const error = document.createElement('p')
  error.className = 'console-error'
  error.setAttribute('role', 'status')
  error.hidden = true
  panel.append(surface, error)
  panels.append(panel)

  const terminal = new window.Terminal({
    cols: 100, rows: 35, scrollback: 20000, allowProposedApi: true,
    fontFamily: '"Cascadia Mono", Consolas, "Microsoft JhengHei UI", monospace',
    fontSize: 15, lineHeight: 1.1, theme: {
      background: '#101820', foreground: '#e8f0f4',
    },
  })
  terminal.loadAddon(new window.UnicodeGraphemesAddon.UnicodeGraphemesAddon())
  terminal.unicode.activeVersion = '15-graphemes'
  terminal.open(viewport)
  const scroll = attachConsoleScroll(terminal, surface, viewport, panel.id, t('console.scrollback'))
  const session = { name, run, tab, button, closeButton, panel, surface, terminal, scroll, error, closed: false, closing: false }
  labelSession(session, title ?? name)
  labelClose(session)
  sessions.set(run, session)
  button.addEventListener('click', () => select(session))
  closeButton.addEventListener('click', () => void closeSession(session))

  let pending = Promise.resolve()
  function exchange(request) {
    const next = pending.then(() => {
      if (session.closed) throw new Error('ConsoleTabClosed')
      return invoke('terminal_io', { run, request })
    })
    pending = next.then(() => undefined, () => undefined)
    return next
  }
  let queuedInput = 0
  terminal.onData(data => {
    if (session.closing || active !== session) return
    // ETX is interrupt in ConPTY. Keep it local even if an input method bypasses keydown.
    if (data.includes('\x03')) return
    scroll.acceptInput(data)
    const bytes = encoder.encode(data)
    if (bytes.length > CONSOLE_INPUT_LIMIT || queuedInput >= 32) {
      message(error, t('console.inputNotSent'))
      return
    }
    queuedInput++
    void exchange({ op: 'input', bytes: Array.from(bytes) })
      .catch(() => message(error, t('console.inputUnconfirmed')))
      .finally(() => { queuedInput-- })
  })
  void readSession(session, exchange)
  return session
}

function waitForOutputOrLayout(session, timeout) {
  return new Promise(resolve => {
    const wake = () => {
      clearTimeout(timer)
      session.wakeLayout = null
      resolve()
    }
    const timer = setTimeout(wake, timeout)
    session.wakeLayout = wake
  })
}
async function readSession(session, exchange) {
  let offset = 0
  let failures = 0
  let resizeDisabled = false
  let hostGrid
  let previousGrid
  let writing = false
  let atTail = false
  const targetGrid = () => active === session && !resizeDisabled ? measureLayout(session) : null
  session.fitView = grid => {
    const { terminal } = session
    // Horizontal drags can paint the latest width immediately. Height changes
    // still need a known restore grid or confirmation from the Host.
    if (!resizeDisabled && !writing && hostGrid && grid && !sameGrid(grid, terminal) &&
        (grid.rows === hostGrid.rows || sameGrid(grid, previousGrid)) &&
        canReflowBeforeOutput(terminal, { cols: Math.min(grid.cols, hostGrid.cols) })) {
      applyGrid(terminal, grid)
    }
  }
  // A native restore returns to the previous confirmed grid. Request it at
  // once, while coalescing intermediate sizes during an edge drag.
  const settleDelay = grid => {
    if (!grid || sameGrid(grid, hostGrid)) return Infinity
    if (sameGrid(grid, previousGrid)) return 0
    return Math.max(0, 32 - (performance.now() - session.layoutChangedAt))
  }
  while (!session.closed) {
    // If the last read reached the tail, let the Host compare the byte offset
    // during resize. This avoids a separate empty read before window restore.
    const grid = atTail ? targetGrid() : null
    if (settleDelay(grid) === 0) {
      try {
        const reply = await exchange({ op: 'resize', ...grid, offset })
        if (session.closed) return
        atTail = reply.caughtUp
        if (atTail) {
          if (!sameGrid(hostGrid, reply.grid)) previousGrid = hostGrid
          hostGrid = reply.grid
          const latest = targetGrid()
          // An obsolete empty grid need not be painted. Any VT bytes generated
          // at that width still have their grid supplied by the next read.
          if (sameGrid(hostGrid, latest)) applyGrid(session.terminal, hostGrid)
          else if (settleDelay(latest) === 0) continue
        }
      } catch (error) {
        if (session.closed) return
        resizeDisabled = true
        if (!['InvalidConsoleRequest', 'ConsoleRunEnded'].includes(String(error))) {
          console.warn('Console resize unavailable', error)
        }
      }
    }
    let chunk
    try {
      chunk = await exchange({ op: 'read', offset })
    } catch (error) {
      if (session.closed) return
      if (['ConsoleRunChanged', 'ConsoleRunEnded'].includes(String(error))) {
        if (await closeSession(session)) return
      } else if (++failures === 3) {
        message(session.error, t('console.disconnected'))
      }
      atTail = false
      await delay(250)
      continue
    }
    if (session.closed) return
    if (failures >= 3) message(session.error)
    failures = 0
    hostGrid = chunk.grid ?? hostGrid
    if (chunk.bytes.length) {
      writing = true
      // Historical VT bytes always use the Host grid for their byte offset,
      // even if the viewport was fitted while this read was pending.
      applyGrid(session.terminal, chunk.grid)
      await new Promise(resolve => session.terminal.write(Uint8Array.from(chunk.bytes), resolve))
      writing = false
      session.fitView(targetGrid())
      offset += chunk.bytes.length
      atTail = false
      continue
    }

    atTail = true
    // Measure after the IPC round trip so a layout change during the read is
    // handled now even if there was no waiter when the observer fired.
    const remaining = settleDelay(targetGrid())
    if (remaining === 0) continue
    if (remaining === Infinity) applyGrid(session.terminal, hostGrid)
    const poll = active === session ? 80 : 240
    await waitForOutputOrLayout(session, Math.min(poll, remaining))
  }
}

async function run() {
  try {
    await loadScript(new URL('../../experiments/conpty-a-prototype/ui/vendor/xterm.js', import.meta.url).href)
    await loadScript(new URL('../../experiments/conpty-a-prototype/ui/vendor/graphemes.js', import.meta.url).href)
  } catch {
    message(viewerError, t('console.loadFailed'))
    await invoke('terminal_present')
    return
  }
  while (true) {
    try {
      const pending = await invoke('terminal_pending')
      if (pending.length) {
        languagePreference = pending.at(-1).language
        updateLanguage()
        for (const item of pending) {
          const session = sessions.get(item.run) ?? addSession(item)
          labelSession(session, item.title ?? item.name)
          session.version = item.version
          select(session, false)
        }
        await invoke('terminal_present')
        focusCurrent()
      }
      message(viewerError)
    } catch {
      message(viewerError, t('console.openFailed'))
    }
    await delay(100)
  }
}

void run()
