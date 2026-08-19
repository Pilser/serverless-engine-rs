const MAX_RECONNECT_DELAY = 30000
const BASE_RECONNECT_DELAY = 1000

export const State = {
  DISCONNECTED: 'disconnected',
  CONNECTING: 'connecting',
  CONNECTED: 'connected',
  RECONNECTING: 'reconnecting',
}

let ws = null
let state = State.DISCONNECTED
let url = ''
let callbacks = { onOpen: null, onMessage: null, onClose: null }
let reconnectTimer = null
let reconnectAttempts = 0
let listenerMap = new Map()

export function getState() { return state }

export function on(event, fn) {
  if (!listenerMap.has(event)) listenerMap.set(event, new Set())
  listenerMap.get(event).add(fn)
  return () => listenerMap.get(event)?.delete(fn)
}

export function off(event, fn) {
  listenerMap.get(event)?.delete(fn)
}

function emit(event, data) {
  listenerMap.get(event)?.forEach(fn => fn(data))
}

export function connect(wsUrl, handlers) {
  if (wsUrl) url = wsUrl
  if (handlers) callbacks = handlers

  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return

  setState(State.CONNECTING)
  ws = new WebSocket(url)
  ws.onopen = onWsOpen
  ws.onmessage = onWsMessage
  ws.onclose = onWsClose
  ws.onerror = onWsError
}

export function disconnect() {
  stopReconnect()
  reconnectAttempts = 0
  if (ws) {
    ws.onclose = null
    ws.onerror = null
    ws.close()
    ws = null
  }
  setState(State.DISCONNECTED)
}

export function send(data) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(typeof data === 'string' ? data : JSON.stringify(data))
  }
}

export function sendCancel(sessionId) {
  send(JSON.stringify({ type: 'cancel', session_id: sessionId || '' }))
}

function setState(s) {
  state = s
  emit('stateChange', s)
}

function onWsOpen() {
  setState(State.CONNECTED)
  reconnectAttempts = 0
  if (callbacks.onOpen) callbacks.onOpen()
}

function onWsMessage(e) {
  if (callbacks.onMessage) callbacks.onMessage(e)
}

function onWsClose(e) {
  setState(State.DISCONNECTED)
  if (callbacks.onClose) callbacks.onClose(e)
  scheduleReconnect()
}

function onWsError() { }

function scheduleReconnect() {
  const delay = Math.min(BASE_RECONNECT_DELAY * Math.pow(2, reconnectAttempts), MAX_RECONNECT_DELAY)
  reconnectAttempts++
  emit('reconnecting', { attempt: reconnectAttempts, delay })
  setState(State.RECONNECTING)
  reconnectTimer = setTimeout(() => {
    if (url) connect()
  }, delay)
}

function stopReconnect() {
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null }
}

window.addEventListener('online', () => {
  if (state === State.RECONNECTING || state === State.DISCONNECTED) {
    stopReconnect()
    if (url) connect()
  }
})

window.addEventListener('offline', () => {
  emit('offline')
})
