use std::time::Duration as StdDuration;
use chrono::{NaiveDateTime, Utc};
use serde_json::Value;
use tokio::time;
use zw_db::SessionStore;
use zw_db::cron::next_run;
use zw_db::jobs::SrvJob;

pub fn spawn_scheduler(store: SessionStore) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { run_worker(store).await })
}

fn fmt_iso(now: &NaiveDateTime) -> String {
    now.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn extract_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let p = path.trim().trim_start_matches("$");
    let p = p.strip_prefix('.').unwrap_or(p);
    if p.is_empty() {
        return Some(value);
    }
    let mut cur = value;
    for seg in p.split('.') {
        let key = seg.trim_matches('[').trim_matches(']');
        if key.is_empty() {
            continue;
        }
        if let Some(idx) = key.parse::<usize>().ok() {
            cur = cur.get(idx)?;
        } else {
            cur = cur.get(key)?;
        }
    }
    Some(cur)
}

async fn mark(store: &SessionStore, job: &SrvJob, now: &NaiveDateTime, started: &std::time::Instant, status: &str, message: &str) {
    let duration_ms = started.elapsed().as_millis() as i64;
    let iso = fmt_iso(now);
    let system = store.system();
    if let Err(e) = system.srv_job_mark(job.id, &iso, status, message).await {
        tracing::warn!("scheduler: mark job {} failed: {e}", job.id);
    }
    if let Err(e) = system
        .srv_job_run_insert(&job.board_id, job.id, &job.name, &iso, duration_ms, status, message, "")
        .await
    {
        tracing::warn!("scheduler: record job run {} failed: {e}", job.id);
    }
}

async fn run_job(store: &SessionStore, client: &reqwest::Client, job: SrvJob) {
    let now = Utc::now().naive_utc();
    let started = std::time::Instant::now();
    let next = next_run(&job.schedule, now);
    if let Err(e) = store
        .system()
        .srv_job_reschedule(job.id, next.as_ref().map(fmt_iso).as_deref())
        .await
    {
        tracing::warn!("scheduler: reschedule job {} failed: {e}", job.id);
    }

    dispatch_cron_recipes(store, &job).await;

    let action: Value = match serde_json::from_str(&job.action) {
        Ok(v) => v,
        Err(e) => {
            mark(store, &job, &now, &started, "error", &format!("invalid action json: {e}")).await;
            return;
        }
    };
    let kind = action.get("type").and_then(|t| t.as_str()).unwrap_or("http");
    let Some(url) = action.get("url").and_then(|u| u.as_str()) else {
        mark(store, &job, &now, &started, "error", "missing action url").await;
        return;
    };
    if !zw_db::netguard::valid_url(url) {
        mark(store, &job, &now, &started, "error", "ssrf-blocked url").await;
        return;
    }
    let method = action.get("method").and_then(|m| m.as_str()).unwrap_or("GET");
    let headers: Vec<(String, String)> = action
        .get("headers")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let k = p.get(0).and_then(|k| k.as_str())?;
                    let v = p.get(1).and_then(|v| v.as_str())?;
                    Some((k.to_string(), v.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let body = action.get("body").map(|b| b.to_string());
    let timeout_ms: u64 = action.get("timeout_ms").and_then(|t| t.as_u64()).unwrap_or(15_000);

    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut req = client.request(method, url);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    if let Some(b) = &body {
        req = req.body(b.clone());
    }
    let res = match req.timeout(StdDuration::from_millis(timeout_ms)).send().await {
        Ok(r) => r,
        Err(e) => {
            mark(store, &job, &now, &started, "error", &format!("request failed: {e}")).await;
            return;
        }
    };
    let status = res.status().as_u16();
    let text = match res.text().await {
        Ok(t) => t,
        Err(e) => {
            mark(store, &job, &now, &started, "error", &format!("read body failed: {e}")).await;
            return;
        }
    };

    if kind == "poll" {
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let path = action.get("path").and_then(|p| p.as_str()).unwrap_or("$");
        let record = extract_path(&parsed, path).cloned().unwrap_or(Value::Null);
        if record.is_object() {
            match store.board_pool(&job.board_id).await {
                Ok(pool) => match pool.srv_record_insert(&job.board_id, &record, Some(&job.name), false).await {
                    Ok(seq) => mark(store, &job, &now, &started, "ok", &format!("http {status}, inserted seq {seq}")).await,
                    Err(e) => mark(store, &job, &now, &started, "error", &format!("http {status}, insert failed: {e}")).await,
                },
                Err(e) => mark(store, &job, &now, &started, "error", &format!("http {status}, board lookup failed: {e}")).await,
            }
        } else {
            mark(store, &job, &now, &started, "ok", &format!("http {status}, path '{path}' did not yield an object")).await;
        }
    } else {
        mark(store, &job, &now, &started, "ok", &format!("http {status}")).await;
    }
}

async fn dispatch_cron_recipes(store: &SessionStore, job: &SrvJob) {
    let pool = match store.board_pool(&job.board_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("scheduler: board pool for job {} failed: {e}", job.id);
            return;
        }
    };
    let recipes = match pool.srv_recipe_list(&job.board_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("scheduler: recipe list for job {} failed: {e}", job.id);
            return;
        }
    };
    let wanted = format!("cron:{}", job.name);
    let mut secrets = zw_recipe::Secrets::new();
    if let Ok(map) = pool.srv_secrets_map(&job.board_id).await {
        for (name, value) in map {
            secrets.insert(&name, &value);
        }
    }
    let engine = zw_recipe::Engine { db: pool.as_ref() };
    for sr in recipes {
        let Some(recipe) = to_recipe(&sr) else { continue };
        if recipe.when != wanted {
            continue;
        }
        let ctx = zw_recipe::EventContext::cron(&job.board_id, &job.name, &Value::Null);
        engine.run_recipe(&recipe, &secrets, &ctx, false).await;
    }
}

