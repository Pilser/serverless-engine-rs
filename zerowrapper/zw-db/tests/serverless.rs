use serde_json::{json, Value};
use zw_db::filters::parse_filter;

async fn pool() -> (zw_db::DbPool, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("zwdb_test_srv_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = zw_db::DbPool::open(db_path.to_str().unwrap()).await.unwrap();
    pool.migrate().await.unwrap();
    pool.migrate_session().await.unwrap();
    (pool, dir)
}

async fn register_board(pool: &zw_db::DbPool, board_id: &str, owner: &str) {
    use turso::params::Params;
    let conn = pool.conn().await;
    conn.execute(
        "INSERT INTO srv_board_index (board_id, session_id, owner_key) VALUES (?1, 'test-session', ?2)",
        Params::Positional(vec![turso::Value::from(board_id), turso::Value::from(owner)]),
    )
    .await
    .unwrap();
}

fn payloads() -> Vec<Value> {
    vec![
        json!({"name": "Dell XPS 13", "brand": "Dell", "price": 1200, "body": "the quick brown fox"}),
        json!({"name": "MacBook Air", "brand": "Apple", "price": 1300, "body": "lightweight laptop machine"}),
        json!({"name": "ThinkPad", "brand": "Lenovo", "price": 900, "body": "durable workhorse"}),
    ]
}

#[tokio::test]
async fn set_and_patch_ops() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let seq = pool.srv_record_insert(&board, &payloads()[0], Some("alice"), false).await.unwrap();

    pool.srv_record_set(&board, seq, &json!({"name": "Dell XPS 15", "brand": "Dell", "price": 799}), Some("alice")).await.unwrap();
    let rec = pool.srv_record_get(&board, seq).await.unwrap().unwrap();
    assert_eq!(rec.payload["name"], "Dell XPS 15");
    assert_eq!(rec.writer.as_deref(), Some("alice"));

    let merged = pool.srv_record_patch(&board, seq, &json!({"$set": {"attrs.color": "black"}, "$inc": {"price": 50}, "$dec": {"price": 10}, "$mul": {"price": 2}, "$unset": ["brand"]})).await.unwrap();
    assert_eq!(merged["attrs"]["color"], "black");
    assert_eq!(merged["price"].as_f64(), json!(1678.0).as_f64());
    assert!(merged.get("brand").is_none());

    let merged = pool.srv_record_patch(&board, seq, &json!({"notes": "hello"})).await.unwrap();
    assert_eq!(merged["notes"], "hello");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn unique_duplicates_upsert_writer() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, Some("$.sku")).await.unwrap();

    let s1 = pool.srv_record_insert(&board, &json!({"sku": "A1", "qty": 1}), Some("alice"), false).await.unwrap();
    let err = pool.srv_record_insert(&board, &json!({"sku": "A1", "qty": 2}), Some("bob"), false).await;
    assert!(err.is_err(), "duplicate should be rejected");

    let s2 = pool.srv_record_insert(&board, &json!({"sku": "B1", "qty": 5}), None, false).await.unwrap();
    assert_eq!(s2, s1 + 1);

    let up = pool.srv_record_insert(&board, &json!({"sku": "A1", "qty": 9}), Some("bob"), true).await.unwrap();
    assert_eq!(up, s1, "upsert keeps the same seq");
    let rec = pool.srv_record_get(&board, s1).await.unwrap().unwrap();
    assert_eq!(rec.payload["qty"], 9);

    // writer filter
    let f = parse_filter(&json!({"writer": {"eq": "bob"}})).unwrap();
    let rows = pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].writer.as_deref(), Some("bob"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn offset_ngram_substring_and_snippet() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    for p in payloads() {
        pool.srv_record_insert(&board, &p, None, false).await.unwrap();
    }
    // ngram substring
    let hits = pool.srv_record_search(&board, "lapt", &[], 50, 0, true).await.unwrap();
    assert_eq!(hits.len(), 1, "ngram substring 'lapt' should match MacBook Air, got {:?}", hits);
    assert!(hits[0].snippet.is_some());

    // offset paging in list
    let page1 = pool.srv_record_list(&board, 2, None, 0).await.unwrap();
    let page2 = pool.srv_record_list(&board, 2, None, 2).await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 1);
    assert_ne!(page1[0].seq, page2[0].seq);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn patch_rejects_wrong_shape_and_missing_record() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let seq = pool.srv_record_insert(&board, &json!({"n": 1}), None, false).await.unwrap();
    let err = pool.srv_record_patch(&board, seq, &json!([1, 2])).await;
    assert!(err.is_err(), "array patch should be rejected");
    let err = pool.srv_record_patch(&board, 9999, &json!({"$set": {"a": 1}})).await;
    assert!(err.is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn api_keys_issue_auth_revoke_list() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    register_board(&pool, &board, "owner").await;
    assert!(!pool.srv_board_has_keys(&board).await.unwrap(), "fresh board has no keys");

    let (bucket, secret) = pool.srv_key_issue(&board, "owner", "writer", Some("alice"), None).await.unwrap();
    assert_eq!(bucket.len(), 32);
    assert_eq!(secret.len(), 32);
    assert!(pool.srv_board_has_keys(&board).await.unwrap(), "board becomes key-enforced after issue");

    let key = pool.srv_key_auth(&secret).await.unwrap().expect("auth succeeds with secret");
    assert_eq!(key.board_id, board);
    assert_eq!(key.role, "writer");
    assert_eq!(key.writer.as_deref(), Some("alice"));

    let err = pool.srv_key_issue(&board, "mallory", "writer", None, None).await;
    assert!(err.is_err(), "non-owner cannot issue keys");
    let err = pool.srv_key_issue(&board, "owner", "superuser", None, None).await;
    assert!(err.is_err(), "invalid role rejected");

    let keys = pool.srv_key_list(&board, "owner").await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].bucket, bucket);
    assert!(keys[0].revoked_at.is_none());

    assert!(pool.srv_key_revoke(&board, "owner", &bucket).await.unwrap());
    assert!(pool.srv_key_auth(&secret).await.unwrap().is_none(), "revoked secret no longer authenticates");
    assert!(!pool.srv_board_has_keys(&board).await.unwrap(), "all keys revoked -> board open again");
    let err = pool.srv_key_revoke(&board, "owner", &bucket).await;
    assert!(err.is_err(), "revoking a missing/already-revoked key bails");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn api_key_auth_isolated_between_boards() {
    let (pool, dir) = pool().await;
    let a = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let b = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    register_board(&pool, &a, "owner").await;
    register_board(&pool, &b, "owner").await;
    let (_bucket, secret) = pool.srv_key_issue(&a, "owner", "reader", None, None).await.unwrap();
    let key = pool.srv_key_auth(&secret).await.unwrap().expect("secret authenticates");
    assert_eq!(key.board_id, a);
    assert_ne!(key.board_id, b);
    assert_eq!(key.role, "reader");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn srv_records_after_only_returns_newer_seqs() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let s1 = pool.srv_record_insert(&board, &json!({"n": 1}), None, false).await.unwrap();
    let s2 = pool.srv_record_insert(&board, &json!({"n": 2}), None, false).await.unwrap();
    let s3 = pool.srv_record_insert(&board, &json!({"n": 3}), None, false).await.unwrap();

    let newer = pool.srv_records_after(&board, s1).await.unwrap();
    assert_eq!(newer.len(), 2);
    assert_eq!(newer[0].seq, s2, "ascending order from the after boundary");
    assert_eq!(newer[1].seq, s3);

    let all = pool.srv_records_after(&board, 0).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].seq, s3);

    let none = pool.srv_records_after(&board, s3).await.unwrap();
    assert!(none.is_empty(), "no records past the last seq");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn delete_removes_record_and_returns_notfound_later() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let seq = pool.srv_record_insert(&board, &json!({"n": 1}), None, false).await.unwrap();
    assert!(pool.srv_record_get(&board, seq).await.unwrap().is_some());

    assert!(pool.srv_record_delete_one(&board, seq).await.unwrap(), "delete reports the row was removed");
    assert!(pool.srv_record_get(&board, seq).await.unwrap().is_none(), "record is gone after delete");

    assert!(
        !pool.srv_record_delete_one(&board, seq).await.unwrap(),
        "deleting an already-deleted seq reports no row affected"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn rate_limiter_allows_until_limit_then_denies() {
    let limiter = zw_db::rate::RateLimiter::default();
    let (allowed1, _, remaining1) = limiter.allow("b1:kw:key", "submit", 2, 1000).await;
    let (allowed2, _, remaining2) = limiter.allow("b1:kw:key", "submit", 2, 1000).await;
    assert!(allowed1 && allowed2, "first two requests should be allowed");
    assert!(remaining1 > remaining2, "remaining should decrement: {remaining1} -> {remaining2}");
    let (allowed3, retry_after, remaining3) = limiter.allow("b1:kw:key", "submit", 2, 1000).await;
    assert!(!allowed3, "third request should be denied");
    assert!(retry_after > 0, "denied request should carry a positive retry_after");
    assert_eq!(remaining3, 0, "denied request should report zero remaining");
}

#[test]
fn rate_config_defaults_and_merge() {
    let cfg = zw_db::rate::RateConfig::parse(&json!({"submit": 5}));
    assert_eq!(cfg.submit, 5);
    assert_eq!(cfg.upload, 20);
    assert_eq!(cfg.search, 240);
    assert_eq!(cfg.read, 600);
    assert_eq!(cfg.per_day, 100_000);
    assert_eq!(cfg.limit("search"), 240);
    assert_eq!(cfg.limit("bogus"), 600);
}

#[tokio::test]
async fn rate_config_roundtrip_db() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("u", "t", None, true, None).await.unwrap();
    let before = pool.srv_rate_config(&board).await.unwrap();
    assert_eq!(before.submit, 120, "defaults apply when no config is set");

    pool.srv_rate_set(&board, "u", Some("{\"submit\":7}")).await.unwrap();
    let cfg = pool.srv_rate_config(&board).await.unwrap();
    assert_eq!(cfg.submit, 7);
    assert_eq!(cfg.search, 240);
    assert_eq!(cfg.per_day, 100_000);

    let err = pool.srv_rate_set(&board, "mallory", Some("{\"submit\":1}")).await;
    assert!(err.is_err(), "non-owner cannot change rate config");

    pool.srv_rate_set(&board, "u", None).await.unwrap();
    let after = pool.srv_rate_config(&board).await.unwrap();
    assert_eq!(after.submit, 120, "clearing restores defaults");
    std::fs::remove_dir_all(&dir).ok();
}

fn iso_now(offset_secs: i64) -> String {
    chrono::Utc::now()
        .naive_utc()
        .checked_add_signed(chrono::Duration::seconds(offset_secs))
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn ttl_app_level_expires() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let seq = pool.srv_record_insert(&board, &json!({"n": 1}), None, false).await.unwrap();
    {
        let conn = pool.conn().await;
        conn.execute(
            "UPDATE wb_records SET created_at = datetime('now', '-2 hours') WHERE board_id = ?1",
            turso::params::Params::Positional(vec![turso::Value::from(board.as_str())]),
        )
        .await
        .unwrap();
    }
    pool.srv_ttl_set(&board, "owner", Some(3600), None).await.unwrap();
    let rows = pool.srv_record_filter(&board, &[], &[], 50, 0).await.unwrap();
    assert!(rows.is_empty(), "old record excluded lazily from filter");

    let deleted = pool.srv_ttl_sweep().await.unwrap();
    assert!(deleted >= 1, "sweep deleted the expired record, got {deleted}");
    assert!(pool.srv_record_get(&board, seq).await.unwrap().is_none(), "sweep physically removed it");

    let fresh = pool.srv_record_insert(&board, &json!({"n": 2}), None, false).await.unwrap();
    let rows = pool.srv_record_filter(&board, &[], &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "fresh record still readable; seq={fresh}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn ttl_field_expires() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_ttl_set(&board, "owner", None, Some("$.expires_at")).await.unwrap();
    pool.srv_record_insert(&board, &json!({"expires_at": iso_now(-3600)}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"expires_at": iso_now(3600)}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"name": "no field"}), None, false).await.unwrap();

    let rows = pool.srv_record_filter(&board, &[], &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 2, "past timestamp excluded; future and missing kept");

    let deleted = pool.srv_ttl_sweep().await.unwrap();
    assert_eq!(deleted, 1, "sweep physically deletes just the past record");
    let rows = pool.srv_record_filter(&board, &[], &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn ttl_both_seconds_and_field_expire() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_ttl_set(&board, "owner", Some(3600), Some("$.expires_at")).await.unwrap();

    pool.srv_record_insert(&board, &json!({"expires_at": iso_now(-3600), "k": "past-field"}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"expires_at": iso_now(3600), "k": "future-field"}), None, false).await.unwrap();

    let rows = pool.srv_record_filter(&board, &[], &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "fresh record with expired field still excluded when both rules set");
    assert_eq!(rows[0].payload["k"], "future-field");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn ttl_set_roundtrip_and_clear() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let ttl = pool.srv_ttl_get(&board).await.unwrap();
    assert!(ttl.seconds.is_none() && ttl.field.is_none(), "default is no TTL");

    pool.srv_ttl_set(&board, "owner", Some(3600), Some("$.expires_at")).await.unwrap();
    let ttl = pool.srv_ttl_get(&board).await.unwrap();
    assert_eq!(ttl.seconds, Some(3600));
    assert_eq!(ttl.field.as_deref(), Some("$.expires_at"));

    pool.srv_ttl_set(&board, "owner", Some(60), Some("")).await.unwrap();
    let ttl = pool.srv_ttl_get(&board).await.unwrap();
    assert_eq!(ttl.seconds, Some(60));
    assert!(ttl.field.is_none(), "empty field string clears the field");

    let err = pool.srv_ttl_set(&board, "mallory", Some(5), None).await;
    assert!(err.is_err(), "non-owner cannot change ttl");

    pool.srv_ttl_set(&board, "owner", None, None).await.unwrap();
    let ttl = pool.srv_ttl_get(&board).await.unwrap();
    assert!(ttl.seconds.is_none() && ttl.field.is_none(), "cleared ttl restores both to none");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn hook_register_list_remove_roundtrip() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let url = "https://example.com/webhook";

    pool.srv_hook_register(&board, "owner", url, Some("s3cret")).await.unwrap();
    pool.srv_hook_register(&board, "owner", "http://plain.example/x", None).await.unwrap();

    let err = pool.srv_hook_register(&board, "owner", "ftp://nope.example/x", None).await;
    assert!(err.is_err(), "non-http(s) url rejected");
    let err = pool.srv_hook_register(&board, "mallory", url, None).await;
    assert!(err.is_err(), "non-owner cannot register");

    let hooks = pool.srv_hook_list(&board, "owner").await.unwrap();
    assert_eq!(hooks.len(), 2);
    let secure = hooks.iter().find(|h| h.url == url).unwrap();
    assert!(secure.has_secret, "secret is masked into has_secret=true");

    let urls = pool.srv_hook_urls(&board).await.unwrap();
    let pairs: std::collections::HashMap<_, _> = urls.into_iter().collect();
    assert_eq!(pairs.get(url), Some(&Some("s3cret".to_string())));

    assert!(pool.srv_hook_remove(&board, "owner", url).await.unwrap());
    let err = pool.srv_hook_remove(&board, "owner", url).await;
    assert!(err.is_err(), "removing a missing hook bails");
    let hooks = pool.srv_hook_list(&board, "owner").await.unwrap();
    assert_eq!(hooks.len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn hook_enqueue_appears_due() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let url = "https://example.com/hook";
    register_board(&pool, &board, "owner").await;
    pool.srv_hook_register(&board, "owner", url, None).await.unwrap();

    let event = r#"{"board":"x","type":"record.created","seq":1}"#;
    let id = pool.srv_hook_enqueue(&board, url, event).await.unwrap();
    assert!(id > 0, "enqueue returns the delivery id");

    let err = pool.srv_hook_deliveries(&board, "owner", Some(url)).await.unwrap();
    let err = err.into_iter().find(|d| d.id == id).unwrap();
    assert_eq!(err.event, event);
    assert_eq!(err.attempts, 0);
    assert!(err.delivered_at.is_none());

    let past = pool.srv_hook_due(&iso_now(-3600), 64).await.unwrap();
    assert!(past.is_empty(), "not due yet once next_attempt_at is in the future");

    let due = pool.srv_hook_due(&iso_now(5), 64).await.unwrap();
    assert!(due.iter().any(|d| d.id == id), "delivery becomes due");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn hook_reschedule_deadletter() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let url = "https://example.com/hook";
    register_board(&pool, &board, "owner").await;
    pool.srv_hook_register(&board, "owner", url, None).await.unwrap();
    let id = pool.srv_hook_enqueue(&board, url, r#"{"board":"b","type":"record.created","seq":2}"#).await.unwrap();

    pool.srv_hook_mark(id, 5, Some("500"), None, None).await.unwrap();
    let before = pool.srv_hook_due(&iso_now(60), 64).await.unwrap();
    assert!(!before.iter().any(|d| d.id == id), "dead-lettered delivery is never due");

    let rearmed = pool.srv_hook_reschedule(&board, "owner", url).await.unwrap();
    assert_eq!(rearmed, 1, "dead-letter is re-armed on replay");

    let after = pool.srv_hook_due(&iso_now(60), 64).await.unwrap();
    assert!(after.iter().any(|d| d.id == id), "re-armed delivery appears due again");

    let err = pool.srv_hook_reschedule(&board, "mallory", url).await;
    assert!(err.is_err(), "only the owner can replay deliveries");

    std::fs::remove_dir_all(&dir).ok();
}
#[tokio::test]
async fn join_resolves_parent_payload() {
    let (pool, dir) = pool().await;
    let parent = pool.srv_app_create("owner", "products", None, true, None).await.unwrap();
    let child = pool.srv_app_create("owner", "orders", None, true, None).await.unwrap();
    pool.srv_record_insert(&parent, &json!({"sku": "P1", "name": "Bolt"}), None, false).await.unwrap();
    pool.srv_record_insert(&child, &json!({"sku": "P1", "qty": 2}), None, false).await.unwrap();
    pool.srv_record_insert(&child, &json!({"sku": "Z9", "qty": 1}), None, false).await.unwrap();

    pool.srv_link_set(&child, "owner", "$.sku", &parent, "$.sku").await.unwrap();
    let rows = pool.srv_join_list(&child, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 2);
    let with = rows.iter().find(|(r, _)| r.payload["sku"] == "P1").unwrap();
    let joined = with.1.as_ref().unwrap();
    assert_eq!(joined["$.sku"]["name"], "Bolt", "parent payload embedded");
    let dangling = rows.iter().find(|(r, _)| r.payload["sku"] == "Z9").unwrap();
    assert!(dangling.1.as_ref().unwrap()["$.sku"].is_null(), "unknown sku -> null join");

    let err = pool.srv_link_set(&child, "mallory", "$.sku", &parent, "$.sku").await;
    assert!(err.is_err(), "only owner can link");

    pool.srv_link_clear(&child, "owner").await.unwrap();
    let plain = pool.srv_join_list(&child, &[], 50, 0).await.unwrap();
    assert!(plain.iter().all(|(_, j)| j.is_none()), "after clear joins are None");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn audit_roundtrip_when_enabled_and_skipped_when_off() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();

    let disabled = pool.srv_audit_list(&board, "owner", None, 100).await.unwrap();
    assert!(disabled.is_empty(), "audit off by default, nothing recorded");

    pool.srv_record_insert(&board, &json!({"k": 1}), Some("alice"), false).await.unwrap();
    let none = pool.srv_audit_list(&board, "owner", None, 100).await.unwrap();
    assert!(none.is_empty(), "no audit rows while disabled");

    pool.srv_app_audit_set(&board, "owner", true).await.unwrap();
    let err = pool.srv_app_audit_set(&board, "mallory", true).await;
    assert!(err.is_err(), "only owner can toggle audit");

    let seq = pool.srv_record_insert(&board, &json!({"a": 2}), Some("alice"), false).await.unwrap();
    pool.srv_record_set(&board, seq, &json!({"a": 3}), Some("bob")).await.unwrap();
    let _ = pool.srv_record_patch(&board, seq, &json!({"$inc": {"a": 1}})).await.unwrap();
    pool.srv_record_delete_one(&board, seq).await.unwrap();

    let rows = pool.srv_audit_list(&board, "owner", None, 100).await.unwrap();
    assert!(rows.len() >= 4, "created/updated/patch/delete recorded, got {}", rows.len());
    assert!(rows.iter().all(|r| r.event == "created" || r.event == "updated" || r.event == "deleted"));
    assert!(rows.iter().any(|r| r.event == "created" && r.writer.as_deref() == Some("alice")), "created logged with writer");
    assert!(rows.iter().any(|r| r.event == "deleted" && r.payload_hash.is_none()), "delete has no payload hash");

    let first = &rows[0];
    let since = pool.srv_audit_list(&board, "owner", Some(&first.ts), 100).await.unwrap();
    assert!(!since.is_empty(), "since filter excludes nothing earlier");

    let err = pool.srv_audit_list(&board, "mallory", None, 100).await;
    assert!(err.is_err(), "non-owner cannot read audit trail");

    pool.srv_app_audit_set(&board, "owner", false).await.unwrap();
    let again = pool.srv_audit_list(&board, "owner", None, 100).await.unwrap();
    assert!(!again.is_empty(), "history persists after disabling");
    let before = again.len();
    pool.srv_record_insert(&board, &json!({"k": 9}), Some("carol"), false).await.unwrap();
    let after = pool.srv_audit_list(&board, "owner", None, 100).await.unwrap().len();
    assert_eq!(after, before, "no new audit rows while off");

    std::fs::remove_dir_all(&dir).ok();
}
