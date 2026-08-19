use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::bail;
use serde_json::{json, Value};
use zw_db::DbPool;

use crate::layout;
use crate::manifest;

impl crate::TemplateStore {
    pub async fn head(&self, tpl: &str) -> anyhow::Result<i64> {
        let path = layout::head_path(self.root.as_path(), tpl);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(ver) = text.trim().parse::<i64>() {
                return Ok(ver);
            }
        }
        match self.system.tpl_get(tpl).await? {
            Some(row) => Ok(row.current_version),
            None => bail!("template {tpl} does not exist"),
        }
    }

    pub async fn version_pool(&self, tpl: &str, ver: i64) -> anyhow::Result<Arc<DbPool>> {
        let sid = zw_db::templates::version_sid(tpl, ver);
        self.templates.session(&sid).await
    }

    pub async fn version_exists(&self, tpl: &str, ver: i64) -> anyhow::Result<bool> {
        if !layout::manifest_path(self.root.as_path(), tpl, ver).is_file() {
            return Ok(false);
        }
        let sid = zw_db::templates::version_sid(tpl, ver);
        Ok(self
            .version_pool(tpl, ver)
            .await?
            .srv_app_by_id(&sid)
            .await?
            .is_some())
    }

    pub async fn ensure_template(
        &self,
        tpl: &str,
        name: &str,
        kind: &str,
        description: &str,
    ) -> anyhow::Result<i64> {
        if !layout::is_valid_template_id(tpl) {
            bail!("invalid template_id: {tpl}");
        }
        if layout::head_path(self.root.as_path(), tpl).is_file() {
            let ver = self.head(tpl).await?;
            self.system.tpl_create(tpl, name, kind, description).await?;
            return Ok(ver);
        }
        let ver = 1;
        let sid = zw_db::templates::version_sid(tpl, ver);
        let pool = self.templates.session(&sid).await?;
        if pool.srv_app_by_id(&sid).await?.is_none() {
            pool.srv_app_create_with_id(&sid, &sid, name, None, true, None).await?;
        } else {
            pool.srv_app_update(&sid, &sid, Some(name), None, Some(true), None).await?;
        }
        self.templates.register_board(&sid, &sid, &sid).await?;
        let manifest = manifest::Manifest {
            template_id: tpl.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            description: description.to_string(),
            version: ver,
            capabilities: json!({}),
            created_at: now_iso(),
        };
        manifest::save(self.root.as_path(), tpl, &manifest)?;
        std::fs::create_dir_all(layout::files_dir(self.root.as_path(), tpl, ver))?;
        std::fs::write(layout::head_path(self.root.as_path(), tpl), ver.to_string())?;
        self.system.tpl_create(tpl, name, kind, description).await?;
        Ok(ver)
    }

    pub async fn list(&self) -> anyhow::Result<Value> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.root.as_path())? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "sessions" || !entry.file_type()?.is_dir() {
                continue;
            }
            let head = match self.head(&name).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let manifest = manifest::load(self.root.as_path(), &name, head).unwrap_or(manifest::Manifest {
                template_id: name.clone(),
                name: name.clone(),
                kind: "app".to_string(),
                description: String::new(),
                version: head,
                capabilities: json!({}),
                created_at: String::new(),
            });
            let pool = self.version_pool(&name, head).await?;
            let vsid = zw_db::templates::version_sid(&name, head);
            let records = pool.srv_record_count(&vsid).await.unwrap_or(0);
            let files = list_files(&layout::files_dir(self.root.as_path(), &name, head))
                .map(|f| f.len())
                .unwrap_or(0);
            out.push(json!({
                "template_id": name,
                "name": manifest.name,
                "kind": manifest.kind,
                "description": manifest.description,
                "current_version": head,
                "records": records,
                "files": files,
            }));
        }
        Ok(Value::Array(out))
    }

    pub async fn show(&self, tpl: &str, ver: Option<i64>) -> anyhow::Result<Value> {
        let ver = match ver {
            Some(v) => v,
            None => self.head(tpl).await?,
        };
        if !self.version_exists(tpl, ver).await? {
            bail!("version {tpl}@{ver} does not exist");
        }
        let manifest = manifest::load(self.root.as_path(), tpl, ver)?;
        let pool = self.version_pool(tpl, ver).await?;
        let vsid = zw_db::templates::version_sid(tpl, ver);
        let app = pool.srv_app_by_id(&vsid).await?;
        let rate = pool.srv_rate_config(&vsid).await?;
        let ttl = pool.srv_ttl_get(&vsid).await?;
        let hooks = pool.srv_hook_urls(&vsid).await?;
        let records = pool.srv_record_count(&vsid).await?;
        let files = list_files(&layout::files_dir(self.root.as_path(), tpl, ver))?;
        let changelog = std::fs::read_to_string(layout::changelog_path(self.root.as_path(), tpl, ver))
            .unwrap_or_default();
        let diff = std::fs::read_to_string(layout::diff_path(self.root.as_path(), tpl, ver))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        Ok(json!({
            "template_id": tpl,
            "name": manifest.name,
            "kind": manifest.kind,
            "description": manifest.description,
            "version": ver,
            "created_at": manifest.created_at,
            "schema": app.as_ref().map(|a| a.schema_json.clone()).flatten(),
            "public_reads": app.as_ref().map(|a| a.public_reads).unwrap_or(false),
            "unique_key": app.as_ref().and_then(|a| a.unique_key.clone()),
            "rate": rate,
            "ttl": { "seconds": ttl.seconds, "field": ttl.field },
            "hooks": hooks,
            "records": records,
            "files": files,
            "changelog": changelog,
            "diff": diff,
        }))
    }

    pub async fn versions(&self, tpl: &str) -> anyhow::Result<Vec<i64>> {
        let dir = layout::template_dir(self.root.as_path(), tpl).join("versions");
        if !dir.is_dir() {
            bail!("template {tpl} does not exist");
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if let Ok(ver) = entry.file_name().to_string_lossy().parse::<i64>() {
                out.push(ver);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    pub fn files_dir(&self, tpl: &str, ver: i64) -> PathBuf {
        layout::files_dir(self.root.as_path(), tpl, ver)
    }
}

pub fn list_files(dir: &Path) -> anyhow::Result<Vec<(String, u64)>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(dir)?.to_string_lossy().to_string();
            out.push((rel, entry.metadata().map(|m| m.len()).unwrap_or(0)));
        }
    }
    out.sort();
    Ok(out)
}

pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}
