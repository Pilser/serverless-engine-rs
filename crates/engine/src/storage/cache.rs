use crate::model::Key;
use crate::storage::database::{Cursor, Database, DatabaseCaps, Query, Row, TtlClause};
use crate::storage::ir::SrvFilter;
use crate::storage::object_store::{BlobMeta, KeyInfo, ObjectStore, ObjectStoreCaps, PutInfo};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DEFAULT_TTL_SECS: u64 = 30;
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone)]
enum Cached {
    Row(Row),
    Cursor(Cursor),
}

struct Entry {
    expires: Instant,
    value: Cached,
}

struct CacheData {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let d = h.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in d {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn row_ttl(data: &Json) -> Duration {
    let secs = data.get("ttl_seconds").and_then(|v| v.as_i64()).unwrap_or(DEFAULT_TTL_SECS as i64);
    Duration::from_secs(secs.max(1) as u64)
}

fn key_repr(k: &Key) -> String {
    match k {
        Key::Int(v) => format!("int:{v}"),
        Key::Text(s) => format!("text:{s}"),
    }
}

fn query_hash(table: &str, q: &Query) -> String {
    let mut s = String::new();
    for c in &q.filter.conds {
        s.push_str(&c.field);
        s.push('|');
        s.push_str(c.op.as_str());
        s.push('|');
        s.push_str(&serde_json::to_string(&c.value).unwrap_or_default());
        s.push(';');
    }
    s.push_str("o:");
    for (f, asc) in &q.orders {
        s.push_str(f);
        s.push_str(if *asc { ":1" } else { ":0" });
        s.push(',');
    }
    s.push_str(&format!("l:{}s:{}", q.limit, q.offset));
    if let Some(t) = &q.ttl {
        s.push_str(&format!("t:{:?}:{:?}", t.seconds, t.field));
    }
    format!("{table}::q::{}", sha256_hex(&s))
}

pub struct CachedDatabase {
    inner: Box<dyn Database>,
    cache: Mutex<CacheData>,
}

impl CachedDatabase {
    pub fn new(inner: Box<dyn Database>) -> Self {
        Self {
            inner,
            cache: Mutex::new(CacheData { entries: HashMap::new(), order: VecDeque::new() }),
        }
    }

    fn cache_get(&self, key: &str) -> Option<Cached> {
        let mut c = self.cache.lock().unwrap();
        let e = c.entries.get(key)?;
        if e.expires <= Instant::now() {
            c.entries.remove(key);
            c.order.retain(|k| k != key);
            return None;
        }
        Some(e.value.clone())
    }

    fn cache_put(&self, key: &str, value: Cached, ttl: Duration) {
        let now = Instant::now();
        let mut c = self.cache.lock().unwrap();
        if !c.entries.contains_key(key) {
            c.order.push_back(key.to_string());
        }
        c.entries.insert(
            key.to_string(),
            Entry { expires: now + ttl, value },
        );
        while c.entries.len() > MAX_ENTRIES {
            if let Some(k) = c.order.pop_front() {
                if c.entries.contains_key(&k) {
                    c.entries.remove(&k);
                }
            } else {
                break;
            }
        }
    }

    fn invalidate_table(&self, table: &str) {
        let mut c = self.cache.lock().unwrap();
        let prefix = format!("{table}::");
        c.entries.retain(|k, _| !k.starts_with(&prefix));
        let CacheData { entries, order } = &mut *c;
        order.retain(|k| entries.contains_key(k));
    }
}

impl Database for CachedDatabase {
    fn adapter(&self) -> &'static str {
        "cached"
    }

    fn capabilities(&self) -> DatabaseCaps {
        self.inner.capabilities()
    }

    fn insert(&mut self, table: &str, row: Row) -> anyhow::Result<i64> {
        let seq = self.inner.insert(table, row)?;
        self.invalidate_table(table);
        Ok(seq)
    }

    fn bulk_insert(&mut self, table: &str, rows: Vec<Row>) -> anyhow::Result<Vec<i64>> {
        let seqs = self.inner.bulk_insert(table, rows)?;
        self.invalidate_table(table);
        Ok(seqs)
    }

    fn get(&self, table: &str, pk: &Key) -> anyhow::Result<Option<Row>> {
        let ck = format!("{table}::k::{}", key_repr(pk));
        if let Some(Cached::Row(row)) = self.cache_get(&ck) {
            return Ok(Some(row));
        }
        let row = self.inner.get(table, pk)?;
        if let Some(r) = &row {
            self.cache_put(&ck, Cached::Row(r.clone()), row_ttl(&r.data));
        }
        Ok(row)
    }

    fn update(&mut self, table: &str, pk: &Key, patch: &Json) -> anyhow::Result<()> {
        self.inner.update(table, pk, patch)?;
        self.invalidate_table(table);
        Ok(())
    }

    fn delete(&mut self, table: &str, filter: &SrvFilter) -> anyhow::Result<usize> {
        let n = self.inner.delete(table, filter)?;
        self.invalidate_table(table);
        Ok(n)
    }

    fn query(&self, table: &str, q: &Query) -> anyhow::Result<Cursor> {
        if q.ttl.is_some() {
            return self.inner.query(table, q);
        }
        let ck = query_hash(table, q);
        if let Some(Cached::Cursor(c)) = self.cache_get(&ck) {
            return Ok(c);
        }
        let cursor = self.inner.query(table, q)?;
        self.cache_put(&ck, Cached::Cursor(cursor.clone()), Duration::from_secs(DEFAULT_TTL_SECS));
        Ok(cursor)
    }

