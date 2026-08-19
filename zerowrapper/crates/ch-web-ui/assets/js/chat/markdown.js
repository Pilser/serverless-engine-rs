import hljs from 'highlight.js'
import { marked } from 'marked'
import katex from 'katex'
import mermaid from 'mermaid'
import * as mermaidModal from './mermaid-modal.js'

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

marked.use({
  extensions: [
    {
      name: 'mathBlock',
      level: 'block',
      start(src) { return src.indexOf('$$') },
      tokenizer(src) {
        const match = /^\$\$([\s\S]+?)\$\$\n?/.exec(src)
        if (!match) return undefined
        return { type: 'mathBlock', raw: match[0], text: match[1] }
      },
      renderer(token) {
        try {
          return `<div class="math-block">${katex.renderToString(token.text, { displayMode: true, throwOnError: false })}</div>`
        } catch {
          return `<div class="math-block">${escapeHtml(token.text)}</div>`
        }
      },
    },
    {
      name: 'mathInline',
      level: 'inline',
      start(src) { return src.indexOf('$') },
      tokenizer(src) {
        const match = /^\$([^\s$][^$\n]*[^\s$])\$/.exec(src)
        if (!match) return undefined
        return { type: 'mathInline', raw: match[0], text: match[1] }
      },
      renderer(token) {
        try {
          return katex.renderToString(token.text, { displayMode: false, throwOnError: false })
        } catch {
          return escapeHtml(token.text)
        }
      },
    },
    {
      name: 'mermaid',
      level: 'block',
      start(src) { return src.indexOf('```mermaid') },
      tokenizer(src) {
        const match = /^```mermaid\s*\n([\s\S]*?)```/.exec(src)
        if (!match) return undefined
        return { type: 'mermaid', raw: match[0], text: match[1] }
      },
      renderer(token) {
        return `<div class="mermaid">${escapeHtml(token.text)}</div>`
      },
    },
  ],
})

function cssVar(name, fallback) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}

