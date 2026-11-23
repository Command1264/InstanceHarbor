export const CONSOLE_INPUT_LIMIT = 8192

export function attachConsoleClipboard({currentSession, translate, showError, paste}) {
  const dialog = document.getElementById('console-paste-dialog')
  const title = document.getElementById('console-paste-title')
  const description = document.getElementById('console-paste-description')
  const preview = document.getElementById('console-paste-preview')
  const cancel = document.getElementById('console-paste-cancel')
  const confirm = document.getElementById('console-paste-confirm')
  const form = dialog.querySelector('form')
  let pendingRead = null
  let pendingPaste = null

  function sessionAtTarget(event) {
    const session = currentSession()
    return session && !session.closing && !dialog.open && session.panel.contains(event.target) ? session : null
  }

  function isCurrent(session) {
    return currentSession() === session && !session.closing && document.hasFocus()
  }

  function invalidate() {
    pendingRead = null
    pendingPaste = null
    preview.textContent = ''
    if (dialog.open) dialog.close('cancel')
  }

  function updateLanguage() {
    title.textContent = translate('console.pasteTitle')
    cancel.textContent = translate('console.pasteCancel')
    confirm.textContent = translate('console.pasteConfirm')
    if (pendingPaste) {
      description.textContent = translate('console.pasteDescription', { name: pendingPaste.session.name })
    }
  }

  function offerPaste(session, text) {
    if (!text || pendingPaste || !isCurrent(session)) return
    // Reject control characters other than line breaks before xterm encodes the input.
    if (text.length > CONSOLE_INPUT_LIMIT || new TextEncoder().encode(text).length > CONSOLE_INPUT_LIMIT ||
        /[\x00-\x09\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(text)) {
      showError(session, translate('console.pasteInvalid'))
      return
    }
    if (!/[\r\n]/.test(text)) {
      paste(session, text)
      return
    }
    pendingPaste = {session, text}
    updateLanguage()
    preview.textContent = text
    dialog.returnValue = 'cancel'
    dialog.showModal()
  }

  function settlePaste(confirmed) {
    // Settle before the dialog restores focus. The native close event is deferred.
    const request = pendingPaste
    pendingPaste = null
    preview.textContent = ''
    dialog.close(confirmed ? 'paste' : 'cancel')
    if (!request || !isCurrent(request.session)) return
    if (confirmed) paste(request.session, request.text)
    request.session.terminal.focus()
  }

  form.addEventListener('submit', event => {
    event.preventDefault()
    settlePaste(event.submitter === confirm)
  })
  dialog.addEventListener('cancel', event => {
    event.preventDefault()
    settlePaste(false)
  })

  async function copySelection(session, text) {
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      if (isCurrent(session)) showError(session, translate('console.copyFailed'))
    }
  }

  async function readClipboard(session) {
    if (pendingRead || pendingPaste) return
    const request = {focus: document.activeElement}
    pendingRead = request
    try {
      const text = await navigator.clipboard.readText()
      if (pendingRead === request && request.focus === document.activeElement && !dialog.open) offerPaste(session, text)
    } catch {
      if (pendingRead === request && isCurrent(session)) showError(session, translate('console.pasteFailed'))
    } finally {
      if (pendingRead === request) pendingRead = null
    }
  }

  document.addEventListener('keydown', event => {
    const session = sessionAtTarget(event)
    if (!session || event.altKey || event.metaKey || event.isComposing) return
    const key = event.key.toLowerCase()
    const copy = event.ctrlKey && (key === 'c' || (key === 'insert' && !event.shiftKey))
    const insert = (event.ctrlKey && key === 'v') || (event.shiftKey && !event.ctrlKey && key === 'insert')
    if (!copy && !insert) return
    event.preventDefault()
    event.stopPropagation()
    if (!event.isTrusted || event.repeat) return
    if (copy) {
      const selected = session.terminal.getSelection()
      if (selected) void copySelection(session, selected)
    } else {
      void readClipboard(session)
    }
  }, true)

  document.addEventListener('copy', event => {
    const session = sessionAtTarget(event)
    if (!session || !event.isTrusted) return
    const selected = session.terminal.getSelection()
    if (!selected || !event.clipboardData) return
    event.preventDefault()
    event.stopPropagation()
    event.clipboardData.setData('text/plain', selected)
  }, true)

  document.addEventListener('paste', event => {
    const session = sessionAtTarget(event)
    if (!session) return
    event.preventDefault()
    event.stopPropagation()
    if (event.isTrusted && !pendingRead) offerPaste(session, event.clipboardData?.getData('text/plain'))
  }, true)

  window.addEventListener('blur', invalidate)
  return {invalidate, updateLanguage}
}
