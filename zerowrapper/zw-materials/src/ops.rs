use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};
use zw_db::serverless::srv_board_files_dir;

use crate::layout;
use crate::manifest;
use crate::{diff, snapshot};

impl crate::TemplateStore {
    pub async fn publish(
        &self,
        sid: &str,
        board_id: &str,
        tpl: &str,
        name: &str,
        kind: &str,
        description: &str,
        note: &str,
    ) -> anyhow::Result<Value> {
        if layout::head_path(self.root.as_path(), tpl).is_file() {
            bail!("template {tpl} already exists, use commit to add a version");
        }
        let source = self.sessions.session(sid).await?;
        if source.srv_app_by_id(board_id).await?.is_none() {
            bail!("board {board_id} not found in session {sid}");
        }
        let ver = self.ensure_template(tpl, name, kind, description).await?;
        let src_files = srv_board_files_dir(self.sessions_root.as_path(), sid, board_id);
        let dst = self.version_pool(tpl, ver).await?;
        let dst_files = self.files_dir(tpl, ver);
        let vsid = zw_db::templates::version_sid(tpl, ver);
        let report = snapshot::snapshot_board(
            &source,
            board_id,
            &src_files,
            &dst,
            &vsid,
            &vsid,
            &dst_files,
        )
        .await?;
        self.write_version_meta(tpl, ver, name, kind, description, &report.capabilities, note, &json!({"from": null, "to": format!("{tpl}@{ver}")}))
            .await?;
        Ok(json!({
            "template_id": tpl,
            "version": ver,
            "records": report.records,
            "files": report.files,
            "hooks": report.hooks,
        }))
    }

    pub async fn embed(&self, tpl: &str, ver: i64, sid: &str) -> anyhow::Result<Value> {
        if !self.version_exists(tpl, ver).await? {
            bail!("version {tpl}@{ver} does not exist");
        }
        let pool = self.version_pool(tpl, ver).await?;
        let vsid = zw_db::templates::version_sid(tpl, ver);
        let records = pool.srv_record_count(&vsid).await?;
        let files = crate::store::list_files(&self.files_dir(tpl, ver))?;
        let manifest = manifest::load(self.root.as_path(), tpl, ver)?;
        Ok(json!({
            "template_id": tpl,
            "version": ver,
            "mode": "shared",
            "into_sid": sid,
            "uri": format!("/materials/{tpl}@{ver}"),
            "records": records,
            "files": files.len(),
            "capabilities": manifest.capabilities,
        }))
    }

    pub async fn clone(&self, tpl: &str, ver: i64, sid: &str) -> anyhow::Result<Value> {
        if !self.version_exists(tpl, ver).await? {
            bail!("version {tpl}@{ver} does not exist");
        }
        let board_id = format!("mat:{tpl}@{ver}");
        let session = self.sessions.session(sid).await?;
        if session.srv_app_by_id(&board_id).await?.is_none() {
            let m = manifest::load(self.root.as_path(), tpl, ver)?;
            session.srv_app_create_with_id(&board_id, sid, &m.name, None, true, None).await?;
        }
        let src = self.version_pool(tpl, ver).await?;
        let src_files = self.files_dir(tpl, ver);
        let dst_files = layout::clone_files_dir(self.sessions_root.as_path(), sid, tpl, ver);
        let vsid = zw_db::templates::version_sid(tpl, ver);
        let report = snapshot::snapshot_board(&src, &vsid, &src_files, &session, &board_id, sid, &dst_files).await?;
        let meta = json!({
            "template_id": tpl,
            "from_version": ver,
            "board_id": board_id,
            "cloned_at": crate::store::now_iso(),
        });
        std::fs::create_dir_all(dst_files.parent().unwrap_or(Path::new(".")))?;
        std::fs::write(layout::clone_meta_path(self.sessions_root.as_path(), sid, tpl, ver), serde_json::to_string_pretty(&meta)?)?;
        Ok(json!({
            "template_id": tpl,
            "version": ver,
            "board_id": board_id,
            "uri": format!("/api/srv/{board_id}"),
            "materials": format!("/materials/{tpl}@{ver}"),
            "records": report.records,
            "files": report.files,
        }))
    }

    pub async fn commit(&self, sid: &str, tpl: &str, note: &str) -> anyhow::Result<Value> {
        let meta = self.find_clone_meta(sid, tpl).await?;
        let from_ver = meta["from_version"].as_i64().context("invalid clone meta")?;
        let board_id = meta["board_id"].as_str().context("invalid clone meta")?.to_string();
        let to_ver = self.head(tpl).await? + 1;
        let session = self.sessions.session(sid).await?;
        let src_files = layout::clone_files_dir(self.sessions_root.as_path(), sid, tpl, from_ver);
        let dst = self.version_pool(tpl, to_ver).await?;
        let dst_files = self.files_dir(tpl, to_ver);
        let dst_sid = zw_db::templates::version_sid(tpl, to_ver);
        self.templates.register_board(&dst_sid, &dst_sid, &dst_sid).await?;
        let report = snapshot::snapshot_board(&session, &board_id, &src_files, &dst, &dst_sid, &dst_sid, &dst_files).await?;
        let prev = self.version_pool(tpl, from_ver).await?;
        let prev_files = self.files_dir(tpl, from_ver);
        let prev_sid = zw_db::templates::version_sid(tpl, from_ver);
        let diff_value = diff::build_diff(&prev, &prev_sid, &prev_files, &dst, &dst_sid, &dst_files).await?;
        let summary = format!(
            "commit from {tpl}@{from_ver} to {tpl}@{to_ver}: records={} files={} hooks={}",
            report.records, report.files, report.hooks
        );
        self.write_version_meta(tpl, to_ver, "", "", "", &report.capabilities, &format!("{}\n\n{}", note, summary), &diff_value).await?;
        std::fs::write(layout::head_path(self.root.as_path(), tpl), to_ver.to_string())?;
        self.system.tpl_set_version(tpl, to_ver).await?;
        Ok(json!({
            "template_id": tpl,
            "from": from_ver,
            "to": to_ver,
            "diff": diff_value,
        }))
    }

