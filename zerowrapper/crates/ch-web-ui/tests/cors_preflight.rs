use topcoat::router::{Router, Request, Path as RoutPath, RouterBuilderDiscoverExt, tower::TowerLayer};
use topcoat::router::{Body};

#[tokio::test]
async fn specific_post_route_answers_cors_preflight() {
    let cors = tower_http::cors::CorsLayer::permissive();
    let router = Router::builder()
        .layer(TowerLayer::new(RoutPath::new("/api"), cors))
        .discover()
        .build();

    let request = Request::builder()
        .method("OPTIONS")
        .uri("/api/srv/b_pzd2a674xokd0000/submit")
        .header("Origin", "http://pilser-ai.pilserlabs.com")
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "content-type")
        .body(Body::empty())
        .unwrap();

    let response = router.handle(request).await;
    let status = response.status().as_u16();
    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap_or_default().to_string());
    assert!(matches!(status, 200 | 204), "preflight must not be 405 (got {status})");
    assert_eq!(allow_origin.as_deref(), Some("*"), "missing allow-origin: {allow_origin:?}");
    assert!(
        response.headers().contains_key("access-control-allow-methods"),
        "missing allow-methods"
    );

    let plain = Request::builder()
        .method("GET")
        .uri("/api/srv/b_pzd2a674xokd0000/records")
        .header("Origin", "http://pilser-ai.pilserlabs.com")
        .body(Body::empty())
        .unwrap();
    let plain = router.handle(plain).await;
    let pstatus = plain.status().as_u16();
    assert!(
        !matches!(pstatus, 405 | 500),
        "plain request must route normally, got {pstatus}"
    );
}