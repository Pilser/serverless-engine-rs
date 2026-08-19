import { els } from './dom.js'

const STORAGE_EMAIL = 'zerowrapper_email'
const STORAGE_SESSION = 'zerowrapper_session'
const STORAGE_NAME = 'zerowrapper_name'
const STORAGE_ROLE = 'zerowrapper_role'
const STORAGE_ROLES = 'zerowrapper_roles'

export const state = {
  email: '',
  sessionId: '',
  name: '',
  role: '',
  roles: [],
}

export const agent = (window.ZW_AGENT || '').toString()

export function buildSessionId(email) {
  const sanitized = String(email || '').replace(/[^a-zA-Z0-9_.@-]/g, '_')
  return 'web.' + agent + '.' + sanitized
}

export function loadFromStorage() {
  state.email = localStorage.getItem(STORAGE_EMAIL) || ''
  state.sessionId = localStorage.getItem(STORAGE_SESSION) || ''
  state.name = localStorage.getItem(STORAGE_NAME) || ''
  state.role = localStorage.getItem(STORAGE_ROLE) || ''
  try {
    state.roles = JSON.parse(localStorage.getItem(STORAGE_ROLES) || '[]')
  } catch {
    state.roles = []
  }
}

// Auto-login identity supplied by the embedding host (the engine SPA) via
// ?email=&name=&role=&roles= on the iframe URL. Only applied when no session is
// stored yet so a returning user keeps their own session.
export function loadFromQuery() {
  const qs = new URLSearchParams(location.search)
  const email = (qs.get('email') || '').trim()
  if (!email) return
  if (localStorage.getItem(STORAGE_EMAIL)) return
  state.email = email
  state.sessionId = buildSessionId(email)
  state.name = (qs.get('name') || '').trim()
  state.role = (qs.get('role') || '').trim()
  const roles = (qs.get('roles') || '').split(',').map((r) => r.trim()).filter(Boolean)
  state.roles = roles.length > 0 ? roles : (state.role ? [state.role] : [])
  saveToStorage()
}

export function saveToStorage() {
  if (state.email) localStorage.setItem(STORAGE_EMAIL, state.email)
  if (state.sessionId) localStorage.setItem(STORAGE_SESSION, state.sessionId)
  if (state.name) localStorage.setItem(STORAGE_NAME, state.name)
  if (state.role) localStorage.setItem(STORAGE_ROLE, state.role)
  if (state.roles && state.roles.length > 0) localStorage.setItem(STORAGE_ROLES, JSON.stringify(state.roles))
}

export function clearFromStorage() {
  localStorage.removeItem(STORAGE_EMAIL)
  localStorage.removeItem(STORAGE_SESSION)
  localStorage.removeItem(STORAGE_NAME)
  localStorage.removeItem(STORAGE_ROLE)
  localStorage.removeItem(STORAGE_ROLES)
}

export function showForm() {
  els.authSection.classList.remove('hidden')
  els.userBadge.classList.add('hidden')
}

export function showBadge() {
  els.authSection.classList.add('hidden')
  els.userBadge.classList.remove('hidden')
  els.badgeDisplay.textContent = state.name || state.email
  const roles = state.roles && state.roles.length > 0 ? state.roles : (state.role ? [state.role] : [])
  const roleText = roles.length > 1 ? `${roles[0]} +${roles.length - 1}` : (roles[0] || '')
  if (els.badgeSub) els.badgeSub.textContent = [roleText, state.email].filter(Boolean).join(' · ')
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
