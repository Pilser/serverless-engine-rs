use crate::model::{Board, Key, Principal, Record};
use crate::storage::database::{Database, Query, Row};
use crate::storage::ir::{normalize_path, path_items, scalar_text, FilterCond, Op, SrvFilter};
use crate::tables::{TABLE_APPS, TABLE_RECORDS};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn gen_board_id() -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = String::from("b_");
    let mut n = nanos;
    for _ in 0..16 {
        out.push(CHARS[(n % 36) as usize] as char);
        n /= 36;
    }
    out
}

fn sha256_hex(v: &Json) -> String {
    let mut h = Sha256::new();
    h.update(serde_json::to_string(v).unwrap_or_default().as_bytes());
    let digest = h.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn subtract_seconds(now: &str, secs: i64) -> String {
    match chrono::NaiveDateTime::parse_from_str(now, "%Y-%m-%d %H:%M:%S") {
        Ok(dt) => (dt - chrono::Duration::seconds(secs))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => now.to_string(),
    }
}

fn ttl_config(board: &Board) -> (Option<i64>, Option<String>) {
    let Ok(Json::Object(m)) = serde_json::to_value(board) else {
        return (None, None);
    };
    (
        m.get("ttl_seconds").and_then(|v| v.as_i64()),
        m.get("ttl_field").and_then(|v| v.as_str().map(String::from)),
    )
}

fn ttl_dead(seconds: Option<i64>, field: Option<&str>, created_at: &str, payload: &Json, now: &str) -> bool {
    let age_dead = match seconds {
        Some(n) if n > 0 && !created_at.is_empty() => created_at < subtract_seconds(now, n).as_str(),
        _ => false,
    };
    let field_dead = match field {
        Some(f) => {
            let value = crate::expr::get_path(payload, &normalize_path(f));
            !value.is_null() && scalar_text(&value).as_str() < now
        }
        None => false,
    };
    age_dead || field_dead
}

pub fn is_ttl_dead(board: &Board, created_at: &str, payload: &Json, now: &str) -> bool {
    let (seconds, field) = ttl_config(board);
    ttl_dead(seconds, field.as_deref(), created_at, payload, now)
}

pub fn records_ttl_alive(board: &Board, record: &Record, now: &str) -> bool {
    let created = record.created_at.as_deref().unwrap_or("");
    !is_ttl_dead(board, created, &record.payload, now)
}

pub fn board_cond(board_id: &str) -> FilterCond {
    FilterCond { field: "$.board_id".to_string(), op: Op::Eq, value: Json::String(board_id.to_string()) }
}

fn exact_filter(board_id: &str, seq: i64) -> SrvFilter {
    SrvFilter {
        conds: vec![
            board_cond(board_id),
            FilterCond { field: "$.seq".to_string(), op: Op::Eq, value: Json::from(seq) },
        ],
    }
}

fn scan_rows(db: &dyn Database, board_id: &str) -> anyhow::Result<Vec<Row>> {
    let q = Query {
        filter: SrvFilter { conds: vec![board_cond(board_id)] },
        orders: vec![],
        limit: usize::MAX,
        offset: 0,
    
        ttl: None,
    };
    Ok(db.query(TABLE_RECORDS, &q)?.rows)
}

fn stored_record_json(board_id: &str, seq: i64, payload: &Json, created_at: &str, writer: Option<&str>) -> Json {
    serde_json::json!({
        "board_id": board_id,
        "seq": seq,
        "payload": payload,
        "created_at": created_at,
        "writer": writer,
    })
}

fn find_record(db: &dyn Database, board_id: &str, seq: i64) -> anyhow::Result<Option<Record>> {
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if rec.seq == seq {
            return Ok(Some(rec));
        }
    }
    Ok(None)
}

fn next_seq(db: &dyn Database, board_id: &str) -> anyhow::Result<i64> {
    let mut max = 0i64;
    for row in scan_rows(db, board_id)? {
        if let Some(s) = row.data.get("seq").and_then(|v| v.as_i64()) {
            max = max.max(s);
        }
    }
    Ok(max + 1)
}

pub fn load_board(db: &dyn Database, board_id: &str) -> anyhow::Result<Board> {
    let row = db
        .get(TABLE_APPS, &Key::text(board_id))?
        .ok_or_else(|| anyhow::anyhow!("board {board_id} does not exist"))?;
    Ok(serde_json::from_value(row.data)?)
}

