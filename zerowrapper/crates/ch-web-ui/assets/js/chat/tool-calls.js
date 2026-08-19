import { els } from './dom.js'
import { escapeHtml } from './constants.js'

let pending = []

export function handleToolCall(id, name, args, container) {
  const target = container || els.messages
  if (!target) return
  const pill = document.createElement('div')
  pill.className = 'tool-call-pill'
  pill.dataset.id = id || ''
  const argsStr = args ? JSON.stringify(args, null, 2) : null
  pill.innerHTML = '<div class="tc-pill-inner">' +
    '+ acting' +
    '<span class="tc-pill-status running">\u25CC</span>' +
  '</div>' +
  (argsStr ? '<div class="tc-pill-detail hidden"><pre>' + escapeHtml(argsStr) + '</pre></div>' : '')

  pill.addEventListener('click', () => {
    const detail = pill.querySelector('.tc-pill-detail')
    if (detail) detail.classList.toggle('hidden')
  })

  target.appendChild(pill)
  pill.scrollIntoView({ behavior: 'smooth', block: 'end' })
  pending.push({ id: id || '', el: pill, name: name })
}

export function handleToolResult(id, name, output) {
  const idx = pending.findIndex(p => p.id === id)
  if (idx === -1) return
  const item = pending[idx]
  pending.splice(idx, 1)

  const status = item.el.querySelector('.tc-pill-status')
  if (status) {
    status.className = 'tc-pill-status done'
    status.innerHTML = '\u2713'
  }

  if (output) {
    let detail = item.el.querySelector('.tc-pill-detail')
    const existed = !!detail
    if (!detail) {
      detail = document.createElement('div')
      detail.className = 'tc-pill-detail hidden'
      item.el.appendChild(detail)
    }
    if (output.length <= 80) {
      detail.textContent = output
    } else {
      detail.innerHTML = '<details><summary>' + output.slice(0, 80) + '...</summary><pre>' + escapeHtml(output) + '</pre></details>'
    }
    if (!existed) {
      item.el.addEventListener('click', () => {
        detail.classList.toggle('hidden')
      })
    }
  }
  item.el.scrollIntoView({ behavior: 'smooth', block: 'end' })
}

export function clearPending() {
  pending = []
}

export function pendingCount() {
  return pending.length
}
