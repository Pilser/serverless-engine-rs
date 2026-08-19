use std::collections::HashMap;

use crate::filters::{normalize_path, path_items, PathSeg};
use crate::serverless::SrvRecord;
use crate::DbPool;
use serde::Serialize;
use serde_json::Value as JVal;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct SrvLink {
    pub board_id: String,
    pub from_key: String,
    pub parent_board: String,
    pub parent_key: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn scalar_str(v: &JVal) -> String {
    match v {
        JVal::String(s) => s.clone(),
        JVal::Number(n) => n.to_string(),
        JVal::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn json_str(payload: &JVal, path: &str) -> Option<String> {
    let segs = path_items(path)?;
    let mut cur = payload;
    for seg in &segs {
        match seg {
            PathSeg::Key(k) => cur = cur.get(k.as_str())?,
            PathSeg::Idx(i) => cur = cur.get(*i)?,
        }
    }
    if cur.is_null() {
        return None;
    }
    Some(scalar_str(cur))
}

impl DbPool {
    async fn links_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Text(s) => {
                    if s != owner {
                        anyhow::bail!("board {board_id} is not owned by {owner}");
                    }
                }
                _ => return Err(anyhow::anyhow!("invalid owner_key")),
            },
            None => anyhow::bail!("board {board_id} not found"),
        }
        Ok(())
    }

    pub async fn srv_link_set(
        &self,
        board_id: &str,
        owner: &str,
        from_key: &str,
        parent_board: &str,
        parent_key: &str,
    ) -> anyhow::Result<()> {
        self.links_check_owned(board_id, owner).await?;
        let from = normalize_path(from_key);
        let via = normalize_path(parent_key);
        let conn = self.conn().await;
        let mut exists = conn
            .query(
                "SELECT 1 FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(parent_board)]),
            )
            .await?;
        match exists.next().await? {
            Some(_) => {}
            None => anyhow::bail!("parent board {parent_board} not found"),
        }
        drop(exists);
        let b = sq(board_id);
        let f = sq(&from);
        let p = sq(parent_board);
        let v = sq(&via);
        let sql = format!(
            "INSERT INTO wb_links (board_id, from_key, parent_board, parent_key) \
             VALUES ('{b}', '{f}', '{p}', '{v}') \
             ON CONFLICT(board_id) DO UPDATE SET from_key = '{f}', parent_board = '{p}', parent_key = '{v}'"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_link_clear(&self, board_id: &str, owner: &str) -> anyhow::Result<bool> {
        self.links_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let affected = conn
            .execute(
                "DELETE FROM wb_links WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        if affected == 0 {
            anyhow::bail!("no link configured for board {board_id}");
        }
        Ok(true)
    }

    pub async fn srv_link_get(&self, board_id: &str) -> anyhow::Result<Option<SrvLink>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT board_id, from_key, parent_board, parent_key FROM wb_links WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let board_id = match row.get_value(0)? {
                    Value::Text(s) => s,
                    _ => return Ok(None),
                };
                let from_key = match row.get_value(1)? {
                    Value::Text(s) => s,
                    _ => return Ok(None),
                };
                let parent_board = match row.get_value(2)? {
                    Value::Text(s) => s,
                    _ => return Ok(None),
                };
                let parent_key = match row.get_value(3)? {
                    Value::Text(s) => s,
                    _ => return Ok(None),
                };
                Ok(Some(SrvLink { board_id, from_key, parent_board, parent_key }))
            }
            None => Ok(None),
        }
    }

    pub async fn srv_join_list(
        &self,
        child_board: &str,
        conds: &[crate::filters::FilterCond],
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<(SrvRecord, Option<JVal>)>> {
        let link = self.srv_link_get(child_board).await?;
        let records = self.srv_record_filter(child_board, conds, &[], limit, offset).await?;
        let Some(link) = link else {
            return Ok(records.into_iter().map(|r| (r, None)).collect());
        };
        let mut from_values: Vec<String> = Vec::new();
        for rec in &records {
            if let Some(v) = json_str(&rec.payload, &link.from_key) {
                if !from_values.contains(&v) {
                    from_values.push(v);
                }
            }
        }
        let mut parents: HashMap<String, JVal> = HashMap::new();
        if !from_values.is_empty() {
            let conn = self.conn().await;
            let mut phs = Vec::new();
            let mut params = vec![Value::from(link.parent_board.clone()), Value::from(link.parent_key.clone())];
            for v in &from_values {
                phs.push(format!("?{}", params.len() + 1));
                params.push(Value::from(v.clone()));
            }
            let sql = format!(
                "SELECT payload FROM wb_records \
                 WHERE board_id = ?1 AND CAST(json_extract(payload, ?2) AS TEXT) IN ({})",
                phs.join(", ")
            );
            let mut rows = conn.query(&sql, Params::Positional(params)).await?;
            while let Some(row) = rows.next().await? {
                if let Value::Text(t) = row.get_value(0)? {
                    if let Ok(p) = serde_json::from_str::<JVal>(&t) {
                        let key = json_str(&p, &link.parent_key);
                        if let Some(k) = key {
                            parents.insert(k, p);
                        }
                    }
                }
            }
        }
        if !parents.is_empty() {
            if let Some(redact) = self.srv_redact_get(&link.parent_board).await? {
                for p in parents.values_mut() {
                    crate::logic::apply_redact(&redact, p);
                }
            }
        }
        let mut out = Vec::new();
        for rec in records {
            let joined = json_str(&rec.payload, &link.from_key).and_then(|k| parents.get(&k).cloned());
            let mut j = serde_json::Map::new();
            if let Some(p) = joined {
                j.insert(link.from_key.clone(), p);
            } else {
                j.insert(link.from_key.clone(), JVal::Null);
            }
            out.push((rec, Some(JVal::Object(j))));
        }
        Ok(out)
    }
}