pub fn find_unique_holder(
    db: &dyn Database,
    board_id: &str,
    path: &str,
    text: &str,
    exclude_seq: Option<i64>,
) -> anyhow::Result<Option<i64>> {
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if let Some(e) = exclude_seq {
            if rec.seq == e {
                continue;
            }
        }
        let value = crate::expr::get_path(&rec.payload, path);
        if scalar_text(&value) == text {
            return Ok(Some(rec.seq));
        }
    }
    Ok(None)
}

pub fn app_create(
    db: &mut dyn Database,
    owner: &str,
    title: &str,
    schema: Option<Json>,
    public_reads: bool,
    unique_key: Option<&str>,
) -> anyhow::Result<Board> {
    let board_id = gen_board_id();
    let board = Board {
        board_id: board_id.clone(),
        owner_key: owner.to_string(),
        title: title.to_string(),
        schema_json: schema,
        public_reads,
        unique_key: unique_key.map(String::from),
        computed_json: None,
        validate_json: None,
        redact_json: None,
        rate_json: None,
        ttl_seconds: None,
        ttl_field: None,
        audit: false,
        webhook_secret: None,
        created_at: Some(now_str()),
    };
    db.insert(TABLE_APPS, Row::new(Key::text(board_id), serde_json::to_value(&board)?))?;
    Ok(board)
}

pub fn app_by_id(db: &dyn Database, board_id: &str) -> anyhow::Result<Option<Board>> {
    let Some(row) = db.get(TABLE_APPS, &Key::text(board_id))? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(row.data)?))
}

pub fn app_list(db: &dyn Database, owner: &str) -> anyhow::Result<Vec<Board>> {
    let q = Query {
        filter: SrvFilter {
            conds: vec![FilterCond {
                field: "$.owner_key".to_string(),
                op: Op::Eq,
                value: Json::String(owner.to_string()),
            }],
        },
        orders: vec![("$.created_at".to_string(), false)],
        limit: usize::MAX,
        offset: 0,
        ttl: None,
    };
    let mut out = Vec::new();
    for row in db.query(TABLE_APPS, &q)?.rows {
        out.push(serde_json::from_value(row.data)?);
    }
    Ok(out)
}

pub fn app_update(db: &mut dyn Database, board_id: &str, owner: &str, patch: &Json) -> anyhow::Result<()> {
    let board = app_by_id(db, board_id)?.ok_or_else(|| anyhow::anyhow!("board {board_id} not found"))?;
    if board.owner_key != owner {
        anyhow::bail!("board {board_id} is not owned by {owner}");
    }
    let mut updated = board;
    let obj = patch
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("patch must be a JSON object"))?;
    if let Some(v) = obj.get("title") {
        if let Some(s) = v.as_str() {
            updated.title = s.to_string();
        }
    }
    if let Some(v) = obj.get("schema") {
        updated.schema_json = if v.is_null() { None } else { Some(v.clone()) };
    }
    if let Some(v) = obj.get("public_reads") {
        if let Some(b) = v.as_bool() {
            updated.public_reads = b;
        }
    }
    if let Some(v) = obj.get("unique_key") {
        updated.unique_key = if v.is_null() { None } else { v.as_str().map(String::from) };
    }
    db.update(TABLE_APPS, &Key::text(board_id), &serde_json::to_value(&updated)?)?;
    Ok(())
}

pub fn app_delete(db: &mut dyn Database, board_id: &str, owner: &str) -> anyhow::Result<()> {
    let owned = match app_by_id(db, board_id)? {
        Some(b) => b.owner_key == owner,
        None => false,
    };
    if !owned {
        anyhow::bail!("board {board_id} is not owned by {owner}");
    }
    db.delete(TABLE_RECORDS, &SrvFilter { conds: vec![board_cond(board_id)] })?;
    db.delete(TABLE_APPS, &SrvFilter { conds: vec![board_cond(board_id)] })?;
    Ok(())
}

fn unique_holder(db: &dyn Database, board: &Board, payload: &Json) -> anyhow::Result<Option<i64>> {
    let Some(key) = board.unique_key.as_deref() else {
        return Ok(None);
    };
    let path = normalize_path(key);
    let value = crate::expr::get_path(payload, &path);
    if value.is_null() {
        return Ok(None);
    }
    let text = scalar_text(&value);
    find_unique_holder(db, &board.board_id, &path, &text, None)
}

