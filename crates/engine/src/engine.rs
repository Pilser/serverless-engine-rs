use crate::model::{Board, KeyRecord, Principal, Recipe, Record, Secret};
use crate::storage::database::{Database, DatabaseCaps};
use crate::storage::ir::{Agg, SrvFilter};
use crate::storage::object_store::ObjectStore;
use serde_json::{json, Value as Json};
use std::sync::Arc;

pub type Notify = Arc<dyn Fn(String, String, i64, Json) + Send + Sync>;

/// Facade over the whole serverless engine. Owns a `Database` and an
/// `ObjectStore`; every operation in the system goes through here.
pub struct ServerlessEngine {
    db: Box<dyn Database>,
    store: Box<dyn ObjectStore>,
    notify: Option<Notify>,
}

impl ServerlessEngine {
    pub fn new(db: Box<dyn Database>, store: Box<dyn ObjectStore>) -> Self {
        Self { db, store, notify: None }
    }

    pub fn with_defaults() -> Self {
        Self::new(
            Box::new(crate::storage::memory::InMemoryDatabase::new()),
            Box::new(crate::storage::memory::InMemoryObjectStore::new()),
        )
    }

    pub fn set_notifier(&mut self, notify: Option<Notify>) {
        self.notify = notify;
    }

    fn emit(&self, board: &str, kind: &str, seq: i64, payload: Json) {
        if let Some(f) = &self.notify {
            f(board.to_string(), kind.to_string(), seq, payload);
        }
    }

    pub fn database(&self) -> &dyn Database {
        self.db.as_ref()
    }

    pub fn object_store(&self) -> &dyn ObjectStore {
        self.store.as_ref()
    }

    pub fn caps(&self) -> DatabaseCaps {
        self.db.capabilities()
    }

    // ---- app lifecycle -------------------------------------------------

    pub fn create_app(
        &mut self,
        owner: &str,
        title: &str,
        schema: Option<Json>,
        public_reads: bool,
        unique_key: Option<&str>,
    ) -> anyhow::Result<Board> {
        crate::crud::app_create(self.db.as_mut(), owner, title, schema, public_reads, unique_key)
    }

    pub fn get_app(&self, board_id: &str) -> anyhow::Result<Option<Board>> {
        crate::crud::app_by_id(self.db.as_ref(), board_id)
    }

    pub fn list_apps(&self, owner: &str) -> anyhow::Result<Vec<Board>> {
        crate::crud::app_list(self.db.as_ref(), owner)
    }

    pub fn update_app(&mut self, board_id: &str, owner: &str, patch: &Json) -> anyhow::Result<()> {
        crate::crud::app_update(self.db.as_mut(), board_id, owner, patch)
    }

    pub fn delete_app(&mut self, board_id: &str, owner: &str) -> anyhow::Result<()> {
        crate::crud::app_delete(self.db.as_mut(), board_id, owner)
    }

    // ---- records -------------------------------------------------------

    pub fn insert_record(
        &mut self,
        board_id: &str,
        table: &str,
        payload: Json,
        writer: Option<&str>,
        upsert: bool,
        principal: &Principal,
    ) -> anyhow::Result<i64> {
        let seq = crate::crud::record_insert(self.db.as_mut(), board_id, table, payload.clone(), writer, upsert, principal)?;
        self.emit(board_id, "created", seq, payload);
        Ok(seq)
    }

    pub fn bulk_insert(
        &mut self,
        board_id: &str,
        table: &str,
        records: Vec<Json>,
        writer: Option<&str>,
        upsert: bool,
        principal: &Principal,
    ) -> anyhow::Result<Vec<i64>> {
        let seqs = crate::crud::record_bulk_insert(
            self.db.as_mut(),
            board_id,
            table,
            records.clone(),
            writer,
            upsert,
            principal,
        )?;
        for (seq, record) in seqs.iter().zip(records.into_iter()) {
            self.emit(board_id, "created", *seq, record);
        }
        Ok(seqs)
    }

