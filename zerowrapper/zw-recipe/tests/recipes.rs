use serde_json::json;
use zw_recipe::{Engine, EventContext, Secrets, parse_recipe};
use zw_recipe::actions::match_public;

fn recipe_json() -> serde_json::Value {
    json!({
        "name": "paid-invoice",
        "when": "record.created",
        "match": {"$.status": {"eq": "paid"}},
        "actions": [
            {"$compute": {"total": "$.price * $.qty"}},
            {"$compute": {"label": "concat(upper($.customer), '-', $.id)"}},
            {"$push": {"path": "$.tags", "value": "auto"}}
        ]
    })
}

#[test]
fn parse_and_kind() {
    let r = parse_recipe(&recipe_json()).unwrap();
    assert_eq!(r.name, "paid-invoice");
    assert_eq!(r.event_kind(), Some(zw_recipe::EventKind::Created));
}

#[test]
fn match_filter_combos() {
    let p = json!({"status": "paid", "qty": 5, "tags": ["a"], "sku": 10});
    assert!(match_public(&p, &json!({"$.status": {"eq": "paid"}})).unwrap());
    assert!(match_public(&p, &json!({"$.qty": {"gte": 3}, "$.tags": {"contains": "a"}})).unwrap());
    assert!(match_public(&p, &json!({"$.sku": {"in": [1, 10, 20]}})).unwrap());
    assert!(!match_public(&p, &json!({"$.qty": {"lt": 1}})).unwrap());
}

#[test]
fn secrets_do_not_leak_in_report() {
    let mut s = Secrets::new();
    s.insert("KEY", "sekret");
    assert!(!serde_json::to_string(&s.masked()).unwrap().contains("sekret"));
    assert_eq!(s.substitute("Bearer ${KEY}"), "Bearer sekret");
}

