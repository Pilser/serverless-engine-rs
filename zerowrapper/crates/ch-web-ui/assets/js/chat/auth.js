import { els } from './dom.js'

const STORAGE_EMAIL = 'zerowrapper_email'
const STORAGE_SESSION = 'zerowrapper_session'

export const state = {
  email: '',
  sessionId: '',
}

export const agent = (window.ZW_AGENT || '').toString()

export function buildSessionId(email) {
  const sanitized = String(email || '').replace(/[^a-zA-Z0-9_.@-]/g, '_')
  return 'web.' + agent + '.' + sanitized
}

export function loadFromStorage() {
  state.email = localStorage.getItem(STORAGE_EMAIL) || ''
  state.sessionId = localStorage.getItem(STORAGE_SESSION) || ''
}

export function saveToStorage() {
  if (state.email) localStorage.setItem(STORAGE_EMAIL, state.email)
  if (state.sessionId) localStorage.setItem(STORAGE_SESSION, state.sessionId)
}

export function clearFromStorage() {
  localStorage.removeItem(STORAGE_EMAIL)
  localStorage.removeItem(STORAGE_SESSION)
}

export function showForm() {
  els.authSection.classList.remove('hidden')
  els.userBadge.classList.add('hidden')
}

export function showBadge() {
  els.authSection.classList.add('hidden')
  els.userBadge.classList.remove('hidden')
  els.badgeDisplay.textContent = state.email
}

export function scrollToAuth() {
  els.authSection.scrollIntoView({ behavior: 'smooth', block: 'center' })
  els.emailInput.focus()
  els.authSection.style.transition = 'box-shadow 0.3s'
  els.authSection.style.boxShadow = '0 0 0 3px var(--color-bg-interactive)'
  setTimeout(() => { els.authSection.style.boxShadow = 'none' }, 2000)
}

export function showAuthError(msg) {
  els.authError.textContent = msg
  els.authError.classList.remove('hidden')
}

export function hideAuthError() {
  els.authError.classList.add('hidden')
}