    /// Import records from a JSON (array/JSONL) or CSV dump. Each row is
    /// validated against the table schema; invalid rows are skipped and
    /// reported rather than aborting the whole import.
    pub fn import_records(
        &mut self,
        board_id: &str,
        table: &str,
        format: &str,
        data: &str,
        separator: char,
        upsert: bool,
        principal: &Principal,
    ) -> anyhow::Result<Json> {
        let records = crate::import::parse(format, data, separator)?;
        let mut seqs = Vec::new();
        let mut errors = Vec::new();
        for record in records {
            match self.insert_record(
                board_id,
                table,
                record.clone(),
                Some(&principal.id),
                upsert,
                principal,
            ) {
                Ok(seq) => seqs.push(seq),
                Err(e) => errors.push(json!({ "record": record, "error": e.to_string() })),
            }
        }
        Ok(json!({
            "total": seqs.len() + errors.len(),
            "inserted": seqs.len(),
            "seqs": seqs,
            "errors": errors,
        }))
    }

    pub fn get_record(&self, board_id: &str, table: &str, seq: i64) -> anyhow::Result<Option<Record>> {
        crate::crud::record_get(self.db.as_ref(), board_id, table, seq)
    }

    pub fn set_record(
        &mut self,
        board_id: &str,
        table: &str,
        seq: i64,
        payload: Json,
        writer: Option<&str>,
    ) -> anyhow::Result<()> {
        crate::crud::record_set(self.db.as_mut(), board_id, table, seq, payload.clone(), writer)?;
        self.emit(board_id, "updated", seq, payload);
        Ok(())
    }

    pub fn patch_record(
        &mut self,
        board_id: &str,
        table: &str,
        seq: i64,
        ops: &Json,
        writer: Option<&str>,
    ) -> anyhow::Result<Json> {
        let merged = crate::crud::record_patch(self.db.as_mut(), board_id, table, seq, ops, writer)?;
        self.emit(board_id, "updated", seq, merged.clone());
        Ok(merged)
    }

    pub fn patch_first(
        &mut self,
        board_id: &str,
        table: &str,
        conds: &SrvFilter,
        ops: &Json,
    ) -> anyhow::Result<Option<Json>> {
        crate::crud::record_patch_first(self.db.as_mut(), board_id, table, conds, ops)
    }

    pub fn delete_record(&mut self, board_id: &str, table: &str, seq: i64) -> anyhow::Result<bool> {
        let deleted = crate::crud::record_delete_one(self.db.as_mut(), board_id, table, seq)?;
        if deleted {
            self.emit(board_id, "deleted", seq, Json::Null);
        }
        Ok(deleted)
    }

    pub fn delete_records(&mut self, board_id: &str, table: &str, conds: &SrvFilter) -> anyhow::Result<usize> {
        crate::crud::record_delete_filter(self.db.as_mut(), board_id, table, conds)
    }

    pub fn list_records(
        &self,
        board_id: &str,
        table: &str,
        limit: usize,
        before: Option<i64>,
        offset: usize,
    ) -> anyhow::Result<Vec<Record>> {
        crate::crud::record_list(self.db.as_ref(), board_id, table, limit, before, offset)
    }

    pub fn count_records(&self, board_id: &str, table: &str) -> anyhow::Result<i64> {
        crate::crud::record_count(self.db.as_ref(), board_id, table)
    }

    pub fn records_after(&self, board_id: &str, after: i64) -> anyhow::Result<Vec<Record>> {
        crate::crud::records_after(self.db.as_ref(), board_id, after)
    }

    // ---- tables ---------------------------------------------------------

    pub fn create_table(
        &mut self,
        board_id: &str,
        table: &str,
        schema: Option<Json>,
        unique_key: Option<&str>,
    ) -> anyhow::Result<crate::model::TableConfig> {
        crate::crud::table_create(self.db.as_mut(), board_id, table, schema, unique_key)
    }

    pub fn list_tables(&self, board_id: &str) -> anyhow::Result<Vec<crate::model::TableConfig>> {
        crate::crud::table_list(self.db.as_ref(), board_id)
    }

    pub fn get_table(&self, board_id: &str, table: &str) -> anyhow::Result<Option<crate::model::TableConfig>> {
        crate::crud::table_get(self.db.as_ref(), board_id, table)
    }

    pub fn drop_table(&mut self, board_id: &str, table: &str) -> anyhow::Result<bool> {
        crate::crud::table_delete(self.db.as_mut(), board_id, table)
    }

    // ---- query / search / aggregate / join ----------------------------

