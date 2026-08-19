use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, path_param};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

#[path_param]
struct MaterialRef(str);

#[path_param]
struct MaterialFile(str);

fn json_response(status: StatusCode, v: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(v.to_string()))
        .unwrap()
}

#[route(GET "/materials/{material}")]
pub async fn materials_index(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let material = path_param::<MaterialRef>(cx);
    let (tpl, ver) = match zw_db::templates::parse_version_sid(material) {
        Some((t, v)) => (t, v),
        None => (material.to_string(), 0),
    };
    let store = match ctx.materials() {
        Some(s) => s,
        None => return Ok(json_response(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"error": "materials unavailable"}))),
    };
    let ver = match store.head(&tpl).await {
        Ok(v) => if ver > 0 { ver } else { v },
        Err(_) => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "template not found"}))),
    };
    if !store.version_exists(&tpl, ver).await.unwrap_or(false) {
        return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "version not found"})));
    }
    let files = match zw_materials::store::list_files(&store.files_dir(&tpl, ver)) {
        Ok(f) => f,
        Err(e) => return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": format!("{e}")}))),
    };
    Ok(json_response(StatusCode::OK, serde_json::json!({
        "template_id": tpl,
        "version": ver,
        "base": format!("/materials/{tpl}@{ver}"),
        "files": files.into_iter().map(|(p, s)| serde_json::json!({"path": p, "size": s})).collect::<Vec<_>>(),
    })))
}

#[route(GET "/materials/{material}/{file}")]
pub async fn materials_file(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let material = path_param::<MaterialRef>(cx);
    let file = path_param::<MaterialFile>(cx);
    if file.contains("..") || file.contains('/') || file.contains('\\') {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("bad request"))
            .unwrap());
    }
    let (tpl, ver) = match zw_db::templates::parse_version_sid(material) {
        Some((t, v)) => (t, v),
        None => (material.to_string(), 0),
    };
    let store = match ctx.materials() {
        Some(s) => s,
        None => return Ok(Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from("materials unavailable"))
            .unwrap()),
    };
    let ver = match store.head(&tpl).await {
        Ok(v) => if ver > 0 { ver } else { v },
        Err(_) => return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap()),
    };
    if !store.version_exists(&tpl, ver).await.unwrap_or(false) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap());
    }
    let path = store.files_dir(&tpl, ver).join(file);
    let data = match tokio::fs::read(&path).await {
        Ok(d) => d,
        Err(_) => return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap()),
    };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime.as_ref())
        .header("cache-control", "public, max-age=86400")
        .body(Body::from(data))
        .unwrap())
}