#[tokio::test]
async fn engine_dry_run_applies_compute() {
    let dir = std::env::temp_dir().join(format!("zwrecipe_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = zw_db::DbPool::open(dir.join("test.db").to_str().unwrap()).await.unwrap();
    db.migrate().await.unwrap();
    db.migrate_session().await.unwrap();
    let board = db.srv_app_create("owner", "t", None, true, None).await.unwrap();

    let recipe = parse_recipe(&recipe_json()).unwrap();
    let ctx = EventContext::record_created(&board, 1, &json!({"status": "paid", "price": 10, "qty": 3, "customer": "acme", "id": 7}), None);
    let engine = Engine { db: &db };
    let outcomes = engine.run_recipes(&[recipe], &Secrets::new(), &ctx, true).await;
    assert_eq!(outcomes.len(), 1);
    let (name, out) = &outcomes[0];
    assert_eq!(name, "paid-invoice");
    assert!(out.matched);
    assert!(out.report.actions_run.iter().any(|a| a.contains("Compute")));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn event_substitution_resolves_payload_and_scalars() {
    let mut s = Secrets::new();
    s.insert("TOKEN", "t0k");
    let ctx = EventContext::record_created("b1", 7, &json!({"customer": "acme", "qty": 3}), Some("w1"));
    let v = zw_recipe::secrets::substitute_event(
        &s,
        &ctx,
        &json!({
            "name": "{{payload.customer}}",
            "qty": "{{payload.qty}}",
            "seq": "{{seq}}",
            "board": "{{board}}",
            "writer": "{{writer}}",
            "kind": "{{kind}}",
            "secret": "{{secrets.TOKEN}}"
        }),
    )
    .unwrap();
    assert_eq!(v["name"], json!("acme"));
    assert_eq!(v["qty"], json!(3));
    assert_eq!(v["seq"], json!(7));
    assert_eq!(v["board"], json!("b1"));
    assert_eq!(v["writer"], json!("w1"));
    assert_eq!(v["kind"], json!("created"));
    assert_eq!(v["secret"], json!("t0k"));
}

#[test]
fn single_token_payload_stays_numeric() {
    let s = Secrets::new();
    let ctx = EventContext::record_created("b1", 1, &json!({"qty": 3}), None);
    let v = zw_recipe::secrets::substitute_event(&s, &ctx, &json!("{{payload.qty}}")).unwrap();
    assert!(v.is_number());
    assert_eq!(v, json!(3));
}

#[test]
fn unknown_template_key_fails() {
    let s = Secrets::new();
    let ctx = EventContext::record_created("b1", 1, &json!({"a": 1}), None);
    let v = zw_recipe::secrets::substitute_event(&s, &ctx, &json!("{{payload.missing}}"));
    assert!(v.is_err());
    let v = zw_recipe::secrets::substitute_event(&s, &ctx, &json!("{{bogus}}"));
    assert!(v.is_err());
}

#[tokio::test]
async fn engine_patch_other_and_upsert_other_same_board() {
    let dir = std::env::temp_dir().join(format!("zwpatch_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = zw_db::DbPool::open(dir.join("test.db").to_str().unwrap()).await.unwrap();
    db.migrate().await.unwrap();
    db.migrate_session().await.unwrap();
    let board = db.srv_app_create("owner", "t", None, true, None).await.unwrap();

    db.srv_record_insert(&board, &json!({"sku": "A1", "stock": 10}), None, false).await.unwrap();

    let recipe = parse_recipe(&json!({
        "name": "stock-dec",
        "when": "record.created",
        "actions": [
            {"$patch_other": {"filter": [{"field": "sku", "op": "eq", "value": "A1"}], "patch": {"$inc": {"stock": "{{payload.qty}}"}}}}
        ]
    }))
    .unwrap();
    let ctx = EventContext::record_created(&board, 2, &json!({"sku": "ORD-1", "qty": 3}), None);
    let engine = Engine { db: &db };
    let out = engine.run_recipes(&[recipe], &Secrets::new(), &ctx, false).await;
    assert!(out[0].1.report.logs.is_empty(), "unexpected logs: {:?}", out[0].1.report.logs);

    let records = db.srv_record_list_raw(&board, 100, None, 0).await.unwrap();
    let prod = records.iter().find(|r| r.payload["sku"] == json!("A1")).unwrap();
    assert_eq!(prod.payload["stock"], json!(13));

    let recipe2 = parse_recipe(&json!({
        "name": "note-cron",
        "when": "inbound",
        "actions": [
            {"$upsert_other": {"payload": {"note": "{{payload.msg}}", "qty": "{{payload.qty}}"}}}
        ]
    }))
    .unwrap();
    let ctx2 = EventContext::inbound(&board, &json!({"msg": "hello", "qty": 2}));
    let out2 = engine.run_recipes(&[recipe2], &Secrets::new(), &ctx2, false).await;
    assert!(out2[0].1.report.logs.is_empty(), "unexpected logs: {:?}", out2[0].1.report.logs);
    let records2 = db.srv_record_list_raw(&board, 100, None, 0).await.unwrap();
    assert!(records2.iter().any(|r| r.payload["note"] == json!("hello") && r.payload["qty"] == json!(2)));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn engine_transaction_is_atomic() {
    let dir = std::env::temp_dir().join(format!("zwtx_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = zw_db::DbPool::open(dir.join("test.db").to_str().unwrap()).await.unwrap();
    db.migrate().await.unwrap();
    db.migrate_session().await.unwrap();
    let board = db.srv_app_create("owner", "t", None, true, None).await.unwrap();

    db.srv_record_insert(&board, &json!({"sku": "A1", "stock": 10}), None, false).await.unwrap();
    db.srv_record_insert(&board, &json!({"sku": "A2", "stock": 5}), None, false).await.unwrap();

    let recipe = parse_recipe(&json!({
        "name": "tx-stock",
        "when": "record.created",
        "actions": [
            {"$transaction": [
                {"filter": [{"field": "sku", "op": "eq", "value": "A1"}], "patch": {"$dec": {"stock": "{{payload.qty}}"}}},
                {"filter": [{"field": "sku", "op": "eq", "value": "A2"}], "patch": {"$dec": {"stock": "{{payload.qty}}"}}}
            ]}
        ]
    }))
    .unwrap();
    let ctx = EventContext::record_created(&board, 3, &json!({"qty": 2}), None);
    let engine = Engine { db: &db };
    let out = engine.run_recipes(&[recipe], &Secrets::new(), &ctx, false).await;
    assert!(out[0].1.report.logs.is_empty(), "unexpected logs: {:?}", out[0].1.report.logs);

    let records = db.srv_record_list_raw(&board, 100, None, 0).await.unwrap();
    let a1 = records.iter().find(|r| r.payload["sku"] == json!("A1")).unwrap();
    let a2 = records.iter().find(|r| r.payload["sku"] == json!("A2")).unwrap();
    assert_eq!(a1.payload["stock"], json!(8));
    assert_eq!(a2.payload["stock"], json!(3));

    std::fs::remove_dir_all(&dir).ok();
}