    pub fn query_records(
        &self,
        board_id: &str,
        table: &str,
        filter: &SrvFilter,
        orders: &[(String, bool)],
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<Record>> {
        crate::query::query_records(
            self.db.as_ref(),
            board_id,
            table,
            filter,
            orders,
            limit,
            offset,
        )
    }

    pub fn search_records(
        &self,
        board_id: &str,
        table: &str,
        query: &str,
        conds: &SrvFilter,
        limit: usize,
        offset: usize,
        snippet: bool,
    ) -> anyhow::Result<Vec<Record>> {
        crate::query::search_records(
            self.db.as_ref(),
            board_id,
            table,
            query,
            conds,
            limit,
            offset,
            snippet,
        )
    }

    pub fn aggregate_records(
        &self,
        board_id: &str,
        table: &str,
        conds: &SrvFilter,
        agg: Agg,
        field: Option<&str>,
        group_by: Option<&str>,
    ) -> anyhow::Result<Vec<Json>> {
        crate::query::aggregate_records(
            self.db.as_ref(),
            board_id,
            table,
            conds,
            agg,
            field,
            group_by,
        )
    }

    pub fn join_list(
        &self,
        child_board: &str,
        child_table: &str,
        conds: &SrvFilter,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<Record>> {
        crate::policy::join_list(self.db.as_ref(), child_board, child_table, conds, limit, offset)
    }

    // ---- keys ----------------------------------------------------------

    pub fn issue_key(
        &mut self,
        board_id: &str,
        role: &str,
        writer: Option<&str>,
        scope: Option<&str>,
    ) -> anyhow::Result<(KeyRecord, String)> {
        crate::auth::issue_key(self.db.as_mut(), board_id, role, writer, scope)
    }

    pub fn list_keys(&self, board_id: &str) -> anyhow::Result<Vec<KeyRecord>> {
        crate::auth::list_keys(self.db.as_ref(), board_id)
    }

    pub fn revoke_key(&mut self, bucket: &str) -> anyhow::Result<()> {
        crate::auth::revoke_key(self.db.as_mut(), bucket)
    }

    pub fn signup_user(
        &mut self,
        board_id: &str,
        email: &str,
        password: &str,
        password_hash: Option<&str>,
        role: &str,
        caller: &Principal,
    ) -> anyhow::Result<crate::auth::User> {
        crate::auth::user_signup(
            self.db.as_mut(),
            board_id,
            email,
            password,
            password_hash,
            role,
            caller,
        )
    }

    pub fn login_user(
        &mut self,
        board_id: &str,
        email: &str,
        password: &str,
    ) -> anyhow::Result<(String, String)> {
        crate::auth::user_login(self.db.as_mut(), board_id, email, password)
    }

    pub fn logout_user(&mut self, token: &str) -> anyhow::Result<bool> {
        crate::auth::user_logout(self.db.as_mut(), token)
    }

    pub fn user_by_token(
        &self,
        board_id: &str,
        token: &str,
    ) -> anyhow::Result<Option<crate::auth::User>> {
        crate::auth::resolve_session(self.db.as_ref(), board_id, token)
    }

    pub fn resolve_principal(
        &self,
        board_id: &str,
        token: Option<&str>,
        scope: Option<&str>,
    ) -> anyhow::Result<Principal> {
        crate::auth::resolve_principal(self.db.as_ref(), board_id, token, scope)
    }

    // ---- recipes -------------------------------------------------------

    pub fn add_recipe(&mut self, board_id: &str, recipe: &Recipe) -> anyhow::Result<()> {
        crate::automation::recipe_add(self.db.as_mut(), board_id, recipe)
    }

    pub fn list_recipes(&self, board_id: &str) -> anyhow::Result<Vec<Recipe>> {
        crate::automation::recipe_list(self.db.as_ref(), board_id)
    }

    pub fn remove_recipe(&mut self, board_id: &str, name: &str) -> anyhow::Result<()> {
        crate::automation::recipe_remove(self.db.as_mut(), board_id, name)
    }

    pub fn set_recipe_enabled(&mut self, board_id: &str, name: &str, enabled: bool) -> anyhow::Result<()> {
        crate::automation::recipe_enabled(self.db.as_mut(), board_id, name, enabled)
    }

    // ---- secrets -------------------------------------------------------

    pub fn set_secret(&mut self, board_id: &str, name: &str, value: &str) -> anyhow::Result<()> {
        crate::secrets::secret_set(self.db.as_mut(), board_id, name, value)
    }

    pub fn list_secrets(&self, board_id: &str) -> anyhow::Result<Vec<Secret>> {
        crate::secrets::secret_list(self.db.as_ref(), board_id)
    }

    pub fn secrets_map(&self, board_id: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
        crate::secrets::secrets_map(self.db.as_ref(), board_id)
    }

    pub fn remove_secret(&mut self, board_id: &str, name: &str) -> anyhow::Result<()> {
        crate::secrets::secret_remove(self.db.as_mut(), board_id, name)
    }

    // ---- control plane (config) ---------------------------------------

    pub fn set_rate(&mut self, board_id: &str, rate: &Json) -> anyhow::Result<()> {
        crate::policy::rate_set(self.db.as_mut(), board_id, rate)
    }

    pub fn set_ttl(
        &mut self,
        board_id: &str,
        table: &str,
        seconds: Option<i64>,
        field: Option<&str>,
    ) -> anyhow::Result<()> {
        crate::policy::ttl_set(self.db.as_mut(), board_id, table, seconds, field)
    }

    pub fn clear_ttl(&mut self, board_id: &str, table: &str) -> anyhow::Result<()> {
        crate::policy::ttl_clear(self.db.as_mut(), board_id, table)
    }

    pub fn set_link(&mut self, board_id: &str, child_table: &str, parent: &str, parent_table: &str, from_key: &str, parent_key: &str) -> anyhow::Result<()> {
        crate::policy::link_set(self.db.as_mut(), board_id, child_table, parent, parent_table, from_key, parent_key)
    }

    pub fn clear_link(&mut self, board_id: &str) -> anyhow::Result<()> {
        crate::policy::link_clear(self.db.as_mut(), board_id)
    }

    pub fn set_computed(&mut self, board_id: &str, table: &str, map: &Json) -> anyhow::Result<()> {
        crate::schema::computed_set(self.db.as_mut(), board_id, table, map)
    }

    pub fn set_validate(&mut self, board_id: &str, table: &str, rules: &Json) -> anyhow::Result<()> {
        crate::schema::validate_set(self.db.as_mut(), board_id, table, rules)
    }

    pub fn set_redact(&mut self, board_id: &str, table: &str, paths: &Json) -> anyhow::Result<()> {
        crate::schema::redact_set(self.db.as_mut(), board_id, table, paths)
    }

    pub fn set_webhook_secret(&mut self, board_id: &str, secret: Option<&str>) -> anyhow::Result<()> {
        crate::webhooks::webhook_secret_set(self.db.as_mut(), board_id, secret)
    }

    // ---- webhooks ------------------------------------------------------

    pub fn register_hook(&mut self, board_id: &str, url: &str, secret: Option<&str>) -> anyhow::Result<()> {
        crate::webhooks::hook_register(self.db.as_mut(), board_id, url, secret)
    }

    pub fn list_hooks(&self, board_id: &str) -> anyhow::Result<Vec<crate::model::Hook>> {
        crate::webhooks::hook_list(self.db.as_ref(), board_id)
    }

    pub fn remove_hook(&mut self, board_id: &str, url: &str) -> anyhow::Result<()> {
        crate::webhooks::hook_remove(self.db.as_mut(), board_id, url)
    }

    // ---- audit ---------------------------------------------------------

    pub fn set_audit(&mut self, board_id: &str, enabled: bool) -> anyhow::Result<()> {
        crate::audit::audit_toggle(self.db.as_mut(), board_id, enabled)
    }

    pub fn audit_list(
        &self,
        board_id: &str,
        since: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Json>> {
        crate::audit::audit_list(self.db.as_ref(), board_id, since, limit)
    }

    // ---- files ---------------------------------------------------------

    pub fn upload(
        &mut self,
        board_id: &str,
        table: &str,
        filename: &str,
        content_type: &str,
        bytes: &[u8],
        meta: &Json,
    ) -> anyhow::Result<i64> {
        let seq = crate::files::file_upload(self.db.as_mut(), self.store.as_ref(), board_id, table, filename, content_type, bytes, meta)?;
        self.emit(board_id, "created", seq, meta.clone());
        Ok(seq)
    }

    pub fn download(&self, board_id: &str, table: &str, file: &str) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        crate::files::file_download(self.db.as_ref(), self.store.as_ref(), board_id, table, file)
    }

    pub fn list_files(&self, board_id: &str) -> anyhow::Result<Vec<crate::storage::object_store::KeyInfo>> {
        crate::files::file_list(self.store.as_ref(), board_id)
    }

    // ---- assets (per-board static hosting) ----------------------------

    pub fn put_asset(&self, board_id: &str, rel: &str, bytes: &[u8]) -> anyhow::Result<()> {
        crate::files::asset_put(self.store.as_ref(), board_id, rel, bytes)
    }

    pub fn get_asset(&self, board_id: &str, rel: &str) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        crate::files::asset_get(self.store.as_ref(), board_id, rel)
    }

    pub fn head_asset(&self, board_id: &str, rel: &str) -> anyhow::Result<Option<crate::storage::object_store::BlobMeta>> {
        crate::files::asset_head(self.store.as_ref(), board_id, rel)
    }

    pub fn delete_asset(&self, board_id: &str, rel: &str) -> anyhow::Result<bool> {
        crate::files::asset_delete(self.store.as_ref(), board_id, rel)
    }

    pub fn list_assets(&self, board_id: &str) -> anyhow::Result<Vec<crate::storage::object_store::KeyInfo>> {
        crate::files::asset_list(self.store.as_ref(), board_id)
    }

    // ---- automation trigger -------------------------------------------

    pub fn dispatch_recipes(
        &mut self,
        board_id: &str,
        table: &str,
        event: crate::events::EventKind,
        seq: Option<i64>,
        payload: Option<Json>,
    ) -> anyhow::Result<()> {
        crate::automation::dispatch(self.db.as_mut(), board_id, table, event, seq, payload)
    }

    pub fn ttl_sweep(&mut self) -> anyhow::Result<usize> {
        crate::policy::ttl_sweep(self.db.as_mut())
    }

    // ---- outbound http --------------------------------------------------

    pub fn install_http_caller(&self, caller: Box<dyn crate::http::HttpCaller>) {
        crate::http::install(caller);
    }

    // ---- cron jobs ------------------------------------------------------

    pub fn add_job(
        &mut self,
        board_id: &str,
        name: &str,
        schedule: &str,
        action: &Json,
    ) -> anyhow::Result<String> {
        crate::jobs::job_add(self.db.as_mut(), board_id, name, schedule, action)
    }

    pub fn list_jobs(&self, board_id: &str) -> anyhow::Result<Vec<crate::model::Job>> {
        crate::jobs::job_list(self.db.as_ref(), board_id)
    }

    pub fn remove_job(&mut self, board_id: &str, name: &str) -> anyhow::Result<bool> {
        crate::jobs::job_remove(self.db.as_mut(), board_id, name)
    }

    pub fn job_due(&self, now_iso: &str, limit: usize) -> anyhow::Result<Vec<crate::model::Job>> {
        crate::jobs::job_due(self.db.as_ref(), now_iso, limit)
    }

    pub fn job_reschedule(
        &mut self,
        board_id: &str,
        name: &str,
        next_iso: Option<&str>,
    ) -> anyhow::Result<()> {
        crate::jobs::job_reschedule(self.db.as_mut(), board_id, name, next_iso)
    }

    pub fn job_mark(
        &mut self,
        board_id: &str,
        name: &str,
        last_run_at: &str,
        status: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        crate::jobs::job_mark(self.db.as_mut(), board_id, name, last_run_at, status, message)
    }

    pub fn job_run_insert(
        &mut self,
        board_id: &str,
        job_name: &str,
        triggered_at: &str,
        duration_ms: i64,
        status: &str,
        message: &str,
        result: &str,
    ) -> anyhow::Result<()> {
        crate::jobs::job_run_insert(
            self.db.as_mut(),
            board_id,
            job_name,
            triggered_at,
            duration_ms,
            status,
            message,
            result,
        )
    }

    pub fn job_runs(
        &self,
        board_id: &str,
        job_name: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::model::JobRun>> {
        crate::jobs::job_runs(self.db.as_ref(), board_id, job_name, limit)
    }

    pub fn dispatch_cron_job(&mut self, board_id: &str, job_name: &str) -> anyhow::Result<()> {
        crate::automation::dispatch_cron(self.db.as_mut(), board_id, job_name)
    }

    // ---- webhook delivery queue ----------------------------------------

    pub fn hook_deliveries_due(&self, now_iso: &str, limit: usize) -> anyhow::Result<Vec<Json>> {
        crate::webhooks::hook_deliveries_due(self.db.as_ref(), now_iso, limit)
    }

    pub fn mark_hook_delivery(
        &mut self,
        id: &str,
        attempts: i64,
        last_status: Option<&str>,
        next_attempt: Option<&str>,
        delivered_at: Option<&str>,
    ) -> anyhow::Result<()> {
        crate::webhooks::hook_mark_delivery(
            self.db.as_mut(),
            id,
            attempts,
            last_status,
            next_attempt,
            delivered_at,
        )
    }
}
