import { renderBody, renderPins } from './whiteboard-render.js'

const dataEl = document.getElementById('wb-data')
if (dataEl) {
  const data = JSON.parse(dataEl.textContent)
  renderBody(document.getElementById('wb-root'), data.body_md)
  renderPins(document.getElementById('wb-pins'), data.pins)
}