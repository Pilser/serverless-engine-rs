use anyhow::bail;
use serde_json::{json, Value};

impl crate::TemplateStore {
    pub async fn query(
        &self,
        tpl: &str,
        ver: i64,
        what: &str,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Value> {
        let ver = if ver <= 0 { self.head(tpl).await? } else { ver };
        if !self.version_exists(tpl, ver).await? {
            bail!("version {tpl}@{ver} does not exist");
        }
        let pool = self.version_pool(tpl, ver).await?;
        let vsid = zw_db::templates::version_sid(tpl, ver);
        match what {
            "info" => self.show(tpl, Some(ver)).await,
            "schema" => {
                let app = pool.srv_app_by_id(&vsid).await?;
                Ok(json!({ "version": ver, "schema": app.and_then(|a| a.schema_json) }))
            }
            "records" => {
                let records = pool.srv_record_list_raw(&vsid, limit, None, offset).await?;
                Ok(serde_json::to_value(records)?)
            }
            "count" => Ok(json!({ "version": ver, "records": pool.srv_record_count(&vsid).await? })),
            "files" => {
                let files = crate::store::list_files(&self.files_dir(tpl, ver))?;
                let out: Vec<Value> = files
                    .into_iter()
                    .map(|(p, s)| json!({ "path": p, "size": s }))
                    .collect();
                Ok(json!({ "version": ver, "files": out }))
            }
            "hooks" => Ok(json!({ "version": ver, "hooks": pool.srv_hook_urls(&vsid).await? })),
            "rate" => Ok(json!({ "version": ver, "rate": pool.srv_rate_config(&vsid).await? })),
            "ttl" => {
                let ttl = pool.srv_ttl_get(&vsid).await?;
                Ok(json!({ "version": ver, "ttl": { "seconds": ttl.seconds, "field": ttl.field } }))
            }
            _ => bail!("unknown query {what} (info|schema|records|count|files|hooks|rate|ttl)"),
        }
    }
}
