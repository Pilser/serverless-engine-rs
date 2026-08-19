use chrono::NaiveDateTime;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

use crate::DbPool;
use crate::cron::next_run;

#[derive(Debug, Clone, Serialize)]
pub struct SrvJob {
    pub id: i64,
    pub board_id: String,
    pub name: String,
    pub schedule: String,
    pub action: String,
    pub next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    pub created_at: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn fmt_iso(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

async fn read_job(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvJob>> {
    if let Some(row) = rows.next().await? {
        let id = match row.get_value(0)? {
            Value::Integer(i) => i,
            _ => return Err(anyhow::anyhow!("invalid id")),
        };
        let board_id = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid board_id")),
        };
        let name = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid name")),
        };
        let schedule = match row.get_value(3)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid schedule")),
        };
        let action = match row.get_value(4)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid action")),
        };
        let next_run_at = match row.get_value(5)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let last_run_at = match row.get_value(6)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let last_status = match row.get_value(7)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let last_message = match row.get_value(8)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let created_at = match row.get_value(9)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        Ok(Some(SrvJob {
            id,
            board_id,
            name,
            schedule,
            action,
            next_run_at,
            last_run_at,
            last_status,
            last_message,
            created_at,
        }))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub async fn srv_job_add(&self, board_id: &str, name: &str, schedule: &str, action: &str) -> anyhow::Result<(i64, String)> {
        let now = fmt_iso(chrono::Utc::now().naive_utc());
        let next = match next_run(schedule, chrono::Utc::now().naive_utc()) {
            Some(dt) => fmt_iso(dt),
            None => anyhow::bail!("invalid schedule '{schedule}'"),
        };
        let conn = self.conn().await;
        let b = sq(board_id);
        let n = sq(name);
        let sch = sq(schedule);
        let a = sq(action);
        let sql = format!(
            "INSERT INTO app_jobs (board_id, name, schedule, action, next_run_at, created_at) \
             VALUES ('{b}', '{n}', '{sch}', '{a}', '{next}', '{now}')"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok((conn.last_insert_rowid(), next))
    }

    pub async fn srv_job_list(&self, board_id: Option<&str>) -> anyhow::Result<Vec<SrvJob>> {
        let conn = self.conn().await;
        let mut rows = match board_id {
            Some(b) => {
                conn.query(
                    "SELECT id, board_id, name, schedule, action, next_run_at, last_run_at, last_status, last_message, created_at \
                     FROM app_jobs WHERE board_id = ?1 ORDER BY id",
                    Params::Positional(vec![Value::from(b)]),
                )
                .await?
            }
            None => {
                conn.query(
                    "SELECT id, board_id, name, schedule, action, next_run_at, last_run_at, last_status, last_message, created_at \
                     FROM app_jobs ORDER BY id",
                    Params::Positional(vec![]),
                )
                .await?
            }
        };
        let mut out = Vec::new();
        while let Some(job) = read_job(&mut rows).await? {
            out.push(job);
        }
        Ok(out)
    }

    pub async fn srv_job_remove(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let affected = conn
            .execute("DELETE FROM app_jobs WHERE id = ?1", Params::Positional(vec![Value::Integer(id)]))
            .await?;
        Ok(affected > 0)
    }

    pub async fn srv_job_due(&self, now_iso: &str, limit: i64) -> anyhow::Result<Vec<SrvJob>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT id, board_id, name, schedule, action, next_run_at, last_run_at, last_status, last_message, created_at \
                 FROM app_jobs \
                 WHERE next_run_at IS NOT NULL AND next_run_at <= ?1 \
                 ORDER BY next_run_at ASC LIMIT ?2",
                Params::Positional(vec![Value::from(now_iso), Value::Integer(limit)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(job) = read_job(&mut rows).await? {
            out.push(job);
        }
        Ok(out)
    }

    pub async fn srv_job_reschedule(&self, id: i64, next_run_at: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let next = match next_run_at {
            Some(s) => format!("'{s}'"),
            None => "NULL".to_string(),
        };
        let sql = format!("UPDATE app_jobs SET next_run_at = {next} WHERE id = {id}");
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_job_mark(&self, id: i64, last_run_at: &str, status: &str, message: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let t = sq(last_run_at);
        let st = sq(status);
        let m = sq(message);
        let sql = format!(
            "UPDATE app_jobs SET last_run_at = '{t}', last_status = '{st}', last_message = '{m}' WHERE id = {id}"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_job_run_insert(
        &self,
        board_id: &str,
        job_id: i64,
        job_name: &str,
        triggered_at: &str,
        duration_ms: i64,
        status: &str,
        message: &str,
        result: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let b = sq(board_id);
        let n = sq(job_name);
        let t = sq(triggered_at);
        let st = sq(status);
        let m = sq(message);
        let r = sq(result);
        let sql = format!(
            "INSERT INTO wb_job_runs (board_id, job_id, job_name, triggered_at, duration_ms, status, message, result) \
             VALUES ('{b}', {job_id}, '{n}', '{t}', {duration_ms}, '{st}', '{m}', '{r}')"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_job_runs(&self, board_id: &str, job_id: Option<i64>, limit: i64) -> anyhow::Result<Vec<JobRun>> {
        let conn = self.conn().await;
        let (sql, params) = match job_id {
            Some(jid) => (
                "SELECT id, board_id, job_id, job_name, triggered_at, duration_ms, status, message, result \
                 FROM wb_job_runs WHERE board_id = ?1 AND job_id = ?2 ORDER BY id DESC LIMIT ?3"
                    .to_string(),
                Params::Positional(vec![Value::from(board_id), Value::Integer(jid), Value::Integer(limit)]),
            ),
            None => (
                "SELECT id, board_id, job_id, job_name, triggered_at, duration_ms, status, message, result \
                 FROM wb_job_runs WHERE board_id = ?1 ORDER BY id DESC LIMIT ?2"
                    .to_string(),
                Params::Positional(vec![Value::from(board_id), Value::Integer(limit)]),
            ),
        };
        let mut rows = conn.query(&sql, params).await?;
        let mut out = Vec::new();
        while let Some(r) = read_job_run(&mut rows).await? {
            out.push(r);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobRun {
    pub id: i64,
    pub board_id: String,
    pub job_id: i64,
    pub job_name: String,
    pub triggered_at: String,
    pub duration_ms: i64,
    pub status: String,
    pub message: String,
    pub result: String,
}

async fn read_job_run(rows: &mut turso::Rows) -> anyhow::Result<Option<JobRun>> {
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let id = match row.get_value(0)? {
        Value::Integer(i) => i,
        _ => 0,
    };
    let board_id = match row.get_value(1)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let job_id = match row.get_value(2)? {
        Value::Integer(i) => i,
        _ => 0,
    };
    let job_name = match row.get_value(3)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let triggered_at = match row.get_value(4)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let duration_ms = match row.get_value(5)? {
        Value::Integer(i) => i,
        _ => 0,
    };
    let status = match row.get_value(6)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let message = match row.get_value(7)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let result = match row.get_value(8)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    Ok(Some(JobRun {
        id,
        board_id,
        job_id,
        job_name,
        triggered_at,
        duration_ms,
        status,
        message,
        result,
    }))
}
