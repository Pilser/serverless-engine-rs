const LOADING_CLASS = 'btn-loading'

export async function fetchWithTimeout(url, options = {}, timeoutMs = 15000) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    return await fetch(url, { ...options, signal: controller.signal })
  } catch (err) {
    if (err.name === 'AbortError') {
      const e = new Error('timeout')
      e.code = 'TIMEOUT'
      throw e
    }
    throw err
  } finally {
    clearTimeout(timer)
  }
}

export function setButtonLoading(btn, loading, loadingText) {
  if (!btn) return
  if (loading) {
    btn.dataset.originalLabel = btn.dataset.originalLabel || btn.textContent
    btn.disabled = true
    btn.classList.add(LOADING_CLASS)
    if (loadingText) btn.textContent = loadingText
  } else {
    btn.disabled = false
    btn.classList.remove(LOADING_CLASS)
    if (btn.dataset.originalLabel) {
      btn.textContent = btn.dataset.originalLabel
      delete btn.dataset.originalLabel
    }
  }
}