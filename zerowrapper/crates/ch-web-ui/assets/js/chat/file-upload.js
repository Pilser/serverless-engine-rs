export const uploadState = {
  pending: [],
}

export async function uploadFile(file) {
  try {
    const formData = new FormData()
    formData.append('file', file)

    const res = await fetch('?api=upload', {
      method: 'POST',
      headers: { 'X-Filename': file.name },
      body: file,
    })
    const data = await res.json()
    if (data.error) throw new Error(data.error)
    return data
  } catch (err) {
    throw err
  }
}

export function createAttachmentChip(fileInfo) {
  const chip = document.createElement('div')
  chip.className = 'attachment-chip'
  chip.style.cssText = 'display: inline-flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 6px; border: 1px solid var(--color-border-weak); background: var(--color-bg-weak); font-size: 0.75rem; margin-bottom: 4px;'
  chip.innerHTML = `
    <span style="color: var(--color-text-strong);">${fileInfo.filename}</span>
    <span style="color: var(--color-text-weak); font-size: 0.625rem;">${formatSize(fileInfo.size)}</span>
    <button class="remove-attachment" style="background: none; border: none; cursor: pointer; color: var(--color-status-error); font-size: 0.875rem; padding: 0; line-height: 1;">&times;</button>
  `
  chip.querySelector('.remove-attachment').addEventListener('click', () => {
    chip.remove()
    uploadState.pending = uploadState.pending.filter(a => a.path !== fileInfo.path)
  })
  return chip
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}
