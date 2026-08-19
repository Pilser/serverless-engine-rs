use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, path_param};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

#[path_param]
struct FileHash(str);

#[route(GET "/favicon.svg")]
pub async fn favicon(cx: &Cx) -> Result<Response> {
    static_file(cx, "favicon.svg", "image/svg+xml").await
}

#[route(GET "/logo.svg")]
pub async fn logo(cx: &Cx) -> Result<Response> {
    static_file(cx, "logo.svg", "image/svg+xml").await
}

#[route(GET "/robots.txt")]
pub async fn robots(cx: &Cx) -> Result<Response> {
    static_file(cx, "robots.txt", "text/plain").await
}

#[route(GET "/sitemap-index.xml")]
pub async fn sitemap_index(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let url = &ctx.settings.site.url;
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>{}/sitemap.xml</loc>
  </sitemap>
</sitemapindex>
"#,
        url
    );
    xml_response(body)
}

#[route(GET "/sitemap.xml")]
pub async fn sitemap(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let url = &ctx.settings.site.url;
    let mut entries = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>{url}/</loc>
    <changefreq>weekly</changefreq>
    <priority>1.0</priority>
  </url>
  <url>
    <loc>{url}/chat</loc>
    <changefreq>weekly</changefreq>
    <priority>0.8</priority>
  </url>
"#
    );
    for branch in ctx
        .settings
        .business
        .as_ref()
        .map(|b| &b.branches)
        .into_iter()
        .flatten()
    {
        if let Some(burl) = &branch.url {
            entries.push_str(&format!(
                r#"  <url>
    <loc>{burl}/</loc>
    <changefreq>weekly</changefreq>
    <priority>0.9</priority>
  </url>
"#
            ));
        }
    }
    entries.push_str("</urlset>\n");
    xml_response(entries)
}

fn xml_response(body: String) -> Result<Response> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/xml; charset=utf-8")
        .header("cache-control", "public, max-age=3600")
        .body(Body::from(body))
        .unwrap())
}

async fn static_file(cx: &Cx, name: &str, mime: &str) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let path = ctx.config_dir.join(name);

    let data = match tokio::fs::read(&path).await {
        Ok(d) => d,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not found"))
                .unwrap());
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .header("cache-control", "public, max-age=3600")
        .body(Body::from(data))
        .unwrap())
}

#[route(GET "/files/{file_hash}")]
pub async fn files(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let hash = path_param::<FileHash>(cx);

    let meta = match ctx.cache.get_meta(hash) {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("File not found"))
                .unwrap());
        }
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("Cache error: {}", e)))
                .unwrap());
        }
    };

    let data = match tokio::fs::read(&meta.original_path).await {
        Ok(d) => d,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("Error reading file: {}", e)))
                .unwrap());
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", &meta.mime_type)
        .header("cache-control", "public, max-age=3600")
        .body(Body::from(data))
        .unwrap())
}