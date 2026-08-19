use turso::params::Params;
use turso::Value;

#[tokio::test]
async fn fts_index_works() {
    let dir = std::env::temp_dir().join(format!("zwdb_test_fts_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = zw_db::DbPool::open(db_path.to_str().unwrap()).await.unwrap();
    pool.migrate().await.unwrap();
    pool.migrate_session().await.unwrap();

    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_record_insert(&board, &serde_json::json!({"name": "Dell XPS 13", "brand": "Dell", "body": "the quick brown fox"}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &serde_json::json!({"name": "MacBook Air", "brand": "Apple", "body": "lightweight laptop"}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &serde_json::json!({"name": "ThinkPad", "brand": "Lenovo", "body": "durable workhorse"}), None, false).await.unwrap();

    let conn = pool.conn().await;
    let mut rows = conn
        .query(
            "SELECT seq FROM wb_records WHERE board_id = ?1 AND fts_match(payload, ?2)",
            Params::Positional(vec![Value::from(board.as_str()), Value::from("laptop")]),
        )
        .await
        .expect("fts query");
    let mut seqs = Vec::new();
    while let Some(r) = rows.next().await.unwrap() {
        seqs.push(match r.get_value(0).unwrap() { Value::Integer(i) => i, _ => panic!("not int") });
    }
    assert_eq!(seqs.len(), 1, "'laptop' should match MacBook Air only, got {:?}", seqs);

    let mut rows = conn
        .query(
            "SELECT seq FROM wb_records WHERE board_id = ?1 AND fts_match(payload, ?2) ORDER BY fts_score(payload, ?2) DESC",
            Params::Positional(vec![Value::from(board.as_str()), Value::from("laptop")]),
        )
        .await
        .expect("fts scored query");
    let mut scored = Vec::new();
    while let Some(r) = rows.next().await.unwrap() {
        scored.push(match r.get_value(0).unwrap() { Value::Integer(i) => i, _ => panic!("not int") });
    }
    assert_eq!(scored.len(), 1, "'laptop' scored query, got {:?}", scored);

    let mut nohit = conn
        .query(
            "SELECT seq FROM wb_records WHERE board_id = ?1 AND fts_match(payload, ?2)",
            Params::Positional(vec![Value::from(board.as_str()), Value::from("zebra")]),
        )
        .await
        .expect("fts no-hit query");
    assert!(nohit.next().await.unwrap().is_none(), "zebra must not match");
    drop(conn);

    let hits = pool.srv_record_search(&board, "laptop", &[], 10, 0, false).await.unwrap();
    assert_eq!(hits.len(), 1, "store search should find MacBook Air, got {:?}", hits);
    assert!(hits[0].score.is_some(), "search results should carry a score");

    let f = zw_db::filters::parse_filter(&serde_json::json!({"search": "workhorse", "brand": {"eq": "Lenovo"}})).unwrap();
    let hits = pool.srv_record_search(&board, "workhorse", &f.conds, 10, 0, false).await.unwrap();
    assert_eq!(hits.len(), 1, "search + filter should find ThinkPad, got {:?}", hits);

    std::fs::remove_dir_all(&dir).ok();
}