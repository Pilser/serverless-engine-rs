use topcoat::Result;
use topcoat::view::{view, Unescaped};
use topcoat::router::{Body, Response, StatusCode, route, path_param, IntoResponse};
use topcoat::context::{Cx, app_context};
use topcoat::asset::bundled_asset;

use crate::context::AppContext;
use crate::assets::{STYLESHEET, PUBLIC_WB_JS};

#[path_param]
struct WbSlug(str);

#[route(GET "/w/{wb_slug}")]
pub async fn public_wb(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let slug = path_param::<WbSlug>(cx);

    let snap = match ctx.db.public_snapshot(slug).await {
        Ok(Some(s)) => s,
        _ => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from("<h1>Page not found</h1>"))
                .unwrap());
        }
    };

    let page_title = snap
        .body_md
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "Shared Whiteboard Page".to_string());
    let favicon = ctx.settings.site.favicon.as_deref().unwrap_or("/favicon.svg");
    let brand = ctx.settings.site.brand_or_title();

    let css_url = format!("/?asset={}", bundled_asset(cx, STYLESHEET).name());
    let wb_js_url = format!("/?asset={}", bundled_asset(cx, PUBLIC_WB_JS).name());

    let pins = serde_json::from_str::<serde_json::Value>(&snap.pins)
        .unwrap_or_else(|_| serde_json::json!([]));
    let data = serde_json::json!({
        "body_md": snap.body_md,
        "pins": pins,
    });
    let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
    let data_json = data_json.replace("</script", "<\\/script");

    let view = view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>(&page_title)</title>
                <meta name="description" content="Shared whiteboard page">
                <meta property="og:type" content="website">
                <meta property="og:site_name" content=(brand)>
                <meta property="og:title" content=(&page_title)>
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
                            if (!localStorage.getItem('theme')) document.documentElement.setAttribute('data-theme', e.matches ? 'dark' : 'light');
                        });
                    }
                })();"#)) }</script>
                <script type="importmap">{ (Unescaped::new_unchecked(r#"{"imports":{"marked":"https://cdn.jsdelivr.net/npm/marked@18.0.5/lib/marked.esm.js","highlight.js":"https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/es/highlight.min.js","katex":"https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.mjs","mermaid":"https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs"}}"#)) }</script>
            </head>
            <body class="flex flex-col min-h-screen" style="font-family: var(--font-sans); color: var(--color-text); background: var(--color-bg);">
                <div id="page-root" class="w-full mx-auto flex flex-col" style="max-width: 67.5rem; min-height: 100dvh; border-left: 1px solid var(--color-border-weak); border-right: 1px solid var(--color-border-weak);">
                    <header class="flex items-center justify-between px-6 py-4" style="border-bottom: 1px solid var(--color-border-weak);">
                        <a href="/" class="no-underline text-sm font-semibold" style="color: var(--color-text-strong);">(brand)</a>
                        <span class="text-xs" style="color: var(--color-text-weak);">"Shared whiteboard page"</span>
                    </header>
                    <main class="flex-1 flex flex-col min-h-0">
                        <div id="wb-root" class="chat-markdown" style="max-width: 860px; margin: 0 auto; width: 100%; padding: 28px 20px 0;"></div>
                        <div id="wb-pins" style="max-width: 860px; margin: 0 auto; width: 100%; padding: 0 20px 60px;"></div>
                    </main>
                </div>
                <script type="application/json" id="wb-data">(data_json)</script>
                <script type="module" src=(wb_js_url)></script>
            </body>
        </html>
    };

    Ok(view.into_response(cx)?)
}