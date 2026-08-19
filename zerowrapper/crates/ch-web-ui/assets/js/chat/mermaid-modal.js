let modal = null
let stage = null
let canvas = null
let scale = 1
let tx = 0
let ty = 0
let fit = 1
let diagramW = 600
let diagramH = 400
let drag = null
let pointers = new Map()
let pinchDist = 0

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v))
}

function ensure() {
  if (modal) return
  modal = document.createElement('div')
  modal.id = 'mermaid-modal'
  modal.innerHTML =
    '<button class="mm-close" title="Close">&times;</button>' +
    '<div class="mm-controls">' +
    '<button class="mm-btn" data-zoom="out" title="Zoom out">&minus;</button>' +
    '<button class="mm-btn" data-zoom="in" title="Zoom in">+</button>' +
    '<button class="mm-btn" data-zoom="reset" title="Reset view">&#10227;</button>' +
    '</div>' +
    '<div class="mm-stage"><div class="mm-canvas"></div></div>'
  document.body.appendChild(modal)
  stage = modal.querySelector('.mm-stage')
  canvas = modal.querySelector('.mm-canvas')
  modal.querySelector('.mm-close').addEventListener('click', close)
  modal.addEventListener('click', (e) => { if (e.target === modal) close() })
  modal.querySelectorAll('.mm-btn').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation()
      const z = btn.dataset.zoom
      if (z === 'in') zoomAt(1.4, stage.clientWidth / 2, stage.clientHeight / 2)
      else if (z === 'out') zoomAt(1 / 1.4, stage.clientWidth / 2, stage.clientHeight / 2)
      else resetView()
    })
  })
  stage.addEventListener('wheel', (e) => {
    e.preventDefault()
    const rect = stage.getBoundingClientRect()
    zoomAt(e.deltaY < 0 ? 1.12 : 1 / 1.12, e.clientX - rect.left, e.clientY - rect.top)
  }, { passive: false })
  stage.addEventListener('pointerdown', onDown)
  stage.addEventListener('pointermove', onMove)
  stage.addEventListener('pointerup', onUp)
  stage.addEventListener('pointercancel', onUp)
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') close() })
}

function apply() {
  canvas.style.transform = `translate(${tx}px, ${ty}px) scale(${scale})`
}

function zoomAt(factor, px, py) {
  const ns = clamp(scale * factor, 0.15, 10)
  tx = px - ((px - tx) * (ns / scale))
  ty = py - ((py - ty) * (ns / scale))
  scale = ns
  apply()
}

function resetView() {
  scale = fit
  tx = (stage.clientWidth - diagramW * fit) / 2
  ty = (stage.clientHeight - diagramH * fit) / 2
  apply()
}

function onDown(e) {
  if (e.target.closest('.mm-btn, .mm-close')) return
  pointers.set(e.pointerId, { x: e.clientX, y: e.clientY })
  if (pointers.size === 1) {
    drag = { x: e.clientX, y: e.clientY, tx, ty }
    stage.setPointerCapture(e.pointerId)
  } else if (pointers.size === 2) {
    drag = null
    const [a, b] = Array.from(pointers.values())
    pinchDist = Math.hypot(a.x - b.x, a.y - b.y)
  }
}

function onMove(e) {
  if (!pointers.has(e.pointerId)) return
  pointers.set(e.pointerId, { x: e.clientX, y: e.clientY })
  if (pointers.size === 2) {
    const [a, b] = Array.from(pointers.values())
    const d = Math.hypot(a.x - b.x, a.y - b.y)
    if (pinchDist > 0) {
      const rect = stage.getBoundingClientRect()
      zoomAt(d / pinchDist, (a.x + b.x) / 2 - rect.left, (a.y + b.y) / 2 - rect.top)
    }
    pinchDist = d
  } else if (drag) {
    tx = drag.tx + (e.clientX - drag.x)
    ty = drag.ty + (e.clientY - drag.y)
    apply()
  }
}

function onUp(e) {
  pointers.delete(e.pointerId)
  if (pointers.size < 2) pinchDist = 0
  if (pointers.size === 0) drag = null
}

export function open(svg) {
  ensure()
  const clone = svg.cloneNode(true)
  clone.removeAttribute('style')
  clone.style.maxWidth = 'none'
  clone.style.maxHeight = 'none'
  clone.style.overflow = 'visible'
  canvas.innerHTML = ''
  canvas.appendChild(clone)
  canvas.style.transform = ''
  modal.classList.add('active')
  document.body.style.overflow = 'hidden'
  const rect = clone.getBoundingClientRect()
  diagramW = rect.width || stage.clientWidth
  diagramH = rect.height || stage.clientHeight
  fit = Math.min(stage.clientWidth / diagramW, stage.clientHeight / diagramH, 1)
  resetView()
}

export function close() {
  if (!modal) return
  modal.classList.remove('active')
  document.body.style.overflow = ''
}

export function isOpen() {
  return !!modal && modal.classList.contains('active')
}
