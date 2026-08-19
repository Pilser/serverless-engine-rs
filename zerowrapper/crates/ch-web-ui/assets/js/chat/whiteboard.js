import * as network from './network.js'
import { renderBoard } from './whiteboard-render.js'

const SWIPE_THRESHOLD = 40

let sessionId = ''
let mode = 'closed'
let panel = null
let body = null
let pins = null
let down = null
let up = null
let dragStartY = null
let dragging = false
let currentState = null
let currentPage = 0
let syncHideTimer = null

function getEl(id) { return document.getElementById(id) }

function getInputTop() {
  const input = activeInput()
  if (!input) return window.innerHeight * 0.6
  return input.getBoundingClientRect().top
}

function applyMode() {
  if (!panel) return
  panel.style.transform = ''
  panel.classList.remove('wb-closed', 'wb-peek', 'wb-full')
  if (mode === 'peek') {
    const offset = -(window.innerHeight - getInputTop())
    panel.style.setProperty('--wb-peek-offset', offset + 'px')
    panel.classList.add('wb-peek')
  } else if (mode === 'full') {
    panel.classList.add('wb-full')
  } else {
    panel.classList.add('wb-closed')
  }
}

function setMode(next) {
  if (next === mode) return
  mode = next
  applyMode()
}

function cycleDown() {
  if (mode === 'closed') setMode('peek')
  else if (mode === 'peek') setMode('full')
}

function cycleUp() {
  if (mode === 'full') setMode('peek')
  else if (mode === 'peek') setMode('closed')
}

function activeInput() {
  const input = getEl('chat-input')
  if (!input) return null
  return input
}

function positionArrows() {
  if (!down || !up) return
  const input = activeInput()
  if (!input) return
  const r = input.getBoundingClientRect()
  const cx = r.left + r.width / 2 + 55
  const top = r.top - 48
  const pos = (btn, x) => {
    btn.style.left = (cx + x) + 'px'
    btn.style.top = top + 'px'
    btn.style.right = 'auto'
    btn.style.bottom = 'auto'
  }
  pos(down, -40)
  pos(up, 6)
}

function wireButton(btn, onTap) {
  if (!btn) return
  btn.addEventListener('click', (e) => {
    if (!dragging) onTap()
  })
  btn.addEventListener('pointerdown', (e) => {
    dragStartY = e.clientY
    dragging = true
    btn.setPointerCapture(e.pointerId)
  })
  btn.addEventListener('pointermove', (e) => {
    if (!dragging || dragStartY === null || !panel) return
    const dy = e.clientY - dragStartY
    if (mode === 'closed') panel.style.transform = `translateY(${Math.min(dy, 120)}px)`
    else if (mode === 'full') panel.style.transform = `translateY(${Math.max(dy, -120)}px)`
  })
  const endDrag = (e) => {
    if (!dragging) return
    const dy = e.clientY - dragStartY
    dragStartY = null
    dragging = false
    if (dy > SWIPE_THRESHOLD) cycleDown()
    else if (dy < -SWIPE_THRESHOLD) cycleUp()
    else applyMode()
  }
  btn.addEventListener('pointerup', endDrag)
  btn.addEventListener('pointercancel', endDrag)
}

export function init() {
  panel = getEl('whiteboard-panel')
  body = getEl('whiteboard-body')
  pins = getEl('whiteboard-pins')
  down = getEl('whiteboard-down')
  up = getEl('whiteboard-up')

  if (!panel) return

  wireButton(down, cycleDown)
  wireButton(up, cycleUp)

  positionArrows()
  window.addEventListener('resize', positionArrows)
  window.addEventListener('scroll', positionArrows, { passive: true })
  document.addEventListener('visibilitychange', () => { if (!document.hidden) positionArrows() })
  new MutationObserver(positionArrows)
    .observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] })

  const prev = getEl('wb-prev')
  const next = getEl('wb-next')
  const share = getEl('wb-share')
  const copyLink = getEl('wb-copy-link')
  if (prev) prev.addEventListener('click', () => { currentPage--; renderCurrent() })
  if (next) next.addEventListener('click', () => { currentPage++; renderCurrent() })
  if (share) share.addEventListener('click', () => {
    const p = currentPageData()
    if (!p || !p.public_slug) return
    openPublicUrl(p.public_slug)
  })
  if (copyLink) copyLink.addEventListener('click', () => {
    const p = currentPageData()
    if (!p || !p.public_slug) return
    showLinkPop(p.public_slug)
    copyPublicUrl(p.public_slug)
  })
  document.addEventListener('click', (e) => {
    const pop = getEl('wb-link-pop')
    if (pop && !pop.classList.contains('hidden') && !pop.contains(e.target) &&
        e.target.id !== 'wb-copy-link' && e.target.id !== 'wb-share') {
      pop.classList.add('hidden')
    }
  })

  applyMode()
}