    pub async fn update(&self, tpl: &str, to_ver: i64, sid: &str) -> anyhow::Result<Value> {
        let meta = self.find_clone_meta(sid, tpl).await?;
        let from_ver = meta["from_version"].as_i64().context("invalid clone meta")?;
        let board_id = meta["board_id"].as_str().context("invalid clone meta")?.to_string();
        if !self.version_exists(tpl, to_ver).await? {
            bail!("version {tpl}@{to_ver} does not exist");
        }
        let session = self.sessions.session(sid).await?;
        session.srv_record_delete(&board_id, None).await?;
        let clone_files = layout::clone_files_dir(self.sessions_root.as_path(), sid, tpl, from_ver);
        snapshot::remove_tree(&clone_files)?;
        let src = self.version_pool(tpl, to_ver).await?;
        let src_files = self.files_dir(tpl, to_ver);
        let src_sid = zw_db::templates::version_sid(tpl, to_ver);
        snapshot::snapshot_board(&src, &src_sid, &src_files, &session, &board_id, sid, &clone_files).await?;
        let new_meta = json!({
            "template_id": tpl,
            "from_version": to_ver,
            "board_id": board_id,
            "cloned_at": crate::store::now_iso(),
        });
        std::fs::write(layout::clone_meta_path(self.sessions_root.as_path(), sid, tpl, to_ver), serde_json::to_string_pretty(&new_meta)?)?;
        std::fs::remove_file(layout::clone_meta_path(self.sessions_root.as_path(), sid, tpl, from_ver))?;
        Ok(json!({ "template_id": tpl, "from": from_ver, "to": to_ver, "board_id": board_id }))
    }

    pub async fn remove(&self, tpl: &str) -> anyhow::Result<()> {
        self.system.tpl_remove(tpl).await?;
        if let Ok(vers) = self.versions(tpl).await {
            for ver in vers {
                let sid = zw_db::templates::version_sid(tpl, ver);
                self.templates.discard(&sid).await;
                let _ = self.templates.unregister_board(&sid).await;
            }
        }
        snapshot::remove_tree(&layout::template_dir(self.root.as_path(), tpl))?;
        let sessions_dir = self.root.as_path().join("sessions");
        if sessions_dir.is_dir() {
            for entry in std::fs::read_dir(&sessions_dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("{tpl}@")) {
                    self.templates.discard(&name).await;
                    snapshot::remove_tree(&entry.path())?;
                }
            }
        }
        Ok(())
    }

    async fn find_clone_meta(&self, sid: &str, tpl: &str) -> anyhow::Result<Value> {
        let materials = self.sessions_root.as_path().join("sessions").join(sid).join("materials");
        if !materials.is_dir() {
            bail!("no clone of {tpl} in session {sid}");
        }
        let mut best: Option<(i64, Value)> = None;
        for entry in std::fs::read_dir(&materials)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let prefix = format!("{tpl}@");
            if !name.starts_with(&prefix) {
                continue;
            }
            let path = entry.path().join("mat.json");
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            let value: Value = serde_json::from_str(&text)?;
            let ver = value["from_version"].as_i64().unwrap_or(0);
            if best.as_ref().map(|(v, _)| ver > *v).unwrap_or(true) {
                best = Some((ver, value));
            }
        }
        match best {
            Some((_, value)) => Ok(value),
            None => bail!("no clone of {tpl} in session {sid}"),
        }
    }

    async fn write_version_meta(
        &self,
        tpl: &str,
        ver: i64,
        name: &str,
        kind: &str,
        description: &str,
        capabilities: &Value,
        note: &str,
        diff_value: &Value,
    ) -> anyhow::Result<()> {
        let m = manifest::load(self.root.as_path(), tpl, ver).unwrap_or(manifest::Manifest {
            template_id: tpl.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            description: description.to_string(),
            version: ver,
            capabilities: json!({}),
            created_at: crate::store::now_iso(),
        });
        let manifest = manifest::Manifest {
            template_id: tpl.to_string(),
            name: if name.is_empty() { m.name } else { name.to_string() },
            kind: if kind.is_empty() { m.kind } else { kind.to_string() },
            description: if description.is_empty() { m.description } else { description.to_string() },
            version: ver,
            capabilities: capabilities.clone(),
            created_at: m.created_at,
        };
        manifest::save(self.root.as_path(), tpl, &manifest)?;
        let change = format!(
            "{}\n\ntimestamp: {}\n\n{}",
            note,
            crate::store::now_iso(),
            diff::human_summary(diff_value)
        );
        std::fs::write(layout::changelog_path(self.root.as_path(), tpl, ver), change)?;
        std::fs::write(layout::diff_path(self.root.as_path(), tpl, ver), serde_json::to_string_pretty(diff_value)?)?;
        Ok(())
    }
}
