use zw_db::filters::{Agg, parse_filter};

#[tokio::test]
async fn in_filter_matches() {
    let dir = std::env::temp_dir().join(format!("zwdb_test_in_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = zw_db::DbPool::open(db_path.to_str().unwrap()).await.unwrap();
    pool.migrate().await.unwrap();
    pool.migrate_session().await.unwrap();

    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    for brand in ["Dell", "Apple", "Lenovo"] {
        let payload = serde_json::json!({"brand": brand, "price": 100});
        pool.srv_record_insert(&board, &payload, None, false).await.unwrap();
    }

    let f = parse_filter(&serde_json::json!({"brand": {"in": ["Apple", "Dell"]}})).unwrap();
    let rows = pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 2, "in should match Dell + Apple, got {:?}", rows);

    let f = parse_filter(&serde_json::json!({"price": {"gte": 100}})).unwrap();
    let rows = pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 3, "gte should match all");

    let f = parse_filter(&serde_json::json!({"brand": {"contains": "e"}})).unwrap();
    let rows = pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 3, "contains 'e' should match Dell + Apple + Lenovo");

    let agg = pool.srv_record_aggregate(&board, &[], Agg::Count, None, None).await.unwrap();
    assert_eq!(agg[0]["value"], serde_json::json!(3));

    let agg = pool.srv_record_aggregate(&board, &[], Agg::Sum, Some("$.price"), Some("$.brand")).await.unwrap();
    assert_eq!(agg.len(), 3);

    std::fs::remove_dir_all(&dir).ok();
}
