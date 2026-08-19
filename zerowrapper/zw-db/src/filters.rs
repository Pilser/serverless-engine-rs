use crate::DbPool;
use serde_json::Value as JVal;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    NotContains,
    In,
    Search,
}

impl Op {
    pub fn parse(s: &str) -> anyhow::Result<Op> {
        match s {
            "eq" => Ok(Op::Eq),
            "neq" => Ok(Op::Ne),
            "gt" => Ok(Op::Gt),
            "gte" => Ok(Op::Gte),
            "lt" => Ok(Op::Lt),
            "lte" => Ok(Op::Lte),
            "contains" => Ok(Op::Contains),
            "not_contains" => Ok(Op::NotContains),
            "in" => Ok(Op::In),
            "search" => Ok(Op::Search),
            other => anyhow::bail!("unknown filter operator '{other}' (eq, neq, gt, gte, lt, lte, contains, not_contains, in, search)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Agg {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Agg {
    pub fn parse(s: &str) -> anyhow::Result<Agg> {
        match s {
            "count" => Ok(Agg::Count),
            "sum" => Ok(Agg::Sum),
            "avg" => Ok(Agg::Avg),
            "min" => Ok(Agg::Min),
            "max" => Ok(Agg::Max),
            other => anyhow::bail!("unknown aggregate '{other}' (count, sum, avg, min, max)"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterCond {
    pub field: String,
    pub op: Op,
    pub value: JVal,
}

#[derive(Debug, Clone, Default)]
pub struct SrvFilter {
    pub conds: Vec<FilterCond>,
}

pub fn normalize_path(p: &str) -> String {
    if p.starts_with('$') {
        p.to_string()
    } else {
        format!("$.{}", p)
    }
}

#[derive(Debug, Clone)]
pub enum PathSeg {
    Key(String),
    Idx(usize),
}

/// Parse a dot/bracket path like `a.b[2].c` into segments. Returns None on garbage.
pub fn path_items(p: &str) -> Option<Vec<PathSeg>> {
    let bytes = p.as_bytes();
    let mut segs: Vec<PathSeg> = Vec::new();
    let mut i = 0;
    let mut key = String::new();
    let push_key = |key: &mut String, segs: &mut Vec<PathSeg>| {
        if !key.is_empty() {
            segs.push(PathSeg::Key(std::mem::take(key)));
        }
    };
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '.' => {
                push_key(&mut key, &mut segs);
                i += 1;
            }
            '[' => {
                push_key(&mut key, &mut segs);
                let close = p[i + 1..].find(']')? + i + 1;
                let idx: usize = p[i + 1..close].trim().parse().ok()?;
                segs.push(PathSeg::Idx(idx));
                i = close + 1;
                if p[i..].starts_with('.') {
                    i += 1;
                }
            }
            _ => {
                key.push(c);
                i += 1;
            }
        }
    }
    push_key(&mut key, &mut segs);
    if matches!(segs.first(), Some(PathSeg::Key(k)) if k == "$") {
        segs.remove(0);
    }
    if segs.is_empty() {
        return None;
    }
    Some(segs)
}

fn parse_cond_obj(v: &JVal) -> anyhow::Result<Vec<FilterCond>> {
    let obj = v.as_object().ok_or_else(|| anyhow::anyhow!("a condition must be an object"))?;
    let field = normalize_path(
        obj.get("field")
            .and_then(|f| f.as_str())
            .ok_or_else(|| anyhow::anyhow!("a condition needs \"field\" (json path)"))?,
    );
    let mut out = Vec::new();
    if let Some(opname) = obj.get("op").and_then(|o| o.as_str()) {
        let value = obj
            .get("value")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("an operator on field '{field}' needs a \"value\""))?;
        let op = Op::parse(opname)?;
        if op == Op::In && !value.is_array() {
            anyhow::bail!("operator \"in\" on field '{field}' needs an array value");
        }
        if op == Op::Search && !value.is_string() {
            anyhow::bail!("operator \"search\" needs a string query");
        }
        out.push(FilterCond { field, op, value });
        return Ok(out);
    }
    for (k, value) in obj {
        if k == "field" {
            continue;
        }
        let op = Op::parse(k)?;
        if op == Op::In && !value.is_array() {
            anyhow::bail!("operator \"in\" on field '{field}' needs an array value");
        }
        if op == Op::Search && !value.is_string() {
            anyhow::bail!("operator \"search\" needs a string query");
        }
        out.push(FilterCond { field: field.clone(), op, value: value.clone() });
    }
    if out.is_empty() {
        anyhow::bail!("condition for '{field}' has no operators");
    }
    Ok(out)
}

fn push_ops(conds: &mut Vec<FilterCond>, field: &str, ops: &JVal) -> anyhow::Result<()> {
    let field = normalize_path(field);
    match ops {
        JVal::Object(map) => {
            for (opname, value) in map {
                let op = Op::parse(opname)?;
                if op == Op::In && !value.is_array() {
                    anyhow::bail!("operator \"in\" on field '{field}' needs an array value");
                }
                if op == Op::Search && !value.is_string() {
                    anyhow::bail!("operator \"search\" needs a string query");
                }
                conds.push(FilterCond { field: field.clone(), op, value: value.clone() });
            }
        }
        other => {
            conds.push(FilterCond { field, op: Op::Eq, value: other.clone() });
        }
    }
    Ok(())
}

pub fn parse_filter(v: &JVal) -> anyhow::Result<SrvFilter> {
    let mut conds = Vec::new();
    match v {
        JVal::Array(arr) => {
            for el in arr {
                conds.extend(parse_cond_obj(el)?);
            }
        }
        JVal::Object(obj) if obj.contains_key("field") => {
            conds.extend(parse_cond_obj(v)?);
        }
        JVal::Object(obj) if obj.contains_key("search") => {
            let q = obj
                .get("search")
                .and_then(|s| s.as_str())
                .ok_or_else(|| anyhow::anyhow!("\"search\" needs a string query"))?;
            conds.push(FilterCond { field: "_".to_string(), op: Op::Search, value: JVal::String(q.to_string()) });
            for (field, ops) in obj {
                if field == "search" {
                    continue;
                }
                push_ops(&mut conds, field, ops)?;
            }
        }
        JVal::Object(obj) => {
            for (field, ops) in obj {
                if field == "$and" {
                    let arr = ops
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("\"$and\" needs an array of condition objects"))?;
                    for el in arr {
                        conds.extend(parse_filter(el)?.conds);
                    }
                    continue;
                }
                if field == "$or" {
                    anyhow::bail!(
                        "operator \"$or\" is not supported yet (use multiple top-level keys for implicit AND, or request $or support)"
                    );
                }
                push_ops(&mut conds, field, ops)?;
            }
        }
        _ => anyhow::bail!("filter must be a condition object, a {{field: op}} map, an array of conditions, or {{\"$and\":[...]}}"),
    }
    Ok(SrvFilter { conds })
}

fn scalar_text(v: &JVal) -> String {
    match v {
        JVal::String(s) => s.clone(),
        JVal::Number(n) => n.to_string(),
        JVal::Bool(b) => b.to_string(),
        JVal::Null => String::new(),
        _ => v.to_string(),
    }
}

fn ci_replace_once(src: &str, needle: &str) -> String {
    let low_needle = needle.to_lowercase();
    let low_src = src.to_lowercase();
    let mut out = String::new();
    let mut from = 0;
    while let Some(rel) = low_src.get(from..).and_then(|s| s.find(&low_needle)) {
        let start = from + rel;
        let end = start + needle.len();
        out.push_str(&src[from..start]);
        out.push_str("<mark>");
        out.push_str(&src[start..end]);
        out.push_str("</mark>");
        from = end;
    }
    out.push_str(&src[from..]);
    out
}

/// Case-insensitive substring highlighting (matches the ngram FTS semantics: a
/// partial token is as good as a whole word). Wraps hits in <mark>.
pub fn highlight_substring(text: &str, query: &str) -> String {
    let mut out = text.to_string();
    for token in query.split_whitespace() {
        if !token.is_empty() {
            out = ci_replace_once(&out, token);
        }
    }
    out
}

fn like_fragment(raw: &JVal) -> String {
    let mut out = String::new();
    for ch in scalar_text(raw).chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            out.push('\\');
        }
        out.push(ch);
    }
    format!("%{}%", out)
}

fn jval_from_value(v: Value) -> JVal {
    match v {
        Value::Null => JVal::Null,
        Value::Integer(i) => JVal::from(i),
        Value::Real(f) => JVal::from(f),
        Value::Text(s) => JVal::String(s),
        Value::Blob(b) => JVal::String(String::from_utf8_lossy(&b).to_string()),
    }
}

pub fn srv_filter_where(board_id: &str, conds: &[FilterCond]) -> (String, Vec<Value>, Option<usize>) {
    let mut params = vec![Value::from(board_id)];
    let mut parts = vec!["board_id = ?1".to_string()];
    let mut search_ph: Option<usize> = None;
    for c in conds {
        if c.op == Op::Search {
            let qph = params.len() + 1;
            params.push(Value::from(scalar_text(&c.value)));
            parts.push(format!("fts_match(payload, ?{qph})"));
            search_ph = Some(qph);
            continue;
        }
        if c.field == "$.writer" && c.op == Op::Eq {
            let vp = params.len() + 1;
            params.push(Value::from(scalar_text(&c.value)));
            parts.push(format!("writer = ?{vp}"));
            continue;
        }
        let path_ph = params.len() + 1;
        params.push(Value::from(c.field.clone()));
        let cast_text = format!("CAST(json_extract(payload, ?{path_ph}) AS TEXT)");
        let cast_num = format!("CAST(json_extract(payload, ?{path_ph}) AS REAL)");
        let target = if c.value.is_number() { cast_num.clone() } else { cast_text.clone() };
        let vp = params.len() + 1;
        let frag = match c.op {
            Op::Search => unreachable!("search handled above"),
            Op::Eq => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{cast_text} = ?{vp}")
            }
            Op::Ne => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{cast_text} <> ?{vp}")
            }
            Op::Gt => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} > ?{vp}")
            }
            Op::Gte => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} >= ?{vp}")
            }
            Op::Lt => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} < ?{vp}")
            }
            Op::Lte => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} <= ?{vp}")
            }
            Op::Contains => {
                params.push(Value::from(like_fragment(&c.value)));
                format!("LOWER({cast_text}) LIKE LOWER(?{vp}) ESCAPE '\\'")
            }
            Op::NotContains => {
                params.push(Value::from(like_fragment(&c.value)));
                format!("LOWER({cast_text}) NOT LIKE LOWER(?{vp}) ESCAPE '\\'")
            }
            Op::In => {
                let arr = c.value.as_array().expect("validated in-value is an array");
                let mut phs = Vec::new();
                for el in arr {
                    let p = params.len() + 1;
                    params.push(Value::from(scalar_text(el)));
                    phs.push(format!("?{p}"));
                }
                format!("{cast_text} IN ({})", phs.join(", "))
            }
        };
        parts.push(frag);
    }
    (format!("{}", parts.join(" AND ")), params, search_ph)
}

