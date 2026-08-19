use crate::DbPool;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct WbPage {
    pub page_no: i64,
    pub title: String,
    pub body_md: String,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WbMeta {
    pub active_page: Option<i64>,
    pub locked: bool,
    pub rev: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSnap {
    pub id: String,
    pub session_id: String,
    pub page_no: i64,
    pub body_md: String,
    pub pins: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

async fn read_meta(rows: &mut turso::Rows) -> anyhow::Result<Option<WbMeta>> {
    if let Some(row) = rows.next().await? {
        let active_page = match row.get_value(0)? {
            Value::Integer(i) => Some(i),
            _ => None,
        };
        let locked = match row.get_value(1)? {
            Value::Integer(i) => i != 0,
            _ => false,
        };
        let rev = match row.get_value(2)? {
            Value::Integer(i) => i,
            _ => 0,
        };
        Ok(Some(WbMeta { active_page, locked, rev }))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub async fn wb_ensure_session(&self, sid: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let s = sq(sid);
        let sql = format!(
            "BEGIN; \
             INSERT OR IGNORE INTO whiteboard_meta (session_id) VALUES ('{s}'); \
             INSERT INTO whiteboard_pages (session_id, page_no, title) \
               SELECT '{s}', 1, '# Whiteboard' \
               WHERE NOT EXISTS (SELECT 1 FROM whiteboard_pages WHERE session_id = '{s}'); \
             COMMIT;"
        );
        conn.execute_batch(&sql).await?;
        Ok(())
    }

    pub async fn wb_state(&self, sid: &str) -> anyhow::Result<(Vec<WbPage>, WbMeta)> {
        let conn = self.conn().await;
        let mut pages = Vec::new();
        let mut rows = conn
            .query(
                "SELECT page_no, title, body_md, public FROM whiteboard_pages WHERE session_id = ?1 ORDER BY page_no",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let page_no = match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => return Err(anyhow::anyhow!("invalid page_no")),
            };
            let title = match row.get_value(1)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid title")),
            };
            let body_md = match row.get_value(2)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid body_md")),
            };
            let public = match row.get_value(3)? {
                Value::Integer(i) => i != 0,
                _ => false,
            };
            pages.push(WbPage { page_no, title, body_md, public });
        }
        let mut meta_rows = conn
            .query(
                "SELECT active_page, locked, rev FROM whiteboard_meta WHERE session_id = ?1",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        let meta = read_meta(&mut meta_rows).await?.unwrap_or(WbMeta {
            active_page: None,
            locked: false,
            rev: 0,
        });
        Ok((pages, meta))
    }

    pub async fn wb_write_page(&self, sid: &str, page_no: i64, title: &str, body_md: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let sql = format!(
            "BEGIN; \
             INSERT INTO whiteboard_pages (session_id, page_no, title, body_md) VALUES ('{}', {}, '{}', '{}') \
             ON CONFLICT(session_id, page_no) DO UPDATE SET \
               title = excluded.title, body_md = excluded.body_md, updated_at = datetime('now'); \
             UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{}'; \
             COMMIT;",
            sq(sid), page_no, sq(title), sq(body_md), sq(sid)
        );
        conn.execute_batch(&sql).await?;
        Ok(())
    }

    pub async fn wb_add_page(&self, sid: &str, title: &str) -> anyhow::Result<i64> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT COALESCE(MAX(page_no), 0) + 1 FROM whiteboard_pages WHERE session_id = ?1",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        let next = match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => return Err(anyhow::anyhow!("invalid next page_no")),
            },
            None => 1,
        };
        let s = sq(sid);
        let sql = format!(
            "BEGIN; \
             INSERT INTO whiteboard_pages (session_id, page_no, title) VALUES ('{s}', {next}, '{}'); \
             UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{s}'; \
             COMMIT;",
            sq(title)
        );
        conn.execute_batch(&sql).await?;
        Ok(next)
    }

    pub async fn wb_delete_page(&self, sid: &str, page_no: i64) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut meta_rows = conn
            .query(
                "SELECT active_page, locked, rev FROM whiteboard_meta WHERE session_id = ?1",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        if let Some(meta) = read_meta(&mut meta_rows).await? {
            if meta.locked && meta.active_page == Some(page_no) {
                anyhow::bail!(
                    "page {page_no} is the locked active page; close the whiteboard before deleting it"
                );
            }
        }
        let s = sq(sid);
        let sql = format!(
            "BEGIN; \
             DELETE FROM whiteboard_pages WHERE session_id = '{s}' AND page_no = {page_no}; \
             UPDATE whiteboard_pages SET page_no = page_no - 1, updated_at = datetime('now') \
               WHERE session_id = '{s}' AND page_no > {page_no}; \
             DELETE FROM whiteboard_public_pages WHERE session_id = '{s}' AND page_no = {page_no}; \
             UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{s}'; \
             COMMIT;"
        );
        conn.execute_batch(&sql).await?;
        Ok(())
    }

    pub async fn wb_move_page(&self, sid: &str, from: i64, to: i64) -> anyhow::Result<()> {
        if from == to {
            return Ok(());
        }
        let conn = self.conn().await;
        let mut meta_rows = conn
            .query(
                "SELECT active_page, locked, rev FROM whiteboard_meta WHERE session_id = ?1",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        if let Some(meta) = read_meta(&mut meta_rows).await? {
            if meta.locked && (meta.active_page == Some(from) || meta.active_page == Some(to)) {
                anyhow::bail!(
                    "cannot move a locked page; close the whiteboard before reordering pages"
                );
            }
        }
        let mut page_rows = conn
            .query(
                "SELECT page_no FROM whiteboard_pages WHERE session_id = ?1 ORDER BY page_no",
                Params::Positional(vec![Value::from(sid)]),
            )
            .await?;
        let mut order = Vec::new();
        while let Some(row) = page_rows.next().await? {
            match row.get_value(0)? {
                Value::Integer(i) => order.push(i),
                _ => return Err(anyhow::anyhow!("invalid page_no")),
            }
        }
        let fi = order
            .iter()
            .position(|&p| p == from)
            .ok_or_else(|| anyhow::anyhow!("page {from} not found"))?;
        order.remove(fi);
        let ti = if to < 1 {
            0
        } else if (to as usize) > order.len() {
            order.len()
        } else {
            (to - 1) as usize
        };
        order.insert(ti, from);
        const OFFSET: i64 = 1_000_000;
        let s = sq(sid);
        let mut stmts = format!(
            "BEGIN; \
             UPDATE whiteboard_pages SET page_no = page_no + {OFFSET} WHERE session_id = '{s}';"
        );
        for (idx, old_no) in order.iter().enumerate() {
            let new_no = idx as i64 + 1;
            stmts.push_str(&format!(
                " UPDATE whiteboard_pages SET page_no = {new_no}, updated_at = datetime('now') \
                 WHERE session_id = '{s}' AND page_no = {};",
                old_no + OFFSET
            ));
        }
        stmts.push_str(&format!(
            " UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{s}'; \
             COMMIT;"
        ));
        conn.execute_batch(&stmts).await?;
        Ok(())
    }

    pub async fn wb_open(&self, sid: &str, page_no: i64) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE whiteboard_meta SET active_page = ?1, locked = 1, rev = rev + 1, updated_at = datetime('now') WHERE session_id = ?2",
            Params::Positional(vec![Value::Integer(page_no), Value::from(sid)]),
        ).await?;
        Ok(())
    }

    pub async fn wb_close(&self, sid: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE whiteboard_meta SET locked = 0, rev = rev + 1, updated_at = datetime('now') WHERE session_id = ?1",
            Params::Positional(vec![Value::from(sid)]),
        ).await?;
        Ok(())
    }

    pub async fn wb_set_public(&self, sid: &str, page_no: i64, public: bool) -> anyhow::Result<Option<String>> {
        let conn = self.conn().await;
        if public {
            let mut rows = conn
                .query(
                    "SELECT id FROM whiteboard_public_pages WHERE session_id = ?1 AND page_no = ?2",
                    Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
                )
                .await?;
            if let Some(row) = rows.next().await? {
                if let Value::Text(slug) = row.get_value(0)? {
                    return Ok(Some(slug));
                }
            }
            let body = {
                let mut rows = conn
                    .query(
                        "SELECT body_md FROM whiteboard_pages WHERE session_id = ?1 AND page_no = ?2",
                        Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
                    )
                    .await?;
                match rows.next().await? {
                    Some(row) => match row.get_value(0)? {
                        Value::Text(t) => t,
                        _ => String::new(),
                    },
                    None => String::new(),
                }
            };
            let slug = gen_slug();
            let s = sq(sid);
            let sql = format!(
                "BEGIN; \
                 INSERT INTO whiteboard_public_pages (id, session_id, page_no, body_md, pins) \
                   VALUES ('{}', '{s}', {page_no}, '{}', ''); \
                 UPDATE whiteboard_pages SET public = 1, updated_at = datetime('now') WHERE session_id = '{s}' AND page_no = {page_no}; \
                 UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{s}'; \
                 COMMIT;",
                sq(&slug), sq(&body)
            );
            conn.execute_batch(&sql).await?;
            Ok(Some(slug))
        } else {
            let s = sq(sid);
            let sql = format!(
                "BEGIN; \
                 UPDATE whiteboard_pages SET public = 0, updated_at = datetime('now') WHERE session_id = '{s}' AND page_no = {page_no}; \
                 DELETE FROM whiteboard_public_pages WHERE session_id = '{s}' AND page_no = {page_no}; \
                 UPDATE whiteboard_meta SET rev = rev + 1, updated_at = datetime('now') WHERE session_id = '{s}'; \
                 COMMIT;"
            );
            conn.execute_batch(&sql).await?;
            Ok(None)
        }
    }

    pub async fn wb_set_public_content(&self, slug: &str, body_md: &str, pins: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE whiteboard_public_pages SET body_md = ?1, pins = ?2 WHERE id = ?3",
            Params::Positional(vec![Value::from(body_md), Value::from(pins), Value::from(slug)]),
        ).await?;
        Ok(())
    }

    pub async fn wb_public_slug(&self, sid: &str, page_no: i64) -> anyhow::Result<Option<String>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT id FROM whiteboard_public_pages WHERE session_id = ?1 AND page_no = ?2",
                Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            match row.get_value(0)? {
                Value::Text(s) => Ok(Some(s)),
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    pub async fn wb_public_snapshot(&self, slug: &str) -> anyhow::Result<Option<PublicSnap>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT id, session_id, page_no, body_md, pins FROM whiteboard_public_pages WHERE id = ?1",
                Params::Positional(vec![Value::from(slug)]),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let id = match row.get_value(0)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid snapshot id")),
            };
            let session_id = match row.get_value(1)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid snapshot session_id")),
            };
            let page_no = match row.get_value(2)? {
                Value::Integer(i) => i,
                _ => return Err(anyhow::anyhow!("invalid snapshot page_no")),
            };
            let body_md = match row.get_value(3)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid snapshot body_md")),
            };
            let pins = match row.get_value(4)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid snapshot pins")),
            };
            Ok(Some(PublicSnap { id, session_id, page_no, body_md, pins }))
        } else {
            Ok(None)
        }
    }

    pub async fn wb_delete_public_by_page(&self, sid: &str, page_no: i64) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "DELETE FROM whiteboard_public_pages WHERE session_id = ?1 AND page_no = ?2",
            Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
        ).await?;
        Ok(())
    }
}

fn gen_slug() -> String {
    let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let len = 6 + (nanos % 3) as usize;
    let mut slug = String::with_capacity(len);
    let mut n = nanos;
    for _ in 0..len {
        slug.push(chars[(n % 36) as usize] as char);
        n /= 36;
    }
    slug
}
