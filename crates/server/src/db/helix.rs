//! `Database` adapter backed by HelixDB (`POST /v2/query`), one Helix tenant
//! per board. Engine metadata (`wb_*`) lives in the `__srv__` label namespace;
//! user tables get their raw label. See docs/helixdb-integration-plan.md §4.
//!
//! Data model: every engine row becomes ONE Helix node with:
//!   - `data`: the full engine JSON (lossless round-trip)
//!   - `board_id`, `table`, `seq`: scalar routing props for pushdown filters
//!   - `tenantId`: the tenant scope (added by the request builder)
//! The engine `Key` is stored in `_srv_key` so get-by-key is an exact lookup.
//! Filters Helix cannot express (contains/not_contains/search) are applied in
//! Rust after fetching candidate rows.

use engine::model::Key;
use engine::storage::database::{Cursor, Database, DatabaseCaps, Query, Row, TtlClause};
use engine::storage::ir::{scalar_text, FilterCond, Op, SrvFilter};
use engine::tables::{TABLE_APPS, TABLE_AUDIT, TABLE_LINKS};
use helixdb::namespaces::engine_label;
use helixdb::request;
use helixdb::tenant::Tenant;
use helixdb::Client;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::Mutex;

const DATA_PROP: &str = "data";
const SEQ_PROP: &str = "seq";
const BOARD_PROP: &str = "board_id";
const TABLE_PROP: &str = "table";
const SEARCH_PROP: &str = "_search";

/// Flatten a record's payload into a single searchable text blob (all scalar
/// string/number values joined with spaces). Stored in the `_search` prop and
/// indexed with BM25 so `Op::Search` can use Helix full-text search.
fn search_text(data: &Json) -> String {
    let mut parts: Vec<String> = Vec::new();
    fn walk(v: &Json, parts: &mut Vec<String>) {
        match v {
            Json::Object(map) => {
                for (k, val) in map {
                    if k == "board_id" || k == "table" || k == "created_at" || k == "writer" {
                        continue;
                    }
                    walk(val, parts);
                }
            }
            Json::Array(arr) => {
                for v in arr {
                    walk(v, parts);
                }
            }
            Json::String(s) => parts.push(s.clone()),
            Json::Number(n) => parts.push(n.to_string()),
            Json::Bool(b) => parts.push(b.to_string()),
            Json::Null => {}
        }
    }
    if let Some(payload) = data.get("payload") {
        walk(payload, &mut parts);
    } else {
        walk(data, &mut parts);
    }
    parts.join(" ")
}

/// Numeric mirror props: top-level numeric payload fields are mirrored as
/// `p_<name>` node properties so Helix can aggregate them (`sum`/`avg`/`min`/
/// `max`) without seeing inside the `data` blob. Bounded to scalars; nested
/// paths are left to the Rust fallback.
fn numeric_mirror_props(data: &Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    if let Some(payload) = data.get("payload").and_then(|p| p.as_object()) {
        for (k, v) in payload {
            if v.is_number() {
                let name = format!("p_{k}");
                out.push((name, v.clone()));
            }
        }
    }
    out
}

/// Map an engine payload path (e.g. `grade` or `$.grade`) to its mirrored
/// Helix prop name, if it was mirrored at write time.
fn mirror_prop_for(path: &str) -> Option<String> {
    let stripped = path.strip_prefix("$.").unwrap_or(path);
    if stripped.contains('.') {
        return None; // nested paths are not mirrored
    }
    Some(format!("p_{stripped}"))
}

/// The single shared board registry table (global).
fn is_apps(table: &str) -> bool {
    table == TABLE_APPS
}

fn board_from_key(table: &str, k: &Key) -> Option<String> {
    let Key::Text(s) = k else {
        return None;
    };
    if is_apps(table) || table == TABLE_AUDIT || table == TABLE_LINKS {
        return Some(s.clone());
    }
    s.split_once('/').map(|(b, _)| b.to_string())
}