pub fn record_insert(
    db: &mut dyn Database,
    board_id: &str,
    payload: Json,
    writer: Option<&str>,
    upsert: bool,
    principal: &Principal,
) -> anyhow::Result<i64> {
    let board = load_board(db, board_id)?;
    let mut prepared = crate::schema::prepare_payload(db, &board, payload)?;
    crate::auth::force_scope(principal, &mut prepared);
    if upsert {
        if let Some(existing) = unique_holder(db, &board, &prepared)? {
            record_set(&mut *db, board_id, existing, prepared, writer)?;
            return Ok(existing);
        }
    }
    crate::schema::check_unique(db, &board, &prepared, None)?;
    let seq = next_seq(db, board_id)?;
    let stored = stored_record_json(board_id, seq, &prepared, &now_str(), writer);
    db.insert(TABLE_RECORDS, Row::new(Key::Int(seq), stored))?;
    crate::audit::append(
        db,
        board_id,
        "created",
        Some(principal.id.as_str()),
        writer,
        Some(&sha256_hex(&prepared)),
    )?;
    crate::automation::dispatch(db, board_id, crate::events::EventKind::Created, Some(seq), Some(prepared.clone()))?;
    crate::webhooks::fire_hooks(db, board_id, &prepared)?;
    Ok(seq)
}

pub fn record_bulk_insert(
    db: &mut dyn Database,
    board_id: &str,
    records: Vec<Json>,
    writer: Option<&str>,
    upsert: bool,
    principal: &Principal,
) -> anyhow::Result<Vec<i64>> {
    let mut seqs = Vec::with_capacity(records.len());
    for payload in records {
        seqs.push(record_insert(&mut *db, board_id, payload, writer, upsert, principal)?);
    }
    Ok(seqs)
}

pub fn record_get(db: &dyn Database, board_id: &str, seq: i64) -> anyhow::Result<Option<Record>> {
    let board = load_board(db, board_id)?;
    let Some(mut rec) = find_record(db, board_id, seq)? else {
        return Ok(None);
    };
    let now = now_str();
    if !records_ttl_alive(&board, &rec, &now) {
        return Ok(None);
    }
    rec.payload = crate::schema::apply_redact(&board, &rec.payload);
    Ok(Some(rec))
}

pub fn record_set(
    db: &mut dyn Database,
    board_id: &str,
    seq: i64,
    payload: Json,
    writer: Option<&str>,
) -> anyhow::Result<()> {
    let board = load_board(db, board_id)?;
    let prepared = crate::schema::prepare_payload(db, &board, payload)?;
    crate::schema::check_unique(db, &board, &prepared, Some(seq))?;
    let current = find_record(db, board_id, seq)?
        .ok_or_else(|| anyhow::anyhow!("record seq {seq} does not exist"))?;
    let created_at = current.created_at.unwrap_or_default();
    let stored = stored_record_json(board_id, seq, &prepared, &created_at, writer);
    db.delete(TABLE_RECORDS, &exact_filter(board_id, seq))?;
    db.insert(TABLE_RECORDS, Row::new(Key::Int(seq), stored))?;
    crate::audit::append(db, board_id, "updated", None, writer, Some(&sha256_hex(&prepared)))?;
    crate::automation::dispatch(db, board_id, crate::events::EventKind::Updated, Some(seq), Some(prepared.clone()))?;
    crate::webhooks::fire_hooks(db, board_id, &prepared)?;
    Ok(())
}

pub fn record_patch(
    db: &mut dyn Database,
    board_id: &str,
    seq: i64,
    ops: &Json,
    writer: Option<&str>,
) -> anyhow::Result<Json> {
    let board = load_board(db, board_id)?;
    let current = find_record(db, board_id, seq)?
        .ok_or_else(|| anyhow::anyhow!("record seq {seq} does not exist"))?;
    let mut merged = current.payload.clone();
    apply_patch_ops(&mut merged, ops)?;
    crate::schema::validate_schema(&board, &merged)?;
    crate::schema::check_unique(db, &board, &merged, Some(seq))?;
    let resolved_writer = writer.or(current.writer.as_deref());
    let created_at = current.created_at.unwrap_or_default();
    let stored = stored_record_json(board_id, seq, &merged, &created_at, resolved_writer);
    db.delete(TABLE_RECORDS, &exact_filter(board_id, seq))?;
    db.insert(TABLE_RECORDS, Row::new(Key::Int(seq), stored))?;
    crate::audit::append(db, board_id, "updated", None, resolved_writer, Some(&sha256_hex(&merged)))?;
    crate::automation::dispatch(db, board_id, crate::events::EventKind::Updated, Some(seq), Some(merged.clone()))?;
    crate::webhooks::fire_hooks(db, board_id, &merged)?;
    Ok(merged)
}

