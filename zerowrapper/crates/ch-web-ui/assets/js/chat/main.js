import * as network from './network.js'
import { els } from './dom.js'
import * as dom from './dom.js'
import * as auth from './auth.js'
import * as connection from './connection.js'
import * as streaming from './streaming.js'
import { addMessage } from './messages.js'
import * as lightbox from './lightbox.js'
import { uploadFile, createAttachmentChip, uploadState } from './file-upload.js'

function logPendingResources() {
  const entries = performance.getEntriesByType('resource')
  const pending = entries.filter(e => !e.responseEnd || e.responseEnd <= 0)
  if (pending.length) {
    console.warn('[resources] pending:', pending.map(r => r.name))
  } else {
    console.log('[resources] all', entries.length, 'resources loaded')
  }
  const fonts = document.fonts ? document.fonts.status : 'n/a'
  console.log('[resources] readyState:', document.readyState, 'fonts:', fonts)
}

export function init() {
  dom.init()
  logPendingResources()
  setTimeout(logPendingResources, 3000)

  if (els.chatRefreshBtn) {
    els.chatRefreshBtn.addEventListener('click', () => location.reload())
  }

  if (!els.form) return

  lightbox.init(els.messages)

  auth.loadFromQuery()
  auth.loadFromStorage()

  if (auth.state.email) {
    auth.showBadge()
    connection.connect()
  } else {
    auth.showForm()
  }

  els.registerBtn.addEventListener('click', handleLogin)
  els.emailInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') els.registerBtn.click() })

  els.logoutBtn.addEventListener('click', handleLogout)

  if (els.uploadBtn && els.fileInput) {
    els.uploadBtn.addEventListener('click', () => els.fileInput.click())
    els.fileInput.addEventListener('change', async () => {
      const files = Array.from(els.fileInput.files || [])
      for (const file of files) {
        try {
          const info = await uploadFile(file)
          uploadState.pending.push(info)
          els.attachments.appendChild(createAttachmentChip(info))
        } catch (err) {
          addMessage('error', 'Upload failed: ' + err.message)
        }
      }
      els.fileInput.value = ''
    })
  }

  els.form.addEventListener('submit', handleSubmit)
  els.input.addEventListener('input', autoGrow)
  els.cancelBtn.addEventListener('click', handleCancel)
  streaming.setOnFinalize(showSendIcon)
}

function showSendIcon() {
  if (els.sendBtn) els.sendBtn.style.display = 'flex'
  if (els.cancelBtn) els.cancelBtn.style.display = 'none'
}

function showStopIcon() {
  if (els.sendBtn) els.sendBtn.style.display = 'none'
  if (els.cancelBtn) els.cancelBtn.style.display = 'flex'
}

function handleCancel() {
  connection.sendCancel()
  showSendIcon()
}

function handleLogin() {
  auth.hideAuthError()
  const raw = els.emailInput.value.trim()
  if (!raw) { auth.showAuthError('Please enter your email.'); return }

  auth.state.email = raw
  auth.state.sessionId = auth.buildSessionId(raw)
  auth.saveToStorage()
  auth.showBadge()
  connection.connect()
}

function handleLogout() {
  auth.clearFromStorage()
  auth.state.email = ''
  auth.state.sessionId = ''
  connection.disconnect()
  els.messages.innerHTML = ''
  if (els.hint) els.hint.style.display = 'block'
  if (els.hintText) els.hintText.textContent = els.hintText.dataset.fallback || els.hintText.textContent
  if (els.chatRefreshBtn) els.chatRefreshBtn.classList.add('hidden')
  auth.showForm()
}

function handleSubmit(e) {
  e.preventDefault()
  if (!auth.state.email) { auth.scrollToAuth(); return }
  streaming.finalizeIfActive()
  const text = els.input.value.trim()
  const attachments = [...uploadState.pending]
  if (!text && attachments.length === 0) { addMessage('error', 'Cannot send an empty message.'); return }
  if (network.getState() !== network.State.CONNECTED) { addMessage('error', 'Not connected to backend.'); return }
  showStopIcon()
  const combined = text && attachments.length > 0
    ? JSON.stringify({ message: text, attachments: attachments.map(a => ({ filename: a.filename, mime_type: a.mime_type, path: a.path })) })
    : (attachments.length > 0
      ? JSON.stringify({ attachments: attachments.map(a => ({ filename: a.filename, mime_type: a.mime_type, path: a.path })) })
      : text)
  addMessage('user', combined)
  connection.send(text, undefined, attachments.map(a => ({ filename: a.filename, mime_type: a.mime_type, path: a.path })))
  streaming.showTyping()
  els.input.value = ''
  els.input.style.height = 'auto'
  uploadState.pending = []
  if (els.attachments) els.attachments.innerHTML = ''
}

function autoGrow() {
  els.input.style.height = 'auto'
  els.input.style.height = Math.min(els.input.scrollHeight, 300) + 'px'
}