fn agg_target(agg: Agg, field: Option<&str>, params: &mut Vec<Value>) -> anyhow::Result<String> {
    if agg == Agg::Count {
        return Ok("COUNT(*)".to_string());
    }
    let f = field.ok_or_else(|| anyhow::anyhow!("--field <json path> is required for {agg:?}"))?;
    let ph = params.len() + 1;
    params.push(Value::from(f));
    let name = match agg {
        Agg::Sum => "SUM",
        Agg::Avg => "AVG",
        Agg::Min => "MIN",
        Agg::Max => "MAX",
        Agg::Count => unreachable!(),
    };
    Ok(format!("{name}(CAST(json_extract(payload, ?{ph}) AS REAL))"))
}

impl DbPool {
    pub async fn srv_record_filter(
        &self,
        board_id: &str,
        conds: &[FilterCond],
        orders: &[(&str, bool)],
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<crate::serverless::SrvRecord>> {
        let (where_sql, mut params, search_ph) = srv_filter_where(board_id, conds);
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND {where_sql}"),
            None => where_sql,
        };
        let mut sql = format!("SELECT seq, payload, created_at, writer FROM wb_records WHERE {where_sql}");
        if !orders.is_empty() {
            let mut terms = Vec::new();
            for (field, desc) in orders {
                let op = params.len() + 1;
                params.push(Value::from(*field));
                let dir = if *desc { "DESC" } else { "ASC" };
                terms.push(format!("json_extract(payload, ?{op}) {dir}"));
            }
            terms.push("seq DESC".to_string());
            sql.push_str(&format!(" ORDER BY {}", terms.join(", ")));
        } else {
            match search_ph {
                Some(q) => sql.push_str(&format!(" ORDER BY fts_score(payload, ?{q}) DESC")),
                None => sql.push_str(" ORDER BY seq DESC"),
            }
        }
        let limit = limit.clamp(1, 500);
        let lp = params.len() + 1;
        params.push(Value::from(limit));
        sql.push_str(&format!(" LIMIT ?{lp}"));
        if offset > 0 {
            let op = params.len() + 1;
            params.push(Value::from(offset));
            sql.push_str(&format!(" OFFSET ?{op}"));
        }
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(record) = crate::serverless::read_record(&mut rows).await? {
            out.push(record);
        }
        drop(rows);
        drop(conn);
        self.srv_record_redact(board_id, &mut out).await?;
        Ok(out)
    }