pub fn record_patch_first(
    db: &mut dyn Database,
    board_id: &str,
    conds: &SrvFilter,
    ops: &Json,
) -> anyhow::Result<Option<Json>> {
    let board = load_board(db, board_id)?;
    let now = now_str();
    let mut recs = Vec::new();
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if records_ttl_alive(&board, &rec, &now) && conds.matches(&rec.payload) {
            recs.push(rec);
        }
    }
    recs.sort_by(|a, b| a.seq.cmp(&b.seq));
    let Some(rec) = recs.into_iter().next() else {
        return Ok(None);
    };
    let merged = record_patch(&mut *db, board_id, rec.seq, ops, None)?;
    Ok(Some(merged))
}

pub fn record_delete_one(db: &mut dyn Database, board_id: &str, seq: i64) -> anyhow::Result<bool> {
    let removed = db.delete(TABLE_RECORDS, &exact_filter(board_id, seq))? > 0;
    if removed {
        crate::audit::append(db, board_id, "deleted", None, None, None)?;
        crate::automation::dispatch(db, board_id, crate::events::EventKind::Deleted, Some(seq), None)?;
        crate::webhooks::fire_hooks(db, board_id, &Json::Null)?;
    }
    Ok(removed)
}

pub fn record_delete_filter(
    db: &mut dyn Database,
    board_id: &str,
    conds: &SrvFilter,
) -> anyhow::Result<usize> {
    let board = load_board(db, board_id)?;
    let now = now_str();
    let mut seqs = Vec::new();
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if records_ttl_alive(&board, &rec, &now) && conds.matches(&rec.payload) {
            seqs.push(rec.seq);
        }
    }
    if seqs.is_empty() {
        return Ok(0);
    }
    let filter = SrvFilter {
        conds: vec![
            board_cond(board_id),
            FilterCond {
                field: "$.seq".to_string(),
                op: Op::In,
                value: Json::Array(seqs.iter().map(|s| Json::from(*s)).collect()),
            },
        ],
    };
    Ok(db.delete(TABLE_RECORDS, &filter)?)
}

pub fn record_list(
    db: &dyn Database,
    board_id: &str,
    limit: usize,
    before: Option<i64>,
    offset: usize,
) -> anyhow::Result<Vec<Record>> {
    let board = load_board(db, board_id)?;
    let limit = limit.clamp(1, 200);
    let now = now_str();
    let mut recs = Vec::new();
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if !records_ttl_alive(&board, &rec, &now) {
            continue;
        }
        if let Some(b) = before {
            if rec.seq >= b {
                continue;
            }
        }
        recs.push(rec);
    }
    recs.sort_by(|a, b| b.seq.cmp(&a.seq));
    let start = offset.min(recs.len());
    let end = (start + limit).min(recs.len());
    let mut out = Vec::new();
    for rec in &recs[start..end] {
        let mut r = rec.clone();
        r.payload = crate::schema::apply_redact(&board, &r.payload);
        out.push(r);
    }
    Ok(out)
}

pub fn record_count(db: &dyn Database, board_id: &str) -> anyhow::Result<i64> {
    let board = load_board(db, board_id)?;
    let now = now_str();
    let mut n = 0i64;
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if records_ttl_alive(&board, &rec, &now) {
            n += 1;
        }
    }
    Ok(n)
}

pub fn records_after(db: &dyn Database, board_id: &str, after: i64) -> anyhow::Result<Vec<Record>> {
    let board = load_board(db, board_id)?;
    let now = now_str();
    let mut recs = Vec::new();
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if rec.seq > after && records_ttl_alive(&board, &rec, &now) {
            recs.push(rec);
        }
    }
    recs.sort_by(|a, b| a.seq.cmp(&b.seq));
    recs.truncate(1000);
    for rec in &mut recs {
        rec.payload = crate::schema::apply_redact(&board, &rec.payload);
    }
    Ok(recs)
}

pub fn record_set_raw(
    db: &mut dyn Database,
    board_id: &str,
    seq: i64,
    payload: Json,
) -> anyhow::Result<()> {
    let current = find_record(db, board_id, seq)?
        .ok_or_else(|| anyhow::anyhow!("record seq {seq} does not exist"))?;
    let created_at = current.created_at.unwrap_or_default();
    let stored = stored_record_json(board_id, seq, &payload, &created_at, None);
    db.delete(TABLE_RECORDS, &exact_filter(board_id, seq))?;
    db.insert(TABLE_RECORDS, Row::new(Key::Int(seq), stored))?;
    Ok(())
}

