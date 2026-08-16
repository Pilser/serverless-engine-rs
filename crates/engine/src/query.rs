use crate::model::Record;
use crate::storage::database::{Database, Query, Row, TtlClause};
use crate::storage::ir::{Agg, FilterCond, Op, SrvFilter};
use crate::tables::TABLE_RECORDS;
use serde_json::Value as Json;

fn board_ttl_clause(board: &crate::model::Board) -> Option<TtlClause> {
    Some(TtlClause {
        seconds: board.ttl_seconds,
        field: board.ttl_field.clone(),
    })
}

fn to_record(row: Row) -> anyhow::Result<Record> {
    serde_json::from_value(row.data).map_err(Into::into)
}

pub fn query_records(
    db: &dyn Database,
    board_id: &str,
    filter: &SrvFilter,
    orders: &[(String, bool)],
    limit: usize,
    offset: usize,
) -> anyhow::Result<Vec<Record>> {
    let board = crate::crud::load_board(db, board_id)?;
    let limit = limit.clamp(1, 500);
    let mut conds = vec![crate::crud::board_cond(board_id)];
    conds.extend(filter.conds.clone());
    let q = Query {
        filter: SrvFilter { conds },
        orders: orders.to_vec(),
        limit,
        offset,
        ttl: board_ttl_clause(&board),
    };
    let mut recs = Vec::new();
    for row in db.query(TABLE_RECORDS, &q)?.rows {
        let mut rec = to_record(row)?;
        rec.payload = crate::schema::apply_redact(&board, &rec.payload);
        recs.push(rec);
    }
    Ok(recs)
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

fn highlight(text: &str, query: &str) -> String {
    let mut out = text.to_string();
    for token in query.split_whitespace() {
        if !token.is_empty() {
            out = ci_replace_once(&out, token);
        }
    }
    out
}

pub fn search_records(
    db: &dyn Database,
    board_id: &str,
    query: &str,
    conds: &SrvFilter,
    limit: usize,
    offset: usize,
    snippet: bool,
) -> anyhow::Result<Vec<Record>> {
    let board = crate::crud::load_board(db, board_id)?;
    let limit = limit.clamp(1, 500);
    let mut all = vec![crate::crud::board_cond(board_id)];
    all.push(FilterCond {
        field: "_".to_string(),
        op: Op::Search,
        value: Json::String(query.to_string()),
    });
    all.extend(conds.conds.clone());
    let q = Query {
        filter: SrvFilter { conds: all },
        orders: vec![],
        limit,
        offset,
        ttl: board_ttl_clause(&board),
    };
    let mut recs = Vec::new();
    for row in db.query(TABLE_RECORDS, &q)?.rows {
        let mut rec = to_record(row)?;
        rec.payload = crate::schema::apply_redact(&board, &rec.payload);
        if snippet && !query.trim().is_empty() {
            let text = serde_json::to_string(&rec.payload).unwrap_or_default();
            rec.snippet = Some(highlight(&text, query));
        }
        recs.push(rec);
    }
    Ok(recs)
}

pub fn aggregate_records(
    db: &dyn Database,
    board_id: &str,
    conds: &SrvFilter,
    agg: Agg,
    field: Option<&str>,
    group_by: Option<&str>,
) -> anyhow::Result<Vec<Json>> {
    let board = crate::crud::load_board(db, board_id)?;
    if agg != Agg::Count && field.is_none() {
        anyhow::bail!("field is required for {agg:?}");
    }
    db.aggregate(
        TABLE_RECORDS,
        board_id,
        conds,
        agg,
        field,
        group_by,
        board_ttl_clause(&board),
    )
}