function mermaidTheme() {
  const dark = document.documentElement.getAttribute('data-theme') === 'dark'
  const bg = cssVar('--color-bg-weak', dark ? '#1a1a1a' : '#f7f7f5')
  const border = dark ? cssVar('--color-border-weak', '#3faf3f') : cssVar('--color-border', '#2f8f2f')
  return {
    startOnLoad: false,
    theme: 'base',
    themeVariables: {
      dark,
      fontFamily: cssVar('--font-mono', 'monospace'),
      background: bg,
      mainBkg: bg,
      nodeBkg: bg,
      nodeBorder: border,
      clusterBkg: cssVar('--color-bg', dark ? '#0a0a0a' : '#fcfcfb'),
      clusterBorder: cssVar('--color-border-weak', border),
      lineColor: cssVar('--color-text-weak', '#737373'),
      textColor: cssVar('--color-text', '#212121'),
      titleColor: cssVar('--color-text-strong', '#0d0d0d'),
      nodeTextColor: cssVar('--color-text', '#212121'),
      stateLabelColor: cssVar('--color-text', '#212121'),
      requirementTextColor: cssVar('--color-text', '#212121'),
      branchLabelColor: cssVar('--color-text', '#212121'),
      quadrant1TextFill: cssVar('--color-text', '#212121'),
      quadrant2TextFill: cssVar('--color-text', '#212121'),
      quadrant3TextFill: cssVar('--color-text', '#212121'),
      quadrant4TextFill: cssVar('--color-text', '#212121'),
      quadrantPointTextFill: cssVar('--color-text', '#212121'),
      quadrantXAxisTextFill: cssVar('--color-text', '#212121'),
      quadrantYAxisTextFill: cssVar('--color-text', '#212121'),
      quadrantTitleFill: cssVar('--color-text-strong', '#0d0d0d'),
      primaryColor: cssVar('--color-bg-interactive', '#ff9233'),
      primaryBorderColor: cssVar('--color-bg-interactive', '#ff9233'),
      primaryTextColor: '#ffffff',
      secondaryColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      secondaryTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      tertiaryColor: bg,
      tertiaryTextColor: cssVar('--color-text', '#212121'),
      edgeLabelBackground: cssVar('--color-bg', dark ? '#0a0a0a' : '#fcfcfb'),
      labelBackground: cssVar('--color-bg', dark ? '#0a0a0a' : '#fcfcfb'),
      actorBkg: bg,
      actorBorder: cssVar('--color-border-weak', border),
      actorTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      actorLineColor: cssVar('--color-border-weak', border),
      signalColor: cssVar('--color-text', '#212121'),
      signalTextColor: cssVar('--color-text', '#212121'),
      labelBoxBkgColor: bg,
      labelBoxBorderColor: cssVar('--color-border-weak', border),
      labelTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      loopTextColor: cssVar('--color-text-weak', '#737373'),
      noteBkgColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      noteTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      noteBorderColor: cssVar('--color-border-weak', border),
      activationBkgColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      activationBorderColor: cssVar('--color-border', '#2f8f2f'),
      sequenceNumberColor: cssVar('--color-text-inverted', '#ffffff'),
      sequenceLineColor: cssVar('--color-text-weak', '#737373'),
      taskBorder: cssVar('--color-border', '#2f8f2f'),
      taskBkg: bg,
      taskTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      taskTextOutsideColor: cssVar('--color-text-strong', '#0d0d0d'),
      taskTextLightColor: cssVar('--color-text-strong', '#0d0d0d'),
      taskTextOutsideLightColor: cssVar('--color-text-weak', '#737373'),
      taskArrowColor: cssVar('--color-bg-interactive', '#ff9233'),
      sectionBkgColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      sectionBkgColor2: bg,
      altSectionBkgColor: bg,
      todayLineColor: cssVar('--color-bg-interactive', '#ff9233'),
      criticalBkgColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      criticalBorderColor: cssVar('--color-status-error', '#e03131'),
      doneBkgColor: bg,
      doneBorderColor: cssVar('--color-border', '#2f8f2f'),
      pie1: '#ff9233',
      pie2: '#8f3f9f',
      pie3: '#2f8f2f',
      pie4: '#c98a2f',
      pie5: '#3f6fc9',
      pie6: '#9f3f6f',
      pie7: '#5f8f5f',
      pieStrokeColor: cssVar('--color-border-weak', border),
      pieOuterStrokeColor: cssVar('--color-border-weak', border),
      attributeBackgroundColorOdd: cssVar('--color-bg-weak', bg),
      attributeBackgroundColorEven: cssVar('--color-bg', dark ? '#0a0a0a' : '#fcfcfb'),
      cScale0: '#ff9233',
      cScale1: '#2f8f2f',
      cScale2: '#3f6fc9',
      cScale3: '#c98a2f',
      cScale4: '#8f3f9f',
      cScale5: '#9f3f6f',
      cScale6: '#5f8f5f',
      cScale7: '#6f8fc9',
      cScale8: '#b35f1f',
      cScale9: '#3f9f9f',
      cScale10: '#9f8f3f',
      cScale11: '#8f5f9f',
      git0: '#ff9233',
      git1: '#8f3f9f',
      git2: '#2f8f2f',
      git3: '#c98a2f',
      git4: '#3f6fc9',
      git5: '#9f3f6f',
      git6: '#5f8f5f',
      git7: '#6f8fc9',
      tagLabelBackground: cssVar('--color-bg-interactive', '#ff9233'),
      tagLabelColor: '#ffffff',
      tagLabelBorder: cssVar('--color-bg-interactive', '#ff9233'),
      tagLabelFontSize: '11px',
      commitLabelColor: cssVar('--color-text-weak', '#737373'),
      commitLabelBackground: cssVar('--color-bg', dark ? '#0a0a0a' : '#fcfcfb'),
      commitLabelFontSize: '11px',
      errorBkgColor: cssVar('--color-bg-interactive-weak', dark ? '#3f2712' : '#f9e9dc'),
      errorTextColor: cssVar('--color-text-strong', '#0d0d0d'),
      xyChart: {
        backgroundColor: bg,
        titleColor: cssVar('--color-text-strong', '#0d0d0d'),
        xAxisTitleColor: cssVar('--color-text', '#212121'),
        xAxisLabelColor: cssVar('--color-text', '#212121'),
        xAxisTickColor: cssVar('--color-border-weak', border),
        xAxisLineColor: cssVar('--color-border-weak', border),
        yAxisTitleColor: cssVar('--color-text', '#212121'),
        yAxisLabelColor: cssVar('--color-text', '#212121'),
        yAxisTickColor: cssVar('--color-border-weak', border),
        yAxisLineColor: cssVar('--color-border-weak', border),
        plotColorPalette: '#ff9233,#2f8f2f,#3f6fc9,#c98a2f,#8f3f9f,#9f3f6f',
        plotBorderColor: cssVar('--color-border', '#2f8f2f'),
        legendBackgroundColor: bg,
        legendTextColor: cssVar('--color-text', '#212121'),
      },
    },
  }
}