function publicUrl(slug) {
  return location.origin + '/w/' + slug
}

function openPublicUrl(slug) {
  window.open(publicUrl(slug), '_blank', 'noopener,noreferrer')
}

function showLinkPop(slug) {
  const pop = getEl('wb-link-pop')
  const input = getEl('wb-link-input')
  const copied = getEl('wb-link-copied')
  if (!pop || !input) return
  input.value = publicUrl(slug)
  input.select()
  if (copied) copied.classList.add('hidden')
  pop.classList.remove('hidden')
}

function copyPublicUrl(slug) {
  const url = publicUrl(slug)
  const done = () => {
    const copied = getEl('wb-link-copied')
    if (!copied) return
    copied.classList.remove('hidden')
    setTimeout(() => copied.classList.add('hidden'), 2000)
  }
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(url).then(done).catch(() => done())
  } else {
    const input = getEl('wb-link-input')
    if (input) { input.select(); try { document.execCommand('copy') } catch (e) {} }
    done()
  }
}

export function sync() {
  if (!sessionId) return
  const spin = getEl('wb-nav-spinner')
  if (spin) spin.classList.remove('hidden')
  clearTimeout(syncHideTimer)
  syncHideTimer = setTimeout(() => { if (spin) spin.classList.add('hidden') }, 5000)
  network.send(JSON.stringify({ type: 'whiteboard_sync', session_id: sessionId }))
}

export function setSession(sid) {
  if (!sid) return
  if (sid !== sessionId) {
    sessionId = sid
    sync()
  }
}

function renderCurrent() {
  const label = getEl('wb-page-label')
  const badge = getEl('wb-lock-badge')
  const share = getEl('wb-share')
  const copyLink = getEl('wb-copy-link')
  const prev = getEl('wb-prev')
  const next = getEl('wb-next')
  if (!currentState || !Array.isArray(currentState.pages) || currentState.pages.length === 0) {
    renderBoard(body, pins, { body_md: '', pins: [] })
    if (label) label.textContent = 'Page 1 of 1'
    if (badge) badge.classList.add('hidden')
    if (share) share.classList.add('hidden')
    if (copyLink) copyLink.classList.add('hidden')
    if (prev) prev.disabled = true
    if (next) next.disabled = true
    return
  }
  const pages = currentState.pages
  currentPage = Math.max(0, Math.min(currentPage, pages.length - 1))
  const page = currentPageData()
  renderBoard(body, pins, page)
  if (label) label.textContent = `Page ${currentPage + 1} of ${pages.length}`
  if (prev) prev.disabled = currentPage <= 0
  if (next) next.disabled = currentPage >= pages.length - 1
  const active = currentState.locked && page && page.page_no === currentState.active_page
  const isPub = !!(page && page.public_slug)
  if (badge) badge.classList.toggle('hidden', !active)
  if (share) share.classList.toggle('hidden', !isPub)
  if (copyLink) copyLink.classList.toggle('hidden', !isPub)
  const pop = getEl('wb-link-pop')
  if (pop) pop.classList.add('hidden')
}

function currentPageData() {
  return (currentState && currentState.pages && currentState.pages[currentPage]) || null
}

export function setBoard(data) {
  const spin = getEl('wb-nav-spinner')
  if (spin) spin.classList.add('hidden')
  clearTimeout(syncHideTimer)
  if (!data) return
  if (data.session_id) sessionId = data.session_id
  if (data.state) {
    currentState = data.state
    const pages = data.state.pages || []
    let idx = pages.findIndex(p => p.page_no === data.state.active_page)
    if (idx < 0) idx = 0
    currentPage = Math.max(0, Math.min(idx, pages.length - 1))
    renderCurrent()
    return
  }
  currentState = null
  renderBoard(body, pins, data)
  const label = getEl('wb-page-label')
  const badge = getEl('wb-lock-badge')
  const share = getEl('wb-share')
  const copyLink = getEl('wb-copy-link')
  const pop = getEl('wb-link-pop')
  const prev = getEl('wb-prev')
  const next = getEl('wb-next')
  if (label) label.textContent = 'Page 1 of 1'
  if (badge) badge.classList.add('hidden')
  if (share) share.classList.add('hidden')
  if (copyLink) copyLink.classList.add('hidden')
  if (pop) pop.classList.add('hidden')
  if (prev) prev.disabled = true
  if (next) next.disabled = true
}