fn to_recipe(sr: &zw_db::recipes::SrvRecipe) -> Option<zw_recipe::Recipe> {
    let actions = match sr.actions_json.as_array() {
        Some(arr) => arr.clone(),
        None => return None,
    };
    Some(zw_recipe::Recipe {
        name: sr.name.clone(),
        when: sr.when_json.clone(),
        match_json: sr.match_json.clone(),
        enabled: sr.enabled,
        dedup_on: sr.dedup_on.clone(),
        actions,
    })
}

async fn run_worker(store: SessionStore) {    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("scheduler: failed to build http client: {e}");
            return;
        }
    };
    loop {
        time::sleep(StdDuration::from_secs(15)).await;
        let now = Utc::now().naive_utc();
        let now_iso = fmt_iso(&now);
        let due = match store.system().srv_job_due(&now_iso, 32).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("scheduler: due query failed: {e}");
                continue;
            }
        };
        for job in due {
            run_job(&store, &client, job).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_path;
    use serde_json::json;

    #[test]
    fn extract_path_root_and_dotted() {
        let v = json!({"a": {"b": [10, 20, {"c": "x"}]}});
        assert_eq!(extract_path(&v, "$"), Some(&v));
        assert_eq!(extract_path(&v, "$.a.b[2].c"), Some(&json!("x")));
        assert_eq!(extract_path(&v, "a.b[1]"), Some(&json!(20)));
        assert_eq!(extract_path(&v, "$.a.b"), Some(&json!([10, 20, {"c": "x"}])));
    }

    #[test]
    fn extract_path_missing_returns_none() {
        let v = json!({"a": {"b": [10]}});
        assert!(extract_path(&v, "$.missing").is_none());
        assert!(extract_path(&v, "$.a.b[9]").is_none());
        assert!(extract_path(&v, "$.a.b.c").is_none(), "traversing a scalar with a key fails");
    }

    #[test]
    fn extract_path_scalar_root_and_unprefixed() {
        let v = json!({"x": 5});
        assert_eq!(extract_path(&v, "x"), Some(&json!(5)), "path without a leading $ works");
        assert_eq!(extract_path(&json!(7), "$"), Some(&json!(7)), "non-object root is returned whole");
    }
}
