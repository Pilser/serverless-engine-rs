use topcoat::Result;
use topcoat::view::{view, Unescaped};
use topcoat::router::layout;
use topcoat::context::{Cx, app_context};
use topcoat::asset::{asset, bundled_asset};

use crate::context::AppContext;
use crate::components::whiteboard_panel::whiteboard_panel;
use crate::assets::STYLESHEET;

#[layout("/")]
pub async fn root_layout(cx: &Cx, slot: Result) -> Result {
    let ctx: &AppContext = app_context(cx);
    let title = &ctx.settings.site.title;
    let favicon = ctx.settings.site.favicon.as_deref().unwrap_or("/favicon.svg");

    // Base-relative asset URLs so an embedded frontend (e.g. behind a tenant
    // prefix like /srv/<board>/) resolves them against the document base
    // instead of the host root (served by the RootDispatchLayer via ?asset=).
    let css_url = format!("?asset={}", bundled_asset(cx, STYLESHEET).name());
    let app_js_url = format!("?asset={}", bundled_asset(cx, asset!("assets/js/chat/app.js")).name());

    // Base href = the directory of the request path, so relative URLs resolve
    // correctly whether the page is served at `/` or under a tenant prefix.
    let req_path = topcoat::router::uri(cx).path().to_string();
    let base_href = if req_path.ends_with('/') {
        req_path.clone()
    } else {
        match req_path.rfind('/') {
            Some(idx) => req_path[..=idx].to_string(),
            None => "/".to_string(),
        }
    };

    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <base href=(base_href)>
                <title>(title)</title>
                <link rel="icon" type="image/svg+xml" href=(favicon)>
                <link rel="preconnect" href="https://fonts.googleapis.com">
                <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="">
                <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
                <link rel="stylesheet" href=(css_url)>
                <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.css">
                <script>{ (Unescaped::new_unchecked(r#"(function(){
                    var saved = localStorage.getItem('theme');
                    var mq = window.matchMedia('(prefers-color-scheme: dark)');
                    if (saved) {
                        document.documentElement.setAttribute('data-theme', saved);
                    } else {
                        document.documentElement.setAttribute('data-theme', mq.matches ? 'dark' : 'light');
                        mq.addEventListener('change', function(e) {
                            if (!localStorage.getItem('theme')) {
                                document.documentElement.setAttribute('data-theme', e.matches ? 'dark' : 'light');
                            }
                        });
                    }
                })();"#)) }</script>
            </head>
            <body class="flex flex-col" style="font-family: var(--font-sans); color: var(--color-text); background: var(--color-bg); height: 100dvh; overflow: hidden;">
                <canvas id="bg-canvas"></canvas>
                <button
                    id="theme-toggle"
                    class="flex items-center justify-center cursor-pointer"
                    style="position: fixed; top: 12px; right: 12px; z-index: 60; padding: 8px; border: 1px solid var(--color-border-weak); border-radius: 6px; background: var(--color-bg-weak); color: var(--color-text-weak);"
                    aria-label="Toggle theme"
                >
                    <span id="theme-icon"></span>
                </button>
                <main class="flex-1 flex flex-col min-h-0">
                    (slot?)
                </main>
                whiteboard_panel()
                <script>{ (Unescaped::new_unchecked(format!("window.ZW_AGENT = \"{}\";", ctx.agent))) }</script>
                <script type="importmap">{ (Unescaped::new_unchecked(r#"{"imports":{"marked":"https://cdn.jsdelivr.net/npm/marked@18.0.5/lib/marked.esm.js","highlight.js":"https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/es/highlight.min.js","katex":"https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.mjs","mermaid":"https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs"}}"#)) }</script>
                <script type="module" src=(app_js_url)></script>
                <script>{ (Unescaped::new_unchecked(r#"(function(){
                    var sunSvg = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>';
                    var moonSvg = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';
                    function updateThemeIcon() {
                        var theme = document.documentElement.getAttribute('data-theme');
                        var icon = document.getElementById('theme-icon');
                        if (icon) icon.innerHTML = theme === 'dark' ? sunSvg : moonSvg;
                    }
                    var themeToggle = document.getElementById('theme-toggle');
                    if (themeToggle) {
                        updateThemeIcon();
                        themeToggle.addEventListener('click', function() {
                            var current = document.documentElement.getAttribute('data-theme');
                            var next = current === 'dark' ? 'light' : 'dark';
                            document.documentElement.setAttribute('data-theme', next);
                            localStorage.setItem('theme', next);
                            updateThemeIcon();
                        });
                    }
                })();"#)) }</script>
                <script>{ (Unescaped::new_unchecked(r#"
                var canvas = document.getElementById('bg-canvas');
                if (canvas) {
                    var ctx = canvas.getContext('2d');
                    var prefersReduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
                    function resize() {
                        var dpr = window.devicePixelRatio || 1;
                        canvas.width = window.innerWidth * dpr;
                        canvas.height = window.innerHeight * dpr;
                        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
                    }
                    function draw(time) {
                        if (!prefersReduced) requestAnimationFrame(draw);
                        var w = window.innerWidth, h = window.innerHeight;
                        ctx.clearRect(0, 0, w, h);
                        var topSpacing = 12, bottomSpacing = 48;
                        var topSize = 1.5, bottomSize = 4;
                        var topAlpha = 0.28, bottomAlpha = 0.05;
                        var isDark = document.documentElement.getAttribute('data-theme') === 'dark';
                        var dotColor = isDark
                            ? 'hsl(14, 100%, 60%)'
                            : getComputedStyle(document.documentElement).getPropertyValue('--color-bg-strong').trim() || 'hsl(0, 5%, 12%)';
                        ctx.fillStyle = dotColor;
                        var y = 4;
                        while (y < h) {
                            var t = Math.min(y / h, 1);
                            var spacing = topSpacing + (bottomSpacing - topSpacing) * t;
                            var size = topSize + (bottomSize - topSize) * t;
                            var alpha = topAlpha + (bottomAlpha - topAlpha) * t;
                            var wave = Math.sin(y * 0.018 + time * 0.0004) * 2.5;
                            ctx.globalAlpha = alpha;
                            for (var x = -spacing; x < w + spacing; x += spacing) {
                                ctx.beginPath();
                                ctx.arc(x + wave, y, size, 0, Math.PI * 2);
                                ctx.fill();
                            }
                            y += spacing;
                        }
                        ctx.globalAlpha = 1;
                        if (prefersReduced) return;
                    }
                    resize();
                    window.addEventListener('resize', resize);
                    var themeObserver = new MutationObserver(function() { resize(); });
                    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
                    requestAnimationFrame(draw);
                }
                "#)) }</script>
            </body>
        </html>
    }
}