const mermaidSources = new Map()

function fitSvg(svg) {
  if (!svg) return
  let bbox
  try {
    bbox = svg.getBBox()
  } catch {
    return
  }
  if (!bbox || !bbox.width || !bbox.height) return
  const pad = 10
  const x = bbox.x - pad
  const y = bbox.y - pad
  const w = bbox.width + pad * 2
  const h = bbox.height + pad * 2
  svg.setAttribute('width', w)
  svg.setAttribute('height', h)
  svg.setAttribute('viewBox', `${x} ${y} ${w} ${h}`)
}

function translateParts(t) {
  const m = /translate\(\s*([\d.-]+)\s*,\s*([\d.-]+)/.exec(t || '')
  return m ? { x: parseFloat(m[1]), y: parseFloat(m[2]) } : null
}

function fixXyChart(svg) {
  const leftTitle = svg.querySelector('g.left-axis > g.title text')
  const leftLabels = svg.querySelector('g.left-axis > g.label')
  if (leftTitle && leftLabels) {
    try {
      const lb = leftLabels.getBBox()
      if (lb.width) {
        const tp = translateParts(leftTitle.getAttribute('transform'))
        if (tp) {
          const fontSize = parseFloat(leftTitle.getAttribute('font-size')) || 14
          const nx = lb.x - 10 - fontSize
          leftTitle.setAttribute('transform', `translate(${nx}, ${tp.y}) rotate(270)`)
        }
      }
    } catch {
      /* ignore */
    }
  }
  const bottomTitle = svg.querySelector('g.bottom-axis > g.title text')
  const bottomLabels = svg.querySelector('g.bottom-axis > g.label')
  if (bottomTitle && bottomLabels) {
    try {
      const lb = bottomLabels.getBBox()
      if (lb.height) {
        const tp = translateParts(bottomTitle.getAttribute('transform'))
        if (tp) {
          const ny = lb.y + lb.height + 8
          bottomTitle.setAttribute('transform', `translate(${tp.x}, ${ny})`)
        }
      }
    } catch {
      /* ignore */
    }
  }
}

function postProcessSvg(svg) {
  if (!svg) return
  fixXyChart(svg)
  fitSvg(svg)
  svg.removeAttribute('style')
}

mermaid.initialize(mermaidTheme())

function renderMermaid(el) {
  const nodes = el.querySelectorAll('div.mermaid')
  if (!nodes.length) return
  const fresh = []
  for (const node of nodes) {
    if (node.querySelector('svg')) continue
    const src = node.textContent
    if (!src) continue
    mermaidSources.set(node, src)
    fresh.push(node)
  }
  if (!fresh.length) return
  try {
    mermaid.run({ nodes: fresh })
      .then(() => fresh.forEach((node) => postProcessSvg(node.querySelector('svg'))))
      .catch(() => {})
  } catch {
    /* ignore */
  }
}

function rerenderMermaid() {
  mermaid.initialize(mermaidTheme())
  const nodes = Array.from(mermaidSources.keys()).filter((n) => n.isConnected)
  if (!nodes.length) return
  for (const node of nodes) {
    node.textContent = mermaidSources.get(node)
  }
  try {
    mermaid.run({ nodes })
      .then(() => nodes.forEach((node) => postProcessSvg(node.querySelector('svg'))))
      .catch(() => {})
  } catch {
    /* ignore */
  }
}

const mermaidThemeObserver = new MutationObserver(rerenderMermaid)
mermaidThemeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })

