use serde_json::json;
use std::collections::HashMap;

async fn tmp_pool() -> (zw_db::DbPool, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "zwdb_logic_{}_{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");
    let pool = zw_db::DbPool::open(path.to_str().unwrap()).await.unwrap();
    pool.migrate().await.unwrap();
    pool.migrate_session().await.unwrap();
    (pool, dir)
}

#[tokio::test]
async fn computed_fields_and_validate_rules_run_on_write() {
    let (pool, dir) = tmp_pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_app_set_computed(&board, "owner", Some(&json!({"total": "$.price * $.qty"}).to_string())).await.unwrap();
    pool.srv_app_set_validate(&board, "owner", Some(&json!([{"when": "$.total > 1000", "error": "total too big"}]).to_string())).await.unwrap();

    let seq = pool.srv_record_insert(&board, &json!({"price": 10, "qty": 3}), None, false).await.unwrap();
    let rec = pool.srv_record_get(&board, seq).await.unwrap().unwrap();
    assert_eq!(rec.payload["total"], json!(30.0), "computed field materialized");

    let err = pool.srv_record_insert(&board, &json!({"price": 500, "qty": 10}), None, false).await;
    assert!(err.is_err(), "validate rule rejects oversized total");
    assert!(err.unwrap_err().to_string().contains("total too big"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn redact_masks_paths() {
    let (pool, dir) = tmp_pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_app_set_redact(&board, "owner", Some(&json!(["$.ssn"]).to_string())).await.unwrap();
    let mut payload = json!({"name": "a", "ssn": "123-45-6789", "nested": {"token": "x"}});
    let redact = pool.srv_redact_get(&board).await.unwrap().unwrap();
    zw_db::logic::apply_redact(&redact, &mut payload);
    assert_eq!(payload["ssn"], json!("***"));
    assert_eq!(payload["name"], json!("a"), "non-redacted path untouched");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn secrets_roundtrip_encrypted_and_masked() {
    let (pool, dir) = tmp_pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    pool.srv_secret_set(&board, "STRIPE", "sk_test_abc").await.unwrap();
    let list = pool.srv_secret_list(&board).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "STRIPE");
    let ser = serde_json::to_string(&list).unwrap();
    assert!(!ser.contains("sk_test_abc"), "secret value never leaks in list");

    let map: HashMap<String, String> = pool.srv_secrets_map(&board).await.unwrap();
    assert_eq!(map.get("STRIPE").map(|s| s.as_str()), Some("sk_test_abc"), "decryptable in-memory only");

    assert!(pool.srv_secret_remove(&board, "STRIPE").await.unwrap());
    assert!(pool.srv_secret_list(&board).await.unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn recipes_crud_and_dedup() {
    let (pool, dir) = tmp_pool().await;
    let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();
    let id = pool
        .srv_recipe_add(&board, "r1", "record.created", Some(&json!({"$.status": "paid"}).to_string()), true, Some("$.id"), &json!([{"$compute": {"a": "1"}}]).to_string())
        .await
        .unwrap();
    assert!(id > 0);
    let list = pool.srv_recipe_list(&board).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "r1");
    assert_eq!(list[0].match_json, Some(json!({"$.status": "paid"})));

    assert!(!pool.srv_recipe_ran(&board, "r1", &json!(7)).await.unwrap());
    pool.srv_recipe_mark_ran(&board, "r1", &json!(7)).await.unwrap();
    assert!(pool.srv_recipe_ran(&board, "r1", &json!(7)).await.unwrap(), "dedup key hashed and remembered");

    assert!(pool.srv_recipe_remove(&board, "r1").await.unwrap());
    assert!(pool.srv_recipe_list(&board).await.unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn logic_pure_functions() {
    let mut p = json!({"a": {"b": [1, 2]}});
    zw_db::logic::apply_computed(&json!({"total": "$.a.b[1] * 2"}), &mut p).unwrap();
    assert_eq!(p["total"], json!(4.0));

    let rules = json!([{"when": "$.total > 3", "error": "big"}]);
    assert_eq!(zw_db::logic::validate_rules(&rules, &p).unwrap(), Some("big".to_string()));
    assert_eq!(zw_db::logic::validate_rules(&rules, &json!({"total": 1})).unwrap(), None);
}
