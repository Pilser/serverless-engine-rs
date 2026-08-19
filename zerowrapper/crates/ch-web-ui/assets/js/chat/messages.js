import { marked } from 'marked'
import { els } from './dom.js'
import { USER_AVATAR, BOT_AVATAR, escapeHtml } from './constants.js'
import { render as renderMarkdown } from './markdown.js'
import { finalize } from './streaming.js'

let abortController = null

function extractMessageText(raw) {
  if (!raw) return ''
  if (raw.startsWith('{')) {
    try {
      const p = JSON.parse(raw)
      if (p.message) return p.message
    } catch (e) {}
  }
  return raw
}

function extractAttachments(raw) {
  if (!raw || !raw.startsWith('{')) return null
  try {
    const p = JSON.parse(raw)
    if (p.attachments && Array.isArray(p.attachments) && p.attachments.length > 0) {
      return p.attachments
    }
  } catch (e) {}
  return null
}

function renderTextWithAttachments(text) {
  const div = document.createElement('div')
  const attachments = extractAttachments(text)
  const msgText = extractMessageText(text)
  if (attachments) {
    const fileList = document.createElement('div')
    fileList.className = 'msg-attachments'
    attachments.forEach((a) => {
      const chip = document.createElement('span')
      chip.className = 'attachment-chip'
      chip.innerHTML = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><polyline points="14 2 14 8 20 8"/></svg> ' + escapeHtml(a.filename || a.name || 'file')
      fileList.appendChild(chip)
    })
    div.appendChild(fileList)
  }
  if (msgText) {
    const p = document.createElement('p')
    p.style.margin = attachments ? '6px 0 0' : '0'
    p.textContent = msgText
    div.appendChild(p)
  }
  return div
}

function buildMessageRow(role, text) {
  const row = document.createElement('div')
  row.className = 'message-row ' + role

  if (role === 'error') {
    const content = document.createElement('div')
    content.className = 'msg-content'
    content.textContent = text
    row.appendChild(content)
  } else if (role === 'user') {
    const avatar = document.createElement('div')
    avatar.className = 'msg-avatar'
    avatar.innerHTML = USER_AVATAR
    row.appendChild(avatar)
    const content = document.createElement('div')
    content.className = 'msg-content'
    content.appendChild(renderTextWithAttachments(text))
    row.appendChild(content)
  } else {
    const avatar = document.createElement('div')
    avatar.className = 'msg-avatar'
    avatar.innerHTML = BOT_AVATAR
    row.appendChild(avatar)
    const content = document.createElement('div')
    content.className = 'msg-content chat-markdown'
    renderMarkdown(content, marked.parse(text || ''))
    row.appendChild(content)
  }
  return row
}

export function addMessage(role, text, scroll = true) {
  finalize()

  const row = buildMessageRow(role, text)
  els.messages.appendChild(row)
  if (els.hint && els.hint.style.display !== 'none') els.hint.style.display = 'none'
  if (scroll) {
    row.scrollIntoView({ behavior: 'smooth', block: 'end' })
  }
}

export function scrollToBottom() {
  const lastMsg = els.messages.lastElementChild
  if (lastMsg) {
    lastMsg.scrollIntoView({ block: 'end' })
  }
}

export function fetchHistory() {
  if (abortController) abortController.abort()
  abortController = new AbortController()

  const email = localStorage.getItem('zerowrapper_email')
  if (!email) return

  const url = '?api=history&user_id=' + encodeURIComponent(email)
  fetch(url, { signal: abortController.signal })
    .then(r => r.json())
    .then(data => {
      if (data.session_id) {
        localStorage.setItem('zerowrapper_session', data.session_id)
      }
      if (!data.messages || data.messages.length === 0) return
      const frag = document.createDocumentFragment()
      data.messages.forEach(msg => {
        if (msg.role === 'user' || msg.role === 'assistant') {
          const row = buildMessageRow(msg.role, msg.content)
          if (row) frag.appendChild(row)
        }
      })
      els.messages.appendChild(frag)
      scrollToBottom()
    })
    .catch(() => {})
}

export function abortHistoryFetch() {
  if (abortController) {
    abortController.abort()
    abortController = null
  }
}