/// Board routing with a `data` fallback: tables like `wb_keys`, `wb_sessions`,
/// `wb_hook_deliveries` and `wb_job_runs` use random/uuid keys that don't
/// encode the board, so the row's `data.board_id` field is authoritative.
fn board_from_row(table: &str, key: &Key, data: &Json) -> Option<String> {
    board_from_key(table, key).or_else(|| {
        data.get("board_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

fn board_from_filter(f: &SrvFilter) -> Option<String> {
    for c in &f.conds {
        if c.field == "$.board_id" && c.op == Op::Eq {
            if let Some(s) = c.value.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn filter_owner(f: &SrvFilter) -> Option<String> {
    for c in &f.conds {
        if c.field == "$.owner_key" && c.op == Op::Eq {
            if let Some(s) = c.value.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// A `$.a.b` engine field becomes the Helix property name (strip the `$.`
/// prefix; nested paths are not flattened by Helix, so we use the engine
/// `data` blob for anything that is not a known routing prop).
fn helix_prop(field: &str) -> Option<&'static str> {
    match field {
        "$.board_id" => Some(BOARD_PROP),
        "$.table" => Some(TABLE_PROP),
        "$.seq" => Some(SEQ_PROP),
        _ => None,
    }
}

/// Like [`helix_prop`] but for ORDER fields — returns the node prop name that
/// Helix can order by, or `None` when the field is only inside `data` (then the
/// adapter sorts in Rust).
fn helix_prop_order(field: &str) -> Option<&'static str> {
    helix_prop(field)
}

/// Translate engine conditions into a Helix predicate over routing props +
/// `data`-JSON-path (best-effort). Returns `(predicate, rest)` where `rest`
/// are conditions to apply in Rust.
fn to_predicate(conds: &[FilterCond], tenant: &str, label: &str) -> (Json, Vec<FilterCond>) {
    let mut pred = helixdb::predicate::Predicate::new()
        .eq("$label", json!(label))
        .eq(helixdb::tenant::TENANT_PROP, json!(tenant));
    let mut rest = Vec::new();
    for c in conds {
        let translated = match c.op {
            Op::Eq | Op::Ne | Op::Gt | Op::Gte | Op::Lt | Op::Lte | Op::In => {
                match helix_prop(&c.field) {
                    Some(prop) => helixdb::predicate::from_cond(prop, c.op.as_str(), &c.value),
                    None => None,
                }
            }
            _ => None,
        };
        match translated {
            Some(p) => pred = pred.and_raw(p),
            None => rest.push(c.clone()),
        }
    }
    (pred.into_json(), rest)
}

/// Node JSON -> engine Row. `data` holds the engine JSON (stored as a string);
/// `_srv_key` restores the engine key.
fn node_to_row(node: &Json, table: &str) -> anyhow::Result<Row> {
    let data = match node.get(DATA_PROP) {
        Some(Json::String(s)) => serde_json::from_str(s).unwrap_or(Json::Null),
        Some(other) => other.clone(),
        None => node.clone(),
    };
    let key = match node.get(helixdb::row::KEY_PROP) {
        Some(Json::String(s)) => Key::text(s.clone()),
        Some(Json::Number(n)) => Key::int(n.as_i64().unwrap_or(0)),
        _ => {
            let id = node.get("$id").and_then(|v| v.as_i64()).unwrap_or(0);
            Key::int(id)
        }
    };
    let _ = table;
    Ok(Row::new(key, data))
}

fn rest_matches(c: &FilterCond, row: &Row) -> bool {
    // Engine record filters target PAYLOAD paths (e.g. `grade` means
    // `payload.grade`), so when the field is not found at the row's top level,
    // resolve it inside the `payload` object.
    let mut actual = engine::expr::get_path(&row.data, &c.field);
    if actual.is_null() {
        if let Some(payload) = row.data.get("payload") {
            actual = engine::expr::get_path(payload, &c.field);
        }
    }
    let text = scalar_text(&actual);
    let want = scalar_text(&c.value);
    // Comparison ops use NUMERIC comparison when both sides are numbers
    // ("20" <= "5" is false numerically but true lexically).
    let cmp_num = |f: fn(f64, f64) -> bool| -> bool {
        match (actual.as_f64(), c.value.as_f64()) {
            (Some(a), Some(b)) => f(a, b),
            _ => false,
        }
    };
    match c.op {
        Op::Eq => text == want,
        Op::Ne => text != want,
        Op::Gt => cmp_num(|a, b| a > b) || (!actual.is_number() && text > want),
        Op::Gte => cmp_num(|a, b| a >= b) || (!actual.is_number() && text >= want),
        Op::Lt => cmp_num(|a, b| a < b) || (!actual.is_number() && text < want),
        Op::Lte => cmp_num(|a, b| a <= b) || (!actual.is_number() && text <= want),
        Op::In => c
            .value
            .as_array()
            .map(|arr| arr.iter().any(|v| {
                match (actual.as_f64(), v.as_f64()) {
                    (Some(a), Some(b)) => a == b,
                    _ => text == scalar_text(v),
                }
            }))
            .unwrap_or(false),
        Op::Contains => text.to_lowercase().contains(&want.to_lowercase()),
        Op::NotContains => !text.to_lowercase().contains(&want.to_lowercase()),
        Op::Search => engine::expr::search_match(&row.data, &want),
    }
}

fn sort_and_slice(mut rows: Vec<Row>, orders: &[(String, bool)], limit: usize, offset: usize) -> Cursor {
    if orders.is_empty() {
        rows.sort_by(|a, b| b.key.cmp(&a.key));
    } else {
        rows.sort_by(|a, b| compare_rows(a, b, orders));
    }
    let start = offset.min(rows.len());
    let has_more = start + limit < rows.len();
    let end = (start + limit).min(rows.len());
    Cursor { rows: rows[start..end].to_vec(), has_more }
}

fn compare_scalar(a: &Json, b: &Json) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => scalar_text(a).cmp(&scalar_text(b)),
    }
}

fn compare_rows(a: &Row, b: &Row, orders: &[(String, bool)]) -> std::cmp::Ordering {
    for (field, desc) in orders {
        let mut va = engine::expr::get_path(&a.data, field);
        let mut vb = engine::expr::get_path(&b.data, field);
        if va.is_null() {
            if let Some(payload) = a.data.get("payload") {
                va = engine::expr::get_path(payload, field);
            }
        }
        if vb.is_null() {
            if let Some(payload) = b.data.get("payload") {
                vb = engine::expr::get_path(payload, field);
            }
        }
        let ord = compare_scalar(&va, &vb);
        if ord != std::cmp::Ordering::Equal {
            return if *desc { ord.reverse() } else { ord };
        }
    }
    b.key.cmp(&a.key)
}

/// Routing props for a row (board_id/table/seq derived from `data`).
fn routing_props(data: &Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    for (prop, key) in [(BOARD_PROP, "board_id"), (TABLE_PROP, "table"), (SEQ_PROP, "seq")] {
        if let Some(v) = data.get(key) {
            if !v.is_null() {
                out.push((prop.to_string(), v.clone()));
            }
        }
    }
    out
}

pub struct HelixDatabase {
    client: Client,
    _boards: Mutex<HashMap<String, Tenant>>,
}

impl HelixDatabase {
    pub fn new(base_url: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::new(base_url)?,
            _boards: Mutex::new(HashMap::new()),
        })
    }

    fn tenant(&self, board_id: &str) -> Tenant {
        Tenant::from_board(board_id)
    }

    /// Fetch rows for a label within a tenant. Applies Helix-pushable
    /// predicates; the rest are applied in Rust. Returns engine Rows.
    fn fetch_rows(
        &self,
        tenant: &str,
        label: &str,
        conds: &[FilterCond],
        limit: usize,
        offset: usize,
        orders: &[(String, bool)],
    ) -> anyhow::Result<Cursor> {
        let (pred, rest) = to_predicate(conds, tenant, label);
        let src = json!({ "nodes_where": { "predicate": pred } });
        // Order fields that map to real node props (seq/board_id/table) push
        // down; everything else (payload paths) is sorted in Rust after fetch.
        let (pushable, rust_orders): (Vec<(String, bool)>, Vec<(String, bool)>) = orders
            .iter()
            .cloned()
            .partition(|(f, _)| helix_prop_order(f).is_some());
        let pushable: Vec<(String, bool)> = pushable
            .into_iter()
            .map(|(f, d)| (helix_prop_order(&f).unwrap_or(&f).to_string(), d))
            .collect();
        // When any order field is not pushable, pagination must happen in Rust
        // (the Helix limit/offset would slice before the Rust sort).
        let (helix_limit, helix_offset) = if rust_orders.is_empty() {
            (limit, offset)
        } else {
            (usize::MAX, 0)
        };
        let shaped = request::order_and_page(src, &pushable, helix_limit, helix_offset);
        let root = request::value_map(shaped, vec!["$id", DATA_PROP, helixdb::row::KEY_PROP]);
        let batch = request::read_batch(vec![request::entry("rows", root)], vec!["rows"]);
        let resp = self.client.post("read", "fetch_rows", batch)?;
        let mut rows = Vec::new();
        for node in resp.rows("rows") {
            let row = node_to_row(&node, label)?;
            if rest.iter().any(|c| !rest_matches(c, &row)) {
                continue;
            }
            rows.push(row);
        }
        if !rust_orders.is_empty() {
            rows.sort_by(|a, b| compare_rows(a, b, &rust_orders));
        }
        let start = offset.min(rows.len());
        let has_more = start + limit < rows.len();
        let end = (start + limit).min(rows.len());
        Ok(Cursor { rows: rows[start..end].to_vec(), has_more })
    }

    /// Write a row as a tenant-scoped node, replacing the node that matches
    /// `_srv_key` if one exists. The node stores:
    ///   - `data`: the full engine JSON (as a string — Helix scalar properties)
    ///   - routing props (`board_id`/`table`/`seq`) for pushdown filters
    ///   - `_srv_key`: the engine key, scalar, for exact get-by-key
    ///   - `_search`: (records only) flattened payload text for BM25
    fn write_node(&self, tenant: &str, label: &str, row: &Row) -> anyhow::Result<i64> {
        let mut props: Vec<(String, Json)> = Vec::new();
        props.push((DATA_PROP.to_string(), json!(row.data.to_string())));
        for (k, v) in routing_props(&row.data) {
            props.push((k, v));
        }
        if label == engine_label("wb_records") {
            props.push((SEARCH_PROP.to_string(), json!(search_text(&row.data))));
            for (k, v) in numeric_mirror_props(&row.data) {
                props.push((k, v));
            }
        }
        let keyval_scalar = match &row.key {
            Key::Int(v) => json!(*v),
            Key::Text(s) => json!(s),
        };
        props.push((helixdb::row::KEY_PROP.to_string(), keyval_scalar));

        // Existing node lookup by _srv_key (exact).
        let keyval = match &row.key {
            Key::Int(v) => json!({ "i64": v }),
            Key::Text(s) => json!({ "string": s }),
        };
        let existing = json!({
            "query": { "name": "existing", "root": {
                "nodes_where": { "predicate": {
                    "and": { "predicates": [
                        { "eq": { "left": { "property": "$label" }, "right": { "constant": { "string": label } } } },
                        { "eq": { "left": { "property": helixdb::tenant::TENANT_PROP }, "right": { "constant": { "string": tenant } } } },
                        { "eq": { "left": { "property": helixdb::row::KEY_PROP }, "right": { "constant": keyval } } }
                    ]}
                }}
            }}
        });
        // New node (only when no existing).
        let created = json!({
            "query": { "name": "created", "condition": { "var_empty": "existing" },
                "root": request::add_n(label, tenant, &props) }
        });
        // Update the existing node's data (only when existing present). For
        // records, also refresh the `_search` blob + numeric mirrors.
        let updated_input = json!({ "nodes": { "reference": { "var": "existing" } } });
        let mut updated = request::set_property(updated_input.clone(), DATA_PROP, &row.data);
        if label == engine_label("wb_records") {
            updated = request::set_property(updated_input.clone(), SEARCH_PROP, &json!(search_text(&row.data)));
            for (k, v) in numeric_mirror_props(&row.data) {
                updated = request::set_property(updated_input.clone(), &k, &v);
            }
        }
        let updated = json!({
            "query": { "name": "updated", "condition": { "var_not_empty": "existing" },
                "root": updated }
        });
        let batch = request::write_batch(vec![existing, created, updated], vec!["created", "updated"]);
        let resp = self.client.post("write", "write_node", batch)?;
        let created = resp.get("created").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let _ = created;
        Ok(0)
    }

    fn get_by_key(&self, tenant: &str, label: &str, keyval: Json) -> anyhow::Result<Option<Row>> {
        let src = json!({
            "nodes_where": { "predicate": {
                "and": { "predicates": [
                    { "eq": { "left": { "property": "$label" }, "right": { "constant": { "string": label } } } },
                    { "eq": { "left": { "property": helixdb::tenant::TENANT_PROP }, "right": { "constant": { "string": tenant } } } },
                    { "eq": { "left": { "property": helixdb::row::KEY_PROP }, "right": { "constant": keyval } } }
                ]}
            }}
        });
        let root = request::value_map(src, vec!["$id", DATA_PROP, helixdb::row::KEY_PROP]);
        let batch = request::read_batch(vec![request::entry("row", root)], vec!["row"]);
        let resp = self.client.post("read", "get", batch)?;
        let mut nodes = resp.rows("row");
        if nodes.is_empty() {
            return Ok(None);
        }
        node_to_row(&nodes.swap_remove(0), label).map(Some)
    }
}

impl Database for HelixDatabase {
    fn adapter(&self) -> &'static str {
        "helix"
    }

    fn capabilities(&self) -> DatabaseCaps {
        let mut c = DatabaseCaps::default();
        c.fts.bm25 = true;
        c.fts.phrase = true;
        c.vector = true;
        c.json.json_path = true;
        c
    }

    fn insert(&mut self, table: &str, row: Row) -> anyhow::Result<i64> {
        let table = table.to_string();
        let (tenant, label) = if is_apps(&table) {
            // Board registry lives in its own tenant (the board id).
            let board = row
                .data
                .get("board_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| board_from_key(&table, &row.key))
                .ok_or_else(|| anyhow::anyhow!("wb_apps insert missing board_id"))?;
            (self.tenant(&board), engine_label("wb_apps"))
        } else if table == "wb_records" {
            // Records carry board_id in the data blob; the key is just the seq.
            let board = row
                .data
                .get("board_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("wb_records insert missing board_id"))?
                .to_string();
            (self.tenant(&board), engine_label("wb_records"))
        } else {
            let board = board_from_row(&table, &row.key, &row.data)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} insert"))?;
            (self.tenant(&board), engine_label(&table))
        };
        self.write_node(tenant.as_str(), &label, &row)
    }

    fn get(&self, table: &str, pk: &Key) -> anyhow::Result<Option<Row>> {
        let table = table.to_string();
        if is_apps(&table) {
            let Key::Text(board) = pk else {
                return Ok(None);
            };
            let tenant = self.tenant(board);
            let keyval = json!({ "string": board });
            return self.get_by_key(tenant.as_str(), &engine_label("wb_apps"), keyval);
        }
        let board = board_from_key(&table, pk)
            .ok_or_else(|| anyhow::anyhow!("cannot route {table} get"))?;
        let tenant = self.tenant(&board);
        let keyval = match pk {
            Key::Int(v) => json!({ "i64": v }),
            Key::Text(s) => json!({ "string": s }),
        };
        self.get_by_key(tenant.as_str(), &engine_label(&table), keyval)
    }

    fn update(&mut self, table: &str, pk: &Key, patch: &Json) -> anyhow::Result<()> {
        let table = table.to_string();
        let board = if is_apps(&table) {
            let Key::Text(b) = pk else {
                return Ok(());
            };
            b.clone()
        } else {
            board_from_key(&table, pk)
                .or_else(|| patch.get("board_id").and_then(|v| v.as_str()).map(str::to_string))
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} update"))?
        };
        let tenant = self.tenant(&board);
        let keyval = match pk {
            Key::Int(v) => json!({ "i64": v }),
            Key::Text(s) => json!({ "string": s }),
        };
        let src = json!({
            "nodes_where": { "predicate": {
                "and": { "predicates": [
                    { "eq": { "left": { "property": "$label" }, "right": { "constant": { "string": engine_label(&table) } } } },
                    { "eq": { "left": { "property": helixdb::tenant::TENANT_PROP }, "right": { "constant": { "string": tenant.as_str() } } } },
                    { "eq": { "left": { "property": helixdb::row::KEY_PROP }, "right": { "constant": keyval } } }
                ]}
            }}
        });
        let root = request::set_property(src, DATA_PROP, patch);
        let batch = request::write_batch(vec![request::entry("row", root)], vec!["row"]);
        let _ = self.client.post("write", "update", batch)?;
        Ok(())
    }

    fn delete(&mut self, table: &str, filter: &SrvFilter) -> anyhow::Result<usize> {
        let table = table.to_string();
        let (pred, rest) = if is_apps(&table) {
            // Delete across every board tenant that matches.
            let mut deleted = 0usize;
            for b in self.all_board_ids()? {
                let tenant = self.tenant(&b);
                let (p, r) = to_predicate(&filter.conds, tenant.as_str(), &engine_label("wb_apps"));
                deleted += self.drop_where(&p, &r)?;
            }
            return Ok(deleted);
        } else {
            let board = board_from_filter(filter)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} delete"))?;
            let tenant = self.tenant(&board);
            to_predicate(&filter.conds, tenant.as_str(), &engine_label(&table))
        };
        self.drop_where(&pred, &rest)
    }

    fn query(&self, table: &str, q: &Query) -> anyhow::Result<Cursor> {
        let table = table.to_string();
        if is_apps(&table) {
            let owner = filter_owner(&q.filter);
            let boards: Vec<String> = match owner {
                Some(o) => self.boards_by_owner(&o)?,
                None => self.all_board_ids()?,
            };
            let mut rows = Vec::new();
            for b in boards {
                let tenant = self.tenant(&b);
                let conds: Vec<FilterCond> = q
                    .filter
                    .conds
                    .iter()
                    .filter(|c| c.field != "$.owner_key" && c.field != "$.board_id")
                    .cloned()
                    .collect();
                let cur = self.fetch_rows(tenant.as_str(), &engine_label("wb_apps"), &conds, usize::MAX, 0, &[])?;
                rows.extend(cur.rows);
            }
            return Ok(sort_and_slice(rows, &q.orders, q.limit, q.offset));
        }
        let board = board_from_filter(&q.filter)
            .ok_or_else(|| anyhow::anyhow!("cannot route {table} query"))?;
        let tenant = self.tenant(&board);
        let label = engine_label(&table);

        // Full-text search: Op::Search on wb_records -> BM25 via Helix.
        if table == "wb_records" {
            if let Some(search) = q.filter.conds.iter().find(|c| c.op == Op::Search) {
                let query_text = scalar_text(&search.value);
                if !query_text.trim().is_empty() {
                    // Ensure the BM25 index exists (fire-and-forget; creation
                    // is async and `if_not_exists` makes it idempotent).
                    let _ = self.ensure_text_index(tenant.as_str(), &label);
                    let extra: Vec<FilterCond> = q
                        .filter
                        .conds
                        .iter()
                        .filter(|c| c.op != Op::Search)
                        .filter(|c| c.field != "$.board_id" && c.field != "$.table")
                        .cloned()
                        .collect();
                    match self.search_rows(tenant.as_str(), &label, &query_text, q.limit, q.offset, &extra) {
                        Ok(cursor) => return Ok(cursor),
                        // Index not built yet (or not yet searchable): fall
                        // back to the Rust scan path so search still works.
                        Err(_) => {}
                    }
                }
            }
        }
        let conds: Vec<FilterCond> = q
            .filter
            .conds
            .iter()
            .filter(|c| c.field != "$.board_id" && c.field != "$.table")
            .cloned()
            .collect();
        // Records default to seq DESC (mirrors turso and the engine's
        // `next_seq` which reads the first row as the max seq).
        let mut orders = q.orders.clone();
        if table == "wb_records" && orders.is_empty() {
            orders.push(("$.seq".to_string(), true));
        }
        self.fetch_rows(tenant.as_str(), &label, &conds, q.limit, q.offset, &orders)
    }

    fn upsert(&mut self, table: &str, key: &str, row: Row) -> anyhow::Result<i64> {
        let table = table.to_string();
        let board = board_from_row(&table, &row.key, &row.data)
            .ok_or_else(|| anyhow::anyhow!("cannot route {table} upsert"))?;
        let tenant = self.tenant(&board);
        let label = engine_label(&table);
        let unique = scalar_text(&engine::expr::get_path(&row.data, key));
        let conds = vec![FilterCond {
            field: key.to_string(),
            op: Op::Eq,
            value: Json::String(unique),
        }];
        let existing = self.fetch_rows(tenant.as_str(), &label, &conds, 2, 0, &[])?.rows;
        match existing.first() {
            Some(e) => {
                let mut row = row;
                row.key = e.key.clone();
                self.write_node(tenant.as_str(), &label, &row)?;
            }
            None => {
                self.write_node(tenant.as_str(), &label, &row)?;
            }
        }
        Ok(0)
    }

    fn begin(&mut self, _board: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn commit(&mut self, _board: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn rollback(&mut self, _board: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn aggregate(
        &self,
        table: &str,
        board: &str,
        table_name: &str,
        filter: &SrvFilter,
        agg: engine::storage::ir::Agg,
        field: Option<&str>,
        group_by: Option<&str>,
        ttl: Option<TtlClause>,
    ) -> anyhow::Result<Vec<Json>> {
        // Push down to Helix when safe: records, no TTL filter, and every
        // aggregate target maps to a mirrored/routing node prop. Otherwise
        // fall back to the Rust aggregation over a tenant-scoped fetch.
        if table == "wb_records" && ttl.is_none() && field.is_none_or(|f| mirror_prop_for(f).is_some())
            && group_by.is_none_or(|g| mirror_prop_for(g).is_some())
        {
            if let Some(out) = self.aggregate_pushdown(board, table_name, filter, agg, field, group_by)? {
                return Ok(out);
            }
        }
        let q = Query {
            filter: SrvFilter {
                conds: vec![
                    FilterCond { field: "$.board_id".to_string(), op: Op::Eq, value: Json::String(board.to_string()) },
                    FilterCond { field: "$.table".to_string(), op: Op::Eq, value: Json::String(table_name.to_string()) },
                ],
            },
            orders: vec![],
            limit: usize::MAX,
            offset: 0,
            ttl: ttl.clone(),
        };
        let rows = self.query(table, &q)?.rows;
        engine::storage::database::aggregate_rows(&rows, filter, agg, field, group_by, ttl)
    }
}

impl HelixDatabase {
    /// Drop nodes matching `pred`; apply `rest` conditions in Rust by fetching
    /// candidates first. Helix's `drop` returns an empty array, so we fetch the
    /// matching ids first to report an accurate count.
    fn drop_where(&self, pred: &Json, rest: &[FilterCond]) -> anyhow::Result<usize> {
        // Fetch candidates (ids + data), filter in Rust if needed.
        let src = json!({ "nodes_where": { "predicate": pred } });
        let root = request::value_map(src, vec!["$id", DATA_PROP, helixdb::row::KEY_PROP]);
        let batch = request::read_batch(vec![request::entry("rows", root)], vec!["rows"]);
        let resp = self.client.post("read", "delete_candidates", batch)?;
        let mut ids = Vec::new();
        for node in resp.rows("rows") {
            let row = node_to_row(&node, "")?;
            if rest.iter().any(|c| !rest_matches(c, &row)) {
                continue;
            }
            if let Some(id) = node.get("$id").and_then(|v| v.as_i64()) {
                ids.push(id);
            }
        }
        let count = ids.len();
        if ids.is_empty() {
            return Ok(0);
        }
        let src = json!({ "nodes": { "reference": { "ids": ids } } });
        let root = request::drop(src);
        let batch = request::write_batch(vec![request::entry("rows", root)], vec!["rows"]);
        let _ = self.client.post("write", "delete", batch)?;
        Ok(count)
    }

    pub fn all_board_ids(&self) -> anyhow::Result<Vec<String>> {
        // Every board tenant has exactly one wb_apps node; scan the reserved
        // apps label WITHOUT tenant scope (this is the one global registry).
        let src = json!({
            "nodes_where": { "predicate": {
                "eq": { "left": { "property": "$label" }, "right": { "constant": { "string": engine_label("wb_apps") } } }
            }}
        });
        let root = request::value_map(src, vec!["$id", DATA_PROP, "board_id"]);
        let batch = request::read_batch(vec![request::entry("boards", root)], vec!["boards"]);
        let resp = self.client.post("read", "all_boards", batch)?;
        let mut out = Vec::new();
        for node in resp.rows("boards") {
            let board = node
                .get("board_id")
                .and_then(|v| v.as_str())
                .or_else(|| node.get(DATA_PROP).and_then(|d| d.get("board_id")).and_then(|v| v.as_str()));
            if let Some(b) = board {
                out.push(b.to_string());
            }
        }
        Ok(out)
    }

    fn boards_by_owner(&self, owner: &str) -> anyhow::Result<Vec<String>> {
        let mut out = Vec::new();
        for b in self.all_board_ids()? {
            let tenant = self.tenant(&b);
            let conds = vec![FilterCond {
                field: "$.owner_key".to_string(),
                op: Op::Eq,
                value: Json::String(owner.to_string()),
            }];
            let cur = self.fetch_rows(tenant.as_str(), &engine_label("wb_apps"), &conds, 1, 0, &[])?;
            if !cur.rows.is_empty() {
                out.push(b);
            }
        }
        Ok(out)
    }

    /// Ensure the BM25 text index exists for `label._search` (tenant-scoped).
    /// Index creation is async: `create_index` returns an `operation_id`, and
    /// the build completes asynchronously. We fire-and-forget: subsequent
    /// searches hit `index_not_found` until it's ready, and the adapter falls
    /// back to a Rust scan in that case.
    fn ensure_text_index(&self, _tenant: &str, label: &str) -> anyhow::Result<()> {
        let spec = json!({
            "node_text": { "label": label, "property": SEARCH_PROP, "tenant_property": helixdb::tenant::TENANT_PROP }
        });
        let root = json!({ "create_index": { "spec": spec, "if_not_exists": true } });
        let batch = request::write_batch(vec![request::entry("idx", root)], vec!["idx"]);
        let _ = self.client.post("write", "ensure_text_index", batch)?;
        Ok(())
    }

    /// BM25 full-text search over a tenant's record nodes. Returns rows ranked
    /// by relevance (score desc). Requires the text index to exist.
    fn search_rows(
        &self,
        tenant: &str,
        label: &str,
        query: &str,
        limit: usize,
        offset: usize,
        extra: &[FilterCond],
    ) -> anyhow::Result<Cursor> {
        let k = (limit + offset).max(1);
        let src = json!({
            "text_search_nodes": {
                "label": label,
                "property": SEARCH_PROP,
                "tenant_value": { "value": { "string": tenant } },
                "query_text": { "value": { "string": query } },
                "k": { "literal": k }
            }
        });
        // Project the full node so `data`/`_srv_key` round-trip (BM25 hits only
        // carry `$id` + `$score` by default).
        let root = request::value_map(src, vec!["$id", DATA_PROP, helixdb::row::KEY_PROP, "$score"]);
        let batch = request::read_batch(vec![request::entry("hits", root)], vec!["hits"]);
        let resp = self.client.post("read", "search_rows", batch)?;
        let mut rows = Vec::new();
        for node in resp.rows("hits") {
            let mut row = node_to_row(&node, label)?;
            if extra.iter().any(|c| !rest_matches(c, &row)) {
                continue;
            }
            if label == engine_label("wb_records") {
                let seq = row.data.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
                row.key = Key::int(seq);
            }
            rows.push(row);
        }
        let start = offset.min(rows.len());
        let end = (start + limit).min(rows.len());
        Ok(Cursor { rows: rows[start..end].to_vec(), has_more: end < rows.len() })
    }

    /// Push an aggregate down to Helix `aggregate_by`. Returns `Ok(None)` when
    /// it can't (e.g. filter conditions that can't be expressed as node
    /// predicates), so the caller falls back to Rust.
    fn aggregate_pushdown(
        &self,
        board: &str,
        table_name: &str,
        filter: &SrvFilter,
        agg: engine::storage::ir::Agg,
        field: Option<&str>,
        group_by: Option<&str>,
    ) -> anyhow::Result<Option<Vec<Json>>> {
        let tenant = self.tenant(board);
        let label = engine_label("wb_records");

        // Build the source predicate: label + tenant + table + any filter conds
        // that map to node props (routing or mirrors). Non-pushable conds abort
        // the pushdown.
        let mut pred = helixdb::predicate::Predicate::new()
            .eq("$label", json!(label))
            .eq(helixdb::tenant::TENANT_PROP, json!(tenant.as_str()))
            .eq(TABLE_PROP, json!(table_name));
        for c in &filter.conds {
            let prop = match c.field.as_str() {
                "$.board_id" => Some(BOARD_PROP.to_string()),
                "$.table" => Some(TABLE_PROP.to_string()),
                "$.seq" => Some(SEQ_PROP.to_string()),
                f => mirror_prop_for(f),
            };
            let Some(prop) = prop else {
                return Ok(None); // payload path filter not mirrored -> Rust
            };
            match helixdb::predicate::from_cond(&prop, c.op.as_str(), &c.value) {
                Some(p) => pred = pred.and_raw(p),
                None => return Ok(None), // contains/search/etc -> Rust
            }
        }
        let src = json!({ "nodes_where": { "predicate": pred.into_json() } });

        // The function tuple: [fn, property]. `count` counts nodes; the others
        // aggregate a numeric prop. `mean` is Helix's avg.
        let fn_name = match agg {
            engine::storage::ir::Agg::Count => "count",
            engine::storage::ir::Agg::Sum => "sum",
            engine::storage::ir::Agg::Avg => "mean",
            engine::storage::ir::Agg::Min => "min",
            engine::storage::ir::Agg::Max => "max",
        };
        let prop = match field {
            Some(f) => mirror_prop_for(f),
            None => None,
        };
        let function = json!({ fn_name: null });
        let mut body = serde_json::Map::new();
        body.insert("input".into(), src);
        body.insert("function".into(), function);
        if let Some(p) = prop {
            body.insert("property".into(), json!(p));
        } else if agg != engine::storage::ir::Agg::Count {
            return Ok(None); // non-count needs a mirrored field
        }
        if let Some(g) = group_by {
            if let Some(gp) = mirror_prop_for(g) {
                body.insert("group".into(), json!(gp));
            } else {
                return Ok(None);
            }
        }
        let root = json!({ "aggregate_by": body });
        let batch = request::read_batch(vec![request::entry("agg", root)], vec!["agg"]);
        let resp = self.client.post("read", "aggregate", batch)?;
        let rows = resp.rows("agg");
        let mut out = Vec::new();
        for row in rows {
            let obj = row.as_object().cloned().unwrap_or_default();
            // Response: [{ "<prop>_<Agg>": value }] or [{ "<prop>_<Agg>": value,
            // "<group>": g }] when grouped. Normalize to engine shape.
            let mut value = Json::Null;
            let mut group = Json::Null;
            for (k, v) in obj {
                if k.ends_with(&format!("_{}", cap_agg(fn_name))) {
                    value = v;
                } else if k != "input" {
                    group = Json::String(k.clone());
                }
            }
            if group_by.is_some() {
                out.push(json!({ "group": group, "value": value }));
            } else {
                out.push(json!({ "value": value }));
            }
        }
        Ok(Some(out))
    }
}

fn cap_agg(fn_name: &str) -> &'static str {
    match fn_name {
        "count" => "Count",
        "sum" => "Sum",
        "mean" => "Avg",
        "min" => "Min",
        "max" => "Max",
        _ => "Count",
    }
}