document.addEventListener('click', (e) => {
  if (mermaidModal.isOpen()) return
  const svg = e.target.closest('svg')
  if (!svg || !svg.closest('.mermaid')) return
  mermaidModal.open(svg)
})

export function highlight(el) {
  el.querySelectorAll('pre code').forEach((block) => {
    hljs.highlightElement(block)
  })
}

export function wrapTables(el) {
  el.querySelectorAll('table').forEach((t) => {
    if (t.parentElement && t.parentElement.classList.contains('table-wrap')) return
    const outer = document.createElement('div')
    outer.className = 'table-outer-wrap'
    const wrapper = document.createElement('div')
    wrapper.className = 'table-wrap'
    t.parentNode.insertBefore(outer, t)
    outer.appendChild(wrapper)
    wrapper.appendChild(t)

    const fade = document.createElement('div')
    fade.className = 'scroll-fade'
    outer.appendChild(fade)
    const arrow = document.createElement('div')
    arrow.className = 'scroll-arrow'
    arrow.textContent = '\u2192'
    outer.appendChild(arrow)

    let scrollTimer
    updateOverflow()
    wrapper.addEventListener('scroll', () => {
      outer.classList.add('scrolling')
      clearTimeout(scrollTimer)
      checkScrollEnd()
    })
    const ro = new ResizeObserver(updateOverflow)
    ro.observe(wrapper)

    function checkScrollEnd() {
      clearTimeout(scrollTimer)
      scrollTimer = setTimeout(() => {
        outer.classList.remove('scrolling')
        updateOverflow()
      }, 350)
    }
    function updateOverflow() {
      if (wrapper.scrollWidth > wrapper.clientWidth) {
        outer.classList.add('overflow')
        const atEnd = wrapper.scrollLeft + wrapper.clientWidth >= wrapper.scrollWidth - 1
        outer.classList.toggle('scrolled-end', atEnd)
      } else {
        outer.classList.remove('overflow', 'scrolled-end', 'scrolling')
      }
    }
  })
}

function hasWidgetMarker(root) {
  if (!root.matches) return false
  if (root.matches('script')) return false
  if (root.matches('style')) return true
  return root.querySelector('style') !== null && root.querySelector('script') === null
}

function isolateWidgets(el) {
  Array.from(el.children).forEach(child => {
    if (!hasWidgetMarker(child)) return
    const wrap = document.createElement('div')
    wrap.className = 'wb-widget'
    el.insertBefore(wrap, child)
    const root = wrap.attachShadow({ mode: 'open' })
    root.appendChild(child)
  })
}

function collectScripts(root) {
  const scripts = Array.from(root.querySelectorAll('script'))
  root.querySelectorAll('*').forEach(node => {
    if (node.shadowRoot) scripts.push(...collectScripts(node.shadowRoot))
  })
  return scripts
}

function executeScripts(el) {
  collectScripts(el).forEach(old => {
    const s = document.createElement('script')
    for (const attr of old.attributes) s.setAttribute(attr.name, attr.value)
    s.textContent = old.textContent
    old.replaceWith(s)
  })
}

export function render(el, html, runScripts = false) {
  html = html.replace(/<img\s/gi, '<img loading="lazy" ')
  html = html.replace(/<iframe\s/gi, '<iframe loading="lazy" ')
  html = html.replace(/<video\s/gi, '<video preload="none" ')
  el.innerHTML = html
  if (runScripts) {
    const hasScript = el.querySelector('script') !== null
    if (!hasScript) isolateWidgets(el)
    executeScripts(el)
  }
  wrapTables(el)
  highlight(el)
  renderMermaid(el)
}