    pub async fn srv_record_search(
        &self,
        board_id: &str,
        query: &str,
        conds: &[FilterCond],
        limit: i64,
        offset: i64,
        snippet: bool,
    ) -> anyhow::Result<Vec<crate::serverless::SrvRecord>> {
        let mut all = conds.to_vec();
        all.push(FilterCond { field: "_".to_string(), op: Op::Search, value: JVal::String(query.to_string()) });
        let (where_sql, mut params, search_ph) = srv_filter_where(board_id, &all);
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND {where_sql}"),
            None => where_sql,
        };
        let q = search_ph.ok_or_else(|| anyhow::anyhow!("search query could not be bound"))?;
        let limit = limit.clamp(1, 500);
        let lp = params.len() + 1;
        params.push(Value::from(limit));
        let mut sql = format!(
            "SELECT seq, payload, created_at, writer, fts_score(payload, ?{q}) AS score, NULL AS snippet FROM wb_records \
             WHERE {where_sql} ORDER BY fts_score(payload, ?{q}) DESC LIMIT ?{lp}"
        );
        if offset > 0 {
            let op = params.len() + 1;
            params.push(Value::from(offset));
            sql.push_str(&format!(" OFFSET ?{op}"));
        }
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(record) = crate::serverless::read_record_scored(&mut rows).await? {
            out.push(record);
        }
        drop(rows);
        drop(conn);
        self.srv_record_redact(board_id, &mut out).await?;
        if snippet && !query.is_empty() {
            for record in &mut out {
                let text = serde_json::to_string(&record.payload).unwrap_or_default();
                record.snippet = Some(highlight_substring(&text, query));
            }
        }
        Ok(out)
    }

    pub async fn srv_record_aggregate(
        &self,
        board_id: &str,
        conds: &[FilterCond],
        agg: Agg,
        field: Option<&str>,
        group_by: Option<&str>,
    ) -> anyhow::Result<Vec<JVal>> {
        let (where_sql, mut params, _search_ph) = srv_filter_where(board_id, conds);
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND {where_sql}"),
            None => where_sql,
        };
        let target = agg_target(agg, field, &mut params)?;
        let sql = match group_by {
            Some(g) => {
                let gp1 = params.len() + 1;
                params.push(Value::from(g));
                let gp2 = params.len() + 1;
                params.push(Value::from(g));
                format!(
                    "SELECT json_extract(payload, ?{gp1}) AS grp, {target} AS value \
                     FROM wb_records WHERE {where_sql} GROUP BY json_extract(payload, ?{gp2}) ORDER BY 1"
                )
            }
            None => format!(
                "SELECT {target} AS value FROM wb_records WHERE {where_sql}"
            ),
        };
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            if group_by.is_some() {
                let grp = match row.get_value(0)? {
                    Value::Null => JVal::Null,
                    v => jval_from_value(v),
                };
                let value = match row.get_value(1)? {
                    Value::Null => JVal::Null,
                    v => jval_from_value(v),
                };
                out.push(serde_json::json!({ "group": grp, "value": value }));
            } else {
                let value = match row.get_value(0)? {
                    Value::Null => JVal::Null,
                    v => jval_from_value(v),
                };
                out.push(serde_json::json!({ "value": value }));
            }
        }
        Ok(out)
    }
}