pub fn record_patch_first_raw(
    db: &mut dyn Database,
    board_id: &str,
    conds: &SrvFilter,
    ops: &Json,
) -> anyhow::Result<Option<Json>> {
    let board = load_board(db, board_id)?;
    let now = now_str();
    let mut recs = Vec::new();
    for row in scan_rows(db, board_id)? {
        let rec: Record = serde_json::from_value(row.data)?;
        if records_ttl_alive(&board, &rec, &now) && conds.matches(&rec.payload) {
            recs.push(rec);
        }
    }
    recs.sort_by(|a, b| a.seq.cmp(&b.seq));
    let Some(rec) = recs.into_iter().next() else {
        return Ok(None);
    };
    let mut merged = rec.payload.clone();
    apply_patch_ops(&mut merged, ops)?;
    let created_at = rec.created_at.unwrap_or_default();
    let stored = stored_record_json(board_id, rec.seq, &merged, &created_at, rec.writer.as_deref());
    db.delete(TABLE_RECORDS, &exact_filter(board_id, rec.seq))?;
    db.insert(TABLE_RECORDS, Row::new(Key::Int(rec.seq), stored))?;
    Ok(Some(merged))
}

fn norm_path(path: &str) -> String {
    if path.starts_with('$') {
        path.to_string()
    } else {
        format!("$.{path}")
    }
}

fn int_json(n: f64) -> serde_json::Number {
    if n.fract() == 0.0 && n >= 0.0 {
        serde_json::Number::from(n as u64)
    } else if n.fract() == 0.0 {
        serde_json::Number::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).expect("finite")
    }
}

fn apply_arith(target: &mut Json, value: &Json, sign: f64) -> anyhow::Result<()> {
    let fields = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("$inc/$dec requires an object of {{path: delta}}"))?;
    for (path, delta) in fields {
        let d = delta
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("$inc/$dec delta for '{path}' must be a number"))?;
        let p = norm_path(path);
        let base = crate::expr::get_path(target, &p).as_f64().unwrap_or(0.0) + sign * d;
        crate::expr::set_path(target, &p, Json::Number(int_json(base)))?;
    }
    Ok(())
}

fn apply_mul(target: &mut Json, value: &Json) -> anyhow::Result<()> {
    let fields = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("$mul requires an object of {{path: factor}}"))?;
    for (path, factor) in fields {
        let f = factor
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("$mul factor for '{path}' must be a number"))?;
        let p = norm_path(path);
        let base = crate::expr::get_path(target, &p).as_f64().unwrap_or(0.0) * f;
        crate::expr::set_path(target, &p, Json::Number(int_json(base)))?;
    }
    Ok(())
}

pub(crate) fn unset_path(target: &mut Json, path: &str) {
    let Some(segs) = path_items(path) else {
        return;
    };
    let n = segs.len();
    if n == 0 {
        return;
    }
    let mut cur = target;
    for seg in &segs[..n - 1] {
        match seg {
            crate::storage::ir::PathSeg::Key(k) => {
                let Some(obj) = cur.as_object_mut() else { return };
                let Some(next) = obj.get_mut(k) else { return };
                cur = next;
            }
            crate::storage::ir::PathSeg::Idx(i) => {
                let Some(arr) = cur.as_array_mut() else { return };
                if *i >= arr.len() {
                    return;
                }
                cur = &mut arr[*i];
            }
        }
    }
    match &segs[n - 1] {
        crate::storage::ir::PathSeg::Key(k) => {
            if let Some(obj) = cur.as_object_mut() {
                obj.remove(k);
            }
        }
        crate::storage::ir::PathSeg::Idx(i) => {
            if let Some(arr) = cur.as_array_mut() {
                if *i < arr.len() {
                    arr.remove(*i);
                }
            }
        }
    }
}

fn apply_patch_ops(target: &mut Json, ops: &Json) -> anyhow::Result<()> {
    let map = ops
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("patch ops must be an object"))?;
    for (key, value) in map {
        match key.as_str() {
            "$set" => {
                let fields = value
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("$set needs an object of {{path: value}}"))?;
                for (path, val) in fields {
                    crate::expr::set_path(target, &norm_path(path), val.clone())?;
                }
            }
            "$unset" => {
                let paths: Vec<String> = match value {
                    Json::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                    Json::Object(m) => m.keys().cloned().collect(),
                    other => anyhow::bail!("$unset requires an array of field paths, got {other}"),
                };
                for path in paths {
                    unset_path(target, &path);
                }
            }
            "$inc" => apply_arith(target, value, 1.0)?,
            "$dec" => apply_arith(target, value, -1.0)?,
            "$mul" => apply_mul(target, value)?,
            other => crate::expr::set_path(target, &norm_path(other), value.clone())?,
        }
    }
    Ok(())
}