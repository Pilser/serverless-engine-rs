use crate::model::{Key, KeyRecord, Principal};
use crate::storage::database::{Database, Query, Row};
use crate::storage::ir::{FilterCond, Op, SrvFilter};
use crate::tables::TABLE_KEYS;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

pub fn role_rank(role: &str) -> u8 {
    match role {
        "customer" => 0,
        "reader" | "list" => 1,
        "writer" => 2,
        "admin" => 3,
        "owner" => 4,
        _ => 0,
    }
}

fn hex(digest: &[u8]) -> String {
    const MAP: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(MAP[(b >> 4) as usize] as char);
        out.push(MAP[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn hash_key(salt: &str, secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(secret.as_bytes());
    hex(&h.finalize())
}

fn board_cond(board_id: &str) -> FilterCond {
    FilterCond {
        field: "$.board_id".to_string(),
        op: Op::Eq,
        value: Json::String(board_id.to_string()),
    }
}

pub fn issue_key(
    db: &mut dyn Database,
    board_id: &str,
    role: &str,
    writer: Option<&str>,
    scope: Option<&str>,
) -> anyhow::Result<(KeyRecord, String)> {
    let role = if role.is_empty() { "writer" } else { role };
    if role == "customer" && scope.is_none() {
        anyhow::bail!("customer keys require a scope (customer id)");
    }
    if role != "customer" && scope.is_some() {
        anyhow::bail!("only customer keys may carry a scope");
    }
    let bucket = uuid::Uuid::new_v4().to_string();
    let salt = uuid::Uuid::new_v4().to_string();
    let key_secret = uuid::Uuid::new_v4().to_string();
    let rec = KeyRecord {
        bucket: bucket.clone(),
        board_id: board_id.to_string(),
        key_hash: hash_key(&salt, &key_secret),
        salt,
        role: role.to_string(),
        writer: writer.map(String::from),
        scope: scope.map(String::from),
        revoked_at: None,
    };
    db.insert(TABLE_KEYS, Row::new(Key::text(&bucket), serde_json::to_value(&rec)?))?;
    Ok((rec, key_secret))
}

pub fn list_keys(db: &dyn Database, board_id: &str) -> anyhow::Result<Vec<KeyRecord>> {
    let q = Query {
        filter: SrvFilter { conds: vec![board_cond(board_id)] },
        orders: vec![],
        limit: usize::MAX,
        offset: 0,
    
        ttl: None,
    };
    let mut out = Vec::new();
    for row in db.query(TABLE_KEYS, &q)?.rows {
        out.push(serde_json::from_value(row.data)?);
    }
    Ok(out)
}

fn find_active_key(db: &dyn Database, board_id: &str, secret: &str) -> anyhow::Result<Option<KeyRecord>> {
    let q = Query {
        filter: SrvFilter {
            conds: vec![
                board_cond(board_id),
                FilterCond { field: "$.revoked_at".to_string(), op: Op::Eq, value: Json::Null },
            ],
        },
        orders: vec![],
        limit: usize::MAX,
        offset: 0,
        ttl: None,
    };
    for row in db.query(TABLE_KEYS, &q)?.rows {
        let kr: KeyRecord = serde_json::from_value(row.data)?;
        if hash_key(&kr.salt, secret) == kr.key_hash {
            return Ok(Some(kr));
        }
    }
    Ok(None)
}

pub fn revoke_key(db: &mut dyn Database, bucket: &str) -> anyhow::Result<()> {
    let filter = SrvFilter {
        conds: vec![FilterCond { field: "$.bucket".to_string(), op: Op::Eq, value: Json::String(bucket.to_string()) }],
    };
    let q = Query { filter: filter.clone(), orders: vec![], limit: 1, offset: 0, ttl: None };
    let Some(row) = db.query(TABLE_KEYS, &q)?.rows.into_iter().next() else {
        anyhow::bail!("key {bucket} not found");
    };
    let mut kr: KeyRecord = serde_json::from_value(row.data)?;
    kr.revoked_at = Some(crate::crud::now_str());
    db.delete(TABLE_KEYS, &filter)?;
    db.insert(TABLE_KEYS, Row::new(Key::text(bucket), serde_json::to_value(&kr)?))?;
    Ok(())
}

pub fn resolve_principal(
    db: &dyn Database,
    board_id: &str,
    token: Option<&str>,
    scope: Option<&str>,
) -> anyhow::Result<Principal> {
    if let Some(t) = token {
        if t == board_id {
            return Ok(Principal {
                id: board_id.to_string(),
                role: "owner".to_string(),
                scope: None,
                writer: None,
            });
        }
        if let Some(kr) = find_active_key(db, board_id, t)? {
            return Ok(Principal {
                id: kr.bucket.clone(),
                role: kr.role.clone(),
                scope: kr.scope.clone().or_else(|| scope.map(String::from)),
                writer: kr.writer.clone(),
            });
        }
    }
    Ok(Principal { id: "anon".to_string(), role: "none".to_string(), scope: None, writer: None })
}

pub fn force_scope(principal: &Principal, payload: &mut serde_json::Value) {
    if let Some(scope) = &principal.scope {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("customer_id".to_string(), Json::String(scope.clone()));
        }
    }
}

pub fn customer_cond(principal: &Principal) -> Option<FilterCond> {
    principal.scope.as_ref().map(|s| FilterCond {
        field: "$.customer_id".to_string(),
        op: Op::Eq,
        value: Json::String(s.clone()),
    })
}

pub fn require_role(principal: &Principal, min: u8) -> anyhow::Result<()> {
    if role_rank(&principal.role) < min {
        anyhow::bail!("forbidden: requires role rank {min} (principal role: {})", principal.role);
    }
    Ok(())
}