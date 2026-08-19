use serde_json::{json, Value};
use zw_db::filters::{Agg, parse_filter};

async fn pool() -> (zw_db::DbPool, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("zwdb_test_feat_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let pool = zw_db::DbPool::open(db_path.to_str().unwrap()).await.unwrap();
    pool.migrate().await.unwrap();
    pool.migrate_session().await.unwrap();
    (pool, dir)
}

fn payloads() -> Vec<Value> {
    vec![
        json!({"name": "Dell XPS 13", "brand": "Dell", "price": 1200}),
        json!({"name": "MacBook Air", "brand": "Apple", "price": 1300}),
        json!({"name": "ThinkPad", "brand": "Lenovo", "price": 900}),
    ]
}

#[tokio::test]
async fn bulk_insert_seqs_writer_and_empty() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();

    let empty = pool.srv_record_bulk_insert(&board, &[], None, false).await.unwrap();
    assert!(empty.is_empty(), "empty batch yields no seqs");

    let recs = payloads();
    let seqs = pool.srv_record_bulk_insert(&board, &recs, Some("alice"), false).await.unwrap();
    assert_eq!(seqs, vec![1, 2, 3], "one seq per record, in order");

    let all = pool.srv_record_list(&board, 50, None, 0).await.unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|r| r.writer.as_deref() == Some("alice")), "batch writer applied to every record");
    assert_eq!(pool.srv_record_count(&board).await.unwrap(), 3);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn bulk_insert_aborts_on_schema_failure() {
    let schema = r#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}"#;
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", Some(schema), true, None).await.unwrap();

    let err = pool.srv_record_bulk_insert(&board, &[json!({"name": "ok"}), json!({"price": 5})], None, false).await;
    assert!(err.is_err(), "schema-violating record must abort the batch");

    let rows = pool.srv_record_list(&board, 50, None, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "records already inserted before the failure persist");

    let err = pool.srv_record_bulk_insert(&board, &[json!({"n": 1}), json!({"n": 2})], None, false).await;
    assert!(err.is_err(), "every record missing required field -> whole batch rejected");
    let rows = pool.srv_record_list(&board, 50, None, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "no new records from a fully-invalid batch");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn bulk_upsert_updates_in_place() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, Some("$.sku")).await.unwrap();
    let s1 = pool.srv_record_insert(&board, &json!({"sku": "A1", "qty": 1}), None, false).await.unwrap();

    let seqs = pool.srv_record_bulk_insert(&board, &[json!({"sku": "A1", "qty": 9}), json!({"sku": "B1", "qty": 2})], None, true).await.unwrap();
    assert_eq!(seqs.len(), 2);
    assert_eq!(seqs[0], s1, "upsert reuses the existing seq");

    let rec = pool.srv_record_get(&board, s1).await.unwrap().unwrap();
    assert_eq!(rec.payload["qty"], 9, "payload replaced in place");
    let rec = pool.srv_record_get(&board, seqs[1]).await.unwrap().unwrap();
    assert_eq!(rec.payload["qty"], 2);

    let err = pool.srv_record_bulk_insert(&board, &[json!({"sku": "B1", "qty": 5})], None, false).await;
    assert!(err.is_err(), "duplicate in bulk without upsert is rejected");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn delete_by_filter_removes_only_matches() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    for p in payloads() {
        pool.srv_record_insert(&board, &p, None, false).await.unwrap();
    }

    let f = parse_filter(&json!({"brand": {"eq": "Dell"}})).unwrap();
    let n = pool.srv_record_delete_filter(&board, &f.conds).await.unwrap();
    assert_eq!(n, 1, "one Dell row deleted");

    let f = parse_filter(&json!({"price": {"gte": 1000}})).unwrap();
    let n = pool.srv_record_delete_filter(&board, &f.conds).await.unwrap();
    assert_eq!(n, 1, "only the 1300 MacBook remains above 1000");

    let remaining = pool.srv_record_list(&board, 50, None, 0).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].payload["brand"], "Lenovo");

    let n = pool.srv_record_delete_filter(&board, &f.conds).await.unwrap();
    assert_eq!(n, 0, "no matches -> zero deleted");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn multi_column_sort_asc_desc_and_secondary_key() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    for p in payloads() {
        pool.srv_record_insert(&board, &p, None, false).await.unwrap();
    }

    let rows = pool.srv_record_filter(&board, &[], &[("$.price", false)], 50, 0).await.unwrap();
    let prices: Vec<i64> = rows.iter().map(|r| r.payload["price"].as_i64().unwrap()).collect();
    assert_eq!(prices, vec![900, 1200, 1300], "ascending price");

    let rows = pool.srv_record_filter(&board, &[], &[("$.price", true)], 50, 0).await.unwrap();
    let prices: Vec<i64> = rows.iter().map(|r| r.payload["price"].as_i64().unwrap()).collect();
    assert_eq!(prices, vec![1300, 1200, 900], "descending price");

    let b2 = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_record_insert(&b2, &json!({"brand": "Apple", "price": 1300}), None, false).await.unwrap();
    pool.srv_record_insert(&b2, &json!({"brand": "Apple", "price": 900}), None, false).await.unwrap();
    pool.srv_record_insert(&b2, &json!({"brand": "Dell", "price": 1200}), None, false).await.unwrap();
    let rows = pool.srv_record_filter(&b2, &[], &[("$.brand", false), ("$.price", true)], 50, 0).await.unwrap();
    let pairs: Vec<(String, i64)> = rows
        .iter()
        .map(|r| (r.payload["brand"].as_str().unwrap().to_string(), r.payload["price"].as_i64().unwrap()))
        .collect();
    assert_eq!(pairs, vec![("Apple".to_string(), 1300), ("Apple".to_string(), 900), ("Dell".to_string(), 1200)], "brand asc then price desc");

    let rows = pool.srv_record_filter(&board, &[], &[("$.price", true)], 50, 1).await.unwrap();
    assert_eq!(rows.len(), 2, "offset skips the top row");
    assert_eq!(rows[0].payload["price"].as_i64().unwrap(), 1200);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn app_lifecycle_create_update_list_delete() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, false, None).await.unwrap();

    let app = pool.srv_app_by_id(&board).await.unwrap().unwrap();
    assert_eq!(app.title, "t");
    assert!(!app.public_reads);
    assert_eq!(app.record_count, 0);

    pool.srv_record_insert(&board, &json!({"k": 1}), None, false).await.unwrap();
    let list = pool.srv_app_list("owner").await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].record_count, 1, "app list reports live record count");

    let not_owned = pool.srv_app_owned(&board, "mallory").await.unwrap();
    assert!(not_owned.is_none(), "srv_app_owned returns none for a non-owner");
    let owned = pool.srv_app_owned(&board, "owner").await.unwrap();
    assert!(owned.is_some());

    pool.srv_app_update(&board, "owner", Some("new title"), Some(Some(r#"{"type":"object"}"#)), Some(true), Some(None)).await.unwrap();
    let app = pool.srv_app_by_id(&board).await.unwrap().unwrap();
    assert_eq!(app.title, "new title");
    assert_eq!(app.schema_json.as_deref(), Some(r#"{"type":"object"}"#));
    assert!(app.public_reads);
    assert!(app.unique_key.is_none(), "unique_key cleared to None");

    let err = pool.srv_app_update(&board, "mallory", Some("x"), None, None, None).await;
    assert!(err.is_err(), "non-owner cannot update");

    pool.srv_app_delete(&board, "owner").await.unwrap();
    assert!(pool.srv_app_by_id(&board).await.unwrap().is_none(), "board deleted");

    let err = pool.srv_app_delete(&board, "mallory").await;
    assert!(err.is_err(), "deleting a missing/foreign board bails");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn schema_enforced_on_insert_set_patch() {
    let schema = r#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"price":{"type":"number"}}}"#;
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", Some(schema), true, None).await.unwrap();

    let seq = pool.srv_record_insert(&board, &json!({"name": "x", "price": 1}), None, false).await.unwrap();
    let err = pool.srv_record_insert(&board, &json!({"price": 5}), None, false).await;
    assert!(err.is_err(), "missing required field rejected on insert");

    let err = pool.srv_record_set(&board, seq, &json!({"name": 42}), None).await;
    assert!(err.is_err(), "wrong type rejected on set");

    let err = pool.srv_record_patch(&board, seq, &json!({"$set": {"price": "NaN"}})).await;
    assert!(err.is_err(), "wrong type rejected on patch");

    let err = pool.srv_validate(&board, &json!({"name": 1})).await;
    assert!(err.is_err(), "srv_validate rejects a bad payload");
    pool.srv_validate(&board, &json!({"name": "ok"})).await.unwrap();
    pool.srv_validate(&board, &json!({"name": "ok", "price": 2})).await.unwrap();

    let err = pool.srv_validate("no-such-board", &json!({"name": "ok"})).await;
    assert!(err.is_err(), "srv_validate on a missing board bails");

    let err = pool.srv_record_insert(&board, &json!({"name": "z", "price": "not-a-number"}), None, false).await.unwrap_err();
    assert!(err.to_string().contains("schema"), "schema error surfaces in the message: {err}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn set_and_patch_enforce_unique_collision() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, Some("$.sku")).await.unwrap();
    pool.srv_record_insert(&board, &json!({"sku": "A1"}), None, false).await.unwrap();
    let s2 = pool.srv_record_insert(&board, &json!({"sku": "B2"}), None, false).await.unwrap();

    let err = pool.srv_record_set(&board, s2, &json!({"sku": "A1"}), None).await;
    assert!(err.is_err(), "set to a colliding unique value bails");

    let err = pool.srv_record_patch(&board, s2, &json!({"$set": {"sku": "A1"}})).await;
    assert!(err.is_err(), "patch to a colliding unique value bails");

    pool.srv_record_set(&board, s2, &json!({"sku": "B2", "extra": true}), None).await.unwrap();
    let rec = pool.srv_record_get(&board, s2).await.unwrap().unwrap();
    assert_eq!(rec.payload["extra"], true, "re-setting the same unique value is fine");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn patch_edge_cases() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let seq = pool.srv_record_insert(&board, &json!({"n": 1}), None, false).await.unwrap();

    let merged = pool.srv_record_patch(&board, seq, &json!({"$inc": {"votes": 2}})).await.unwrap();
    assert_eq!(merged["votes"], json!(2), "$inc on a missing field starts at zero");
    let merged = pool.srv_record_patch(&board, seq, &json!({"$inc": {"votes": 3}})).await.unwrap();
    assert_eq!(merged["votes"], json!(5));
    let merged = pool.srv_record_patch(&board, seq, &json!({"$dec": {"votes": 1}})).await.unwrap();
    assert_eq!(merged["votes"], json!(4));
    let merged = pool.srv_record_patch(&board, seq, &json!({"$mul": {"votes": 10}})).await.unwrap();
    assert_eq!(merged["votes"], json!(40));

    let err = pool.srv_record_patch(&board, seq, &json!({"$": 1})).await;
    assert!(err.is_err(), "bare $ operator is unknown");
    let err = pool.srv_record_patch(&board, seq, &json!({"$set": [1, 2]})).await;
    assert!(err.is_err(), "$set requires an object of paths");
    let err = pool.srv_record_patch(&board, seq, &json!({"$set": {"n.deep": 1}})).await;
    assert!(err.is_err(), "setting a path under a scalar errors");
    let err = pool.srv_record_patch(&board, seq, &json!({"$inc": {"votes": "x"}})).await;
    assert!(err.is_err(), "$inc delta must be numeric");

    let merged = pool.srv_record_patch(&board, seq, &json!({"$unset": {"n": 1}})).await.unwrap();
    assert!(merged.get("n").is_none(), "$unset object form removes the field");
    let merged = pool.srv_record_patch(&board, seq, &json!({"$set": {"n.deep": 1}})).await.unwrap();
    assert_eq!(merged["n"]["deep"], json!(1), "creating a nested object path works");

    let err = pool.srv_record_patch(&board, 99999, &json!({"$set": {"a": 1}})).await;
    assert!(err.is_err(), "patch on a missing seq bails");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn record_query_and_batch_delete() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let s1 = pool.srv_record_insert(&board, &json!({"sku": "A1", "meta": {"cat": "x"}}), None, false).await.unwrap();
    let s2 = pool.srv_record_insert(&board, &json!({"sku": "B2", "meta": {"cat": "y"}}), None, false).await.unwrap();

    let rows = pool.srv_record_query(&board, "$.sku", Some("B2")).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, s2);
    let rows = pool.srv_record_query(&board, "$.meta.cat", Some("x")).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, s1);
    let all = pool.srv_record_query(&board, "$.sku", None).await.unwrap();
    assert_eq!(all.len(), 2, "query without eq returns all records");

    pool.srv_record_delete(&board, Some(s1)).await.unwrap();
    assert!(pool.srv_record_get(&board, s1).await.unwrap().is_none());
    assert!(pool.srv_record_get(&board, s2).await.unwrap().is_some(), "batch delete only removed the one seq");

    pool.srv_record_delete(&board, None).await.unwrap();
    assert_eq!(pool.srv_record_count(&board).await.unwrap(), 0, "delete-all wipes the board");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn aggregate_math_and_group_by() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_record_insert(&board, &json!({"brand": "Apple", "price": 1000}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"brand": "Apple", "price": 2000}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"brand": "Dell", "price": 3000}), None, false).await.unwrap();

    let agg = pool.srv_record_aggregate(&board, &[], Agg::Count, None, None).await.unwrap();
    assert_eq!(agg[0]["value"], json!(3));

    let agg = pool.srv_record_aggregate(&board, &[], Agg::Sum, Some("$.price"), None).await.unwrap();
    assert_eq!(agg[0]["value"].as_f64(), json!(6000.0).as_f64());
    let agg = pool.srv_record_aggregate(&board, &[], Agg::Avg, Some("$.price"), None).await.unwrap();
    assert_eq!(agg[0]["value"].as_f64(), json!(2000.0).as_f64());
    let agg = pool.srv_record_aggregate(&board, &[], Agg::Min, Some("$.price"), None).await.unwrap();
    assert_eq!(agg[0]["value"].as_f64(), json!(1000.0).as_f64());
    let agg = pool.srv_record_aggregate(&board, &[], Agg::Max, Some("$.price"), None).await.unwrap();
    assert_eq!(agg[0]["value"].as_f64(), json!(3000.0).as_f64());

    let f = parse_filter(&json!({"brand": {"eq": "Apple"}})).unwrap();
    let agg = pool.srv_record_aggregate(&board, &f.conds, Agg::Count, None, None).await.unwrap();
    assert_eq!(agg[0]["value"], json!(2), "filtered count");

    let agg = pool.srv_record_aggregate(&board, &[], Agg::Sum, Some("$.price"), Some("$.brand")).await.unwrap();
    assert_eq!(agg.len(), 2, "one row per group");
    let apple = agg.iter().find(|g| g["group"] == "Apple").unwrap();
    assert_eq!(apple["value"].as_f64(), json!(3000.0).as_f64());
    let dell = agg.iter().find(|g| g["group"] == "Dell").unwrap();
    assert_eq!(dell["value"].as_f64(), json!(3000.0).as_f64());

    let err = pool.srv_record_aggregate(&board, &[], Agg::Sum, None, None).await;
    assert!(err.is_err(), "sum/avg/min/max require a field");
    assert!(Agg::parse("bogus").is_err(), "unknown aggregate name rejected");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn filter_ops_and_nested_paths() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    for p in payloads() {
        pool.srv_record_insert(&board, &p, None, false).await.unwrap();
    }

    let f = parse_filter(&json!({"brand": {"neq": "Dell"}})).unwrap();
    assert_eq!(pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap().len(), 2);
    let f = parse_filter(&json!({"price": {"lt": 1200}})).unwrap();
    assert_eq!(pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap().len(), 1);
    let f = parse_filter(&json!({"price": {"lte": 1200}})).unwrap();
    assert_eq!(pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap().len(), 2);
    let f = parse_filter(&json!({"price": {"gt": 900}})).unwrap();
    assert_eq!(pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap().len(), 2);
    let f = parse_filter(&json!({"brand": {"not_contains": "ap"}})).unwrap();
    assert_eq!(pool.srv_record_filter(&board, &f.conds, &[], 50, 0).await.unwrap().len(), 2, "Dell + Lenovo lack 'ap'");

    let b2 = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_record_insert(&b2, &json!({"tags": ["a", "b"], "nested": {"deep": {"n": 5}}}), None, false).await.unwrap();
    pool.srv_record_insert(&b2, &json!({"tags": ["c"], "nested": {"deep": {"n": 9}}}), None, false).await.unwrap();

    let f = parse_filter(&json!({"nested.deep.n": {"gte": 7}})).unwrap();
    let rows = pool.srv_record_filter(&b2, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload["nested"]["deep"]["n"], json!(9));

    let f = parse_filter(&json!({"tags[0]": {"eq": "a"}})).unwrap();
    let rows = pool.srv_record_filter(&b2, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "array index path filter");

    let f = parse_filter(&json!({"tags[1]": {"eq": "b"}})).unwrap();
    let rows = pool.srv_record_filter(&b2, &f.conds, &[], 50, 0).await.unwrap();
    assert_eq!(rows.len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn filter_parse_rejects_garbage_without_panicking() {
    assert!(parse_filter(&json!([{"field": "x", "op": "bogus", "value": 1}])).is_err());
    assert!(parse_filter(&json!({"a": {"in": 5}})).is_err(), "in needs an array");
    assert!(parse_filter(&json!({"search": 5})).is_err(), "search needs a string");
    assert!(parse_filter(&json!(42)).is_err(), "scalar filter rejected");
    assert!(parse_filter(&json!("str")).is_err());
    assert!(parse_filter(&json!({"a": {"eq": 1}})).is_ok());
    assert!(parse_filter(&json!({"a": {"gte": 1}, "b": {"lt": 2}})).is_ok());
}

#[test]
fn path_items_parses_dots_and_indexes() {
    use zw_db::filters::{PathSeg, path_items};

    let segs = path_items("$.a.b[2].c").unwrap();
    assert_eq!(segs.len(), 4);
    assert!(matches!(&segs[0], PathSeg::Key(k) if k == "a"));
    assert!(matches!(&segs[1], PathSeg::Key(k) if k == "b"));
    assert!(matches!(&segs[2], PathSeg::Idx(2)));
    assert!(matches!(&segs[3], PathSeg::Key(k) if k == "c"));

    let segs = path_items("a").unwrap();
    assert!(matches!(&segs[0], PathSeg::Key(k) if k == "a"));

    assert!(path_items("$").is_none(), "bare root is not a valid path");
    assert!(path_items("").is_none(), "empty path is invalid");
    assert!(path_items("a[").is_none(), "unterminated bracket is invalid");
    assert!(path_items("a[x]").is_none(), "non-numeric index is invalid");
}

#[tokio::test]
async fn search_snippet_and_ngram() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_record_insert(&board, &json!({"name": "Dell XPS 13", "brand": "Dell", "body": "the quick brown fox"}), None, false).await.unwrap();
    pool.srv_record_insert(&board, &json!({"name": "MacBook Air", "brand": "Apple", "body": "lightweight laptop machine"}), None, false).await.unwrap();

    let hits = pool.srv_record_search(&board, "lapt", &[], 50, 0, true).await.unwrap();
    assert_eq!(hits.len(), 1, "ngram substring 'lapt' matches MacBook Air");
    assert_eq!(hits[0].payload["brand"], "Apple");
    assert!(hits[0].snippet.as_deref().unwrap_or("").contains("<mark>lapt</mark>"), "snippet highlights the hit");

    let hits = pool.srv_record_search(&board, "fox", &[], 50, 0, false).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].payload["brand"], "Dell");

    let f = parse_filter(&json!({"search": "laptop"})).unwrap();
    let hits = pool.srv_record_search(&board, "laptop", &f.conds, 50, 0, false).await.unwrap();
    assert_eq!(hits.len(), 1, "search + search-filter together");

    let hits = pool.srv_record_search(&board, "laptop", &[], 50, 1, false).await.unwrap();
    assert!(hits.is_empty(), "offset past the single hit");

    let hits = pool.srv_record_search(&board, "zzz", &[], 50, 0, false).await.unwrap();
    assert!(hits.is_empty(), "no hits for a nonsense term");

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
async fn jobs_lifecycle_add_list_due_reschedule_mark_remove() {
    let (pool, dir) = pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();

    let (id1, _) = pool.srv_job_add(&board, "every-1s", "@every 1s", r#"{"type":"http","url":"https://example.com/x"}"#).await.unwrap();
    assert!(id1 > 0);
    let (id2, _) = pool.srv_job_add(&board, "far-future", "@every 100000d", r#"{"type":"poll","url":"https://api.example.com/x","path":"$.data"}"#).await.unwrap();
    let (id3, _) = pool.srv_job_add(&board, "daily", "30 9 1 1 *", r#"{"type":"http","url":"https://example.com/y"}"#).await.unwrap();

    let err = pool.srv_job_add(&board, "bad", "not-a-cron", "{}").await;
    assert!(err.is_err(), "invalid schedule must be rejected");

    let list = pool.srv_job_list(Some(&board)).await.unwrap();
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|j| j.next_run_at.is_some()), "every new job carries a next run");
    let all = pool.srv_job_list(None).await.unwrap();
    assert!(all.len() >= 3, "system-wide list spans boards");

    let due_iso = iso_now(10);
    let due = pool.srv_job_due(&due_iso, 10).await.unwrap();
    assert!(due.iter().any(|j| j.id == id1), "@every 1s job should be due within 10s");
    assert!(!due.iter().any(|j| j.id == id2), "far-future job never due");
    assert!(!due.iter().any(|j| j.id == id3), "annual cron not due");

    let limited = pool.srv_job_due(&due_iso, 1).await.unwrap();
    assert!(limited.len() <= 1, "due limit respected");

    pool.srv_job_reschedule(id1, None).await.unwrap();
    let due = pool.srv_job_due(&due_iso, 10).await.unwrap();
    assert!(!due.iter().any(|j| j.id == id1), "NULL next_run never comes due");

    pool.srv_job_reschedule(id1, Some(&iso_now(60))).await.unwrap();
    pool.srv_job_mark(id1, &iso_now(0), "ok", "http 200").await.unwrap();
    let job = pool.srv_job_list(Some(&board)).await.unwrap().into_iter().find(|j| j.id == id1).unwrap();
    assert_eq!(job.last_status.as_deref(), Some("ok"));
    assert_eq!(job.last_message.as_deref(), Some("http 200"));
    assert!(job.last_run_at.is_some());

    assert!(pool.srv_job_remove(id1).await.unwrap());
    assert!(!pool.srv_job_remove(id1).await.unwrap(), "removing a missing id returns false");
    let list = pool.srv_job_list(Some(&board)).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(!list.iter().any(|j| j.id == id1));

    std::fs::remove_dir_all(&dir).ok();
}