    fn upsert(&mut self, table: &str, key: &str, row: Row) -> anyhow::Result<i64> {
        let seq = self.inner.upsert(table, key, row)?;
        self.invalidate_table(table);
        Ok(seq)
    }

    fn link(
        &mut self,
        board: &str,
        from: i64,
        label: &str,
        to: i64,
        props: &Json,
    ) -> anyhow::Result<i64> {
        self.inner.link(board, from, label, to, props)
    }

    fn link_batch(&mut self, board: &str, edges: &[(i64, String, i64)]) -> anyhow::Result<Vec<i64>> {
        self.inner.link_batch(board, edges)
    }

    fn traverse(
        &self,
        board: &str,
        from: i64,
        label: Option<&str>,
        dir: &str,
        depth: usize,
    ) -> anyhow::Result<Cursor> {
        self.inner.traverse(board, from, label, dir, depth)
    }

    fn search_edges(
        &self,
        board: &str,
        label: &str,
        property: &str,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Cursor> {
        self.inner.search_edges(board, label, property, query, limit)
    }

    fn unlink(&mut self, board: &str, edge_id: i64) -> anyhow::Result<bool> {
        self.inner.unlink(board, edge_id)
    }

    fn delete_node(&mut self, board: &str, node_id: i64) -> anyhow::Result<usize> {
        self.inner.delete_node(board, node_id)
    }

    fn begin(&mut self, board: &str) -> anyhow::Result<()> {
        self.inner.begin(board)
    }

    fn commit(&mut self, board: &str) -> anyhow::Result<()> {
        self.inner.commit(board)
    }

    fn rollback(&mut self, board: &str) -> anyhow::Result<()> {
        self.inner.rollback(board)
    }

    fn aggregate(
        &self,
        table: &str,
        board: &str,
        table_name: &str,
        filter: &SrvFilter,
        agg: crate::storage::ir::Agg,
        field: Option<&str>,
        group_by: Option<&str>,
        ttl: Option<TtlClause>,
    ) -> anyhow::Result<Vec<Json>> {
        self.inner.aggregate(table, board, table_name, filter, agg, field, group_by, ttl)
    }

    fn count_records(&self, board: &str) -> anyhow::Result<i64> {
        self.inner.count_records(board)
    }
}

struct BlobEntry {
    expires: Instant,
    bytes: Vec<u8>,
}

struct BlobCache {
    entries: HashMap<String, BlobEntry>,
    order: VecDeque<String>,
}

const DEFAULT_MAX_CACHEABLE_BYTES: u64 = 256 * 1024;

pub struct CachedObjectStore {
    inner: Box<dyn ObjectStore>,
    cache: Mutex<BlobCache>,
    ttl: Duration,
    max: usize,
    max_cacheable_bytes: u64,
}

impl CachedObjectStore {
    pub fn new(inner: Box<dyn ObjectStore>) -> Self {
        Self {
            inner,
            cache: Mutex::new(BlobCache { entries: HashMap::new(), order: VecDeque::new() }),
            ttl: Duration::from_secs(DEFAULT_TTL_SECS),
            max: 512,
            max_cacheable_bytes: DEFAULT_MAX_CACHEABLE_BYTES,
        }
    }

    pub fn with_max_cacheable_bytes(mut self, bytes: u64) -> Self {
        self.max_cacheable_bytes = bytes;
        self
    }

    fn cacheable(&self, size: u64) -> bool {
        size <= self.max_cacheable_bytes
    }

    fn cache_get(&self, key: &str) -> Option<Vec<u8>> {
        let mut c = self.cache.lock().unwrap();
        let e = c.entries.get(key)?;
        if e.expires <= Instant::now() {
            c.entries.remove(key);
            c.order.retain(|k| k != key);
            return None;
        }
        Some(e.bytes.clone())
    }

    fn cache_put(&self, key: &str, bytes: Vec<u8>) {
        let now = Instant::now();
        let mut c = self.cache.lock().unwrap();
        if !c.entries.contains_key(key) {
            c.order.push_back(key.to_string());
        }
        c.entries.insert(key.to_string(), BlobEntry { expires: now + self.ttl, bytes });
        while c.entries.len() > self.max {
            if let Some(k) = c.order.pop_front() {
                if c.entries.contains_key(&k) {
                    c.entries.remove(&k);
                }
            } else {
                break;
            }
        }
    }

    fn invalidate(&self, key: &str) {
        let mut c = self.cache.lock().unwrap();
        if c.entries.remove(key).is_some() {
            c.order.retain(|k| k != key);
        }
    }
}

impl ObjectStore for CachedObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> anyhow::Result<PutInfo> {
        let info = self.inner.put(key, bytes)?;
        if self.cacheable(info.size) {
            self.cache_put(key, bytes.to_vec());
        } else {
            self.invalidate(key);
        }
        Ok(info)
    }

    fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.cache_get(key) {
            return Ok(Some(bytes));
        }
        let bytes = self.inner.get(key)?;
        if let Some(b) = &bytes {
            if self.cacheable(b.len() as u64) {
                self.cache_put(key, b.clone());
            }
        }
        Ok(bytes)
    }

    fn head(&self, key: &str) -> anyhow::Result<Option<BlobMeta>> {
        self.inner.head(key)
    }

    fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.inner.delete(key)?;
        self.invalidate(key);
        Ok(())
    }

    fn list(&self, prefix: &str) -> anyhow::Result<Vec<KeyInfo>> {
        self.inner.list(prefix)
    }

    fn capabilities(&self) -> ObjectStoreCaps {
        self.inner.capabilities()
    }
}