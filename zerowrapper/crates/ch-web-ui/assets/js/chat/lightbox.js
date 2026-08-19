let lightbox = null
let loadingEl = null
let images = []
let currentIndex = 0
let touchStartX = 0

function ensureLightbox() {
  if (!lightbox) {
    lightbox = document.createElement('div')
    lightbox.id = 'lightbox'
    lightbox.innerHTML = '<button class="lb-close">&times;</button><button class="lb-prev">&#10094;</button><button class="lb-next">&#10095;</button>'
    loadingEl = document.createElement('div')
    loadingEl.className = 'lb-loading'
    loadingEl.textContent = 'Loading...'
    lightbox.appendChild(loadingEl)
    document.body.appendChild(lightbox)
    lightbox.addEventListener('click', (e) => {
      if (e.target === lightbox || e.target.classList.contains('lb-close')) close()
    })
    lightbox.addEventListener('click', (e) => {
      const btn = e.target.closest('.lb-prev, .lb-next')
      if (!btn) return
      btn.classList.contains('lb-prev') ? navigate(-1) : navigate(1)
    })
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') close()
      if (e.key === 'ArrowLeft') navigate(-1)
      if (e.key === 'ArrowRight') navigate(1)
    })
    lightbox.addEventListener('touchstart', (e) => { touchStartX = e.changedTouches[0].screenX }, { passive: true })
    lightbox.addEventListener('touchend', (e) => {
      const delta = touchStartX - e.changedTouches[0].screenX
      if (delta > 50) navigate(1)
      else if (delta < -50) navigate(-1)
    }, { passive: true })
  }
  return lightbox
}

function close() {
  if (lightbox) {
    lightbox.classList.remove('active')
    document.body.style.overflow = ''
    const img = lightbox.querySelector('img')
    if (img) { img.onload = null; img.onerror = null }
  }
}

function navigate(dir) {
  const next = currentIndex + dir
  if (next < 0 || next >= images.length) return
  currentIndex = next
  loadImage(images[currentIndex])
  updateNavButtons()
}

function loadImage(src) {
  const lb = ensureLightbox()
  let img = lb.querySelector('img')
  if (!img) {
    img = document.createElement('img')
    lb.appendChild(img)
  }
  img.style.display = 'none'
  img.onload = null
  img.onerror = null
  if (loadingEl) loadingEl.style.display = 'block'
  img.onload = () => {
    img.style.display = 'block'
    if (loadingEl) loadingEl.style.display = 'none'
  }
  img.onerror = () => {
    if (loadingEl) loadingEl.textContent = 'Failed to load image'
    setTimeout(() => { if (loadingEl) loadingEl.textContent = 'Loading...' }, 2000)
  }
  lb.classList.add('active')
  document.body.style.overflow = 'hidden'
  img.src = src
}

function updateNavButtons() {
  if (!lightbox) return
  const prev = lightbox.querySelector('.lb-prev')
  const next = lightbox.querySelector('.lb-next')
  if (prev) prev.style.display = currentIndex <= 0 ? 'none' : 'flex'
  if (next) next.style.display = currentIndex >= images.length - 1 ? 'none' : 'flex'
}

export function init(messagesEl) {
  messagesEl.addEventListener('click', (e) => {
    if (e.target.tagName === 'IMG' && e.target.closest('.chat-markdown')) {
      const container = e.target.closest('.chat-markdown')
      const imgs = Array.from(container.querySelectorAll('img'))
      images = imgs.map(img => img.src)
      currentIndex = imgs.indexOf(e.target)
      if (currentIndex === -1) currentIndex = 0
      ensureLightbox()
      updateNavButtons()
      loadImage(images[currentIndex])
    }
  })
}
