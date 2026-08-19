import { marked } from 'marked'
import { render as renderMarkdown } from './markdown.js'

export function humanSize(bytes) {
  if (bytes === undefined || bytes === null) return ''
  if (bytes < 1024) return bytes + ' B'
  const units = ['KB', 'MB', 'GB', 'TB']
  let n = bytes
  let i = -1
  do {
    n /= 1024
    i++
  } while (n >= 1024 && i < units.length - 1)
  return n.toFixed(1) + ' ' + units[i]
}

export function renderBody(container, bodyMd) {
  if (!container) return
  container.classList.add('chat-markdown')
  if (!bodyMd || !bodyMd.trim()) {
    container.innerHTML = ''
    return
  }
  renderMarkdown(container, marked.parse(decodeEntities(bodyMd)), true)
}

function decodeEntities(s) {
  return s
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&')
}

function buildCard(pin) {
  const card = document.createElement('div')
  card.className = 'whiteboard-pin-card'
  const title = document.createElement('div')
  title.className = 'pin-name'
  title.textContent = pin.name || 'file'
  const meta = document.createElement('div')
  meta.className = 'pin-meta'
  meta.textContent = (pin.mime || 'file') + (pin.size ? ' · ' + humanSize(pin.size) : '')
  card.appendChild(title)
  card.appendChild(meta)
  return card
}

export function renderPins(container, pins) {
  if (!container) return
  container.innerHTML = ''
  if (!pins || pins.length === 0) return
  const frag = document.createDocumentFragment()
  pins.forEach(pin => {
    const url = pin.url || '#'
    const mime = (pin.mime || '').toLowerCase()
    const name = pin.name || 'file'
    const card = buildCard(pin)

    if (mime.startsWith('image/')) {
      const img = document.createElement('img')
      img.src = url
      img.alt = name
      img.loading = 'lazy'
      card.appendChild(img)
    } else if (mime === 'application/pdf') {
      const frame = document.createElement('iframe')
      frame.src = url
      frame.title = name
      card.appendChild(frame)
    } else if (mime.startsWith('text/')) {
      const pre = document.createElement('pre')
      pre.textContent = 'Loading…'
      card.appendChild(pre)
      fetch(url)
        .then(r => (r.ok ? r.text() : Promise.reject(new Error('fetch failed'))))
        .then(text => { pre.textContent = text })
        .catch(() => { pre.textContent = '' })
    } else {
      const link = document.createElement('a')
      link.href = url
      link.target = '_blank'
      link.rel = 'noopener'
      link.textContent = 'Download ' + name
      card.appendChild(link)
    }
    frag.appendChild(card)
  })
  container.appendChild(frag)
}

export function renderBoard(bodyContainer, pinsContainer, board) {
  renderBody(bodyContainer, board && board.body_md)
  renderPins(pinsContainer, board && board.pins)
}
