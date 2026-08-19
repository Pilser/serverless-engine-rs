import { marked } from 'marked'
import { els } from './dom.js'
import { BOT_AVATAR } from './constants.js'
import { render as renderMarkdown } from './markdown.js'

let el = null
let content = ''
let activeThought = null
let streamId = 0
let renderPending = false
let onFinalize = null

export function setOnFinalize(cb) { onFinalize = cb }

function getEl() {
  if (!el) {
    const row = document.createElement('div')
    row.className = 'message-row assistant'
    const avatar = document.createElement('div')
    avatar.className = 'msg-avatar'
    avatar.innerHTML = BOT_AVATAR
    row.appendChild(avatar)
    el = document.createElement('div')
    el.className = 'msg-content chat-markdown'
    row.appendChild(el)
    els.messages.appendChild(row)
  }
  return el
}

function scheduleRender(id) {
  if (id !== streamId) return
  if (renderPending) return
  renderPending = true
  requestAnimationFrame(() => {
    renderPending = false
    if (streamId !== id) return
    const target = getEl()
    let html = ''
    if (content) {
      html += marked.parse(content)
    }
    renderMarkdown(target, html)
    target.scrollIntoView({ behavior: 'smooth', block: 'end' })
  })
}

export function appendThought(content, container) {
  const target = container || els.messages
  if (!target) return
  if (!activeThought) {
    const pill = document.createElement('div')
    pill.className = 'thought-pill'
    pill.innerHTML = '<div class="tc-pill-inner">+ thoughts</div>' +
      '<div class="tc-pill-detail hidden"><pre></pre></div>'
    pill.addEventListener('click', () => {
      const detail = pill.querySelector('.tc-pill-detail')
      if (detail) detail.classList.toggle('hidden')
    })
    target.appendChild(pill)
    const detail = pill.querySelector('.tc-pill-detail')
    if (detail) detail.classList.remove('hidden')
    pill.scrollIntoView({ behavior: 'smooth', block: 'end' })
    activeThought = pill
  }
  const pre = activeThought.querySelector('.tc-pill-detail pre')
  if (pre) pre.textContent += (content || '')
}

export function closeThought() {
  if (activeThought) {
    const detail = activeThought.querySelector('.tc-pill-detail')
    if (detail) detail.classList.add('hidden')
    activeThought = null
  }
}

function finalizeThought() { closeThought() }

export function handleThinking(text, container) {
  if (!container) showTyping()
  appendThought(text, container)
}

export function handleChunk(text) {
  finalizeThought()
  hideTyping()
  content += text
  scheduleRender(streamId)
}

export function finalize() {
  finalizeThought()
  if (!el) { if (onFinalize) onFinalize(); return }
  const target = el
  const trimmed = content.trim()
  if (trimmed) {
    target.className = 'msg-content chat-markdown'
    renderMarkdown(target, marked.parse(trimmed))
  }
  el = null
  content = ''
  streamId++
  if (onFinalize) onFinalize()
}

export function finalizeIfActive() {
  if (activeThought || content) finalize()
}

export function handleDone(text) {
  finalizeThought()
  hideTyping()
  if (text) content = text
  finalize()
}

export function isActive() {
  return el !== null
}

export function cancel() {
  finalizeThought()
  finalize()
  hideTyping()
}

export function reset() {
  if (activeThought) {
    const parent = activeThought.parentElement
    if (parent) parent.remove()
    activeThought = null
  }
  if (el) {
    const parent = el.parentElement
    if (parent) parent.remove()
    el = null
  }
  content = ''
  streamId++
}

export function showTyping() {
  if (els.typingEl) {
    els.typingEl.classList.remove('hidden')
    els.typingEl.classList.add('active')
  }
}

export function hideTyping() {
  if (els.typingEl) {
    els.typingEl.classList.add('hidden')
    els.typingEl.classList.remove('active')
  }
}
