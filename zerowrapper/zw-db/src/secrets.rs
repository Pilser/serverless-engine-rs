use crate::DbPool;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

use std::collections::HashMap;

const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Serialize)]
pub struct SrvSecret {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub created_at: String,
}

pub type SecretMap = HashMap<String, String>;

fn master_key() -> [u8; 32] {
    let gen = match std::env::var("WAOS_SECRET_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => format!("waos-dev-key-{}-{}", std::process::id(), crate::audit::audit_hash_str("x")[..16].to_string()),
    };
    let mut out = [0u8; 32];
    let bytes = gen.as_bytes();
    for (i, b) in out.iter_mut().enumerate() {
        *b = bytes[i % bytes.len().max(1)];
    }
    out
}

fn encrypt(plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(&master_key()).map_err(|e| anyhow::anyhow!("key: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    Ok(base64::engine::general_purpose::STANDARD.encode(&blob))
}

fn decrypt(encoded: &str) -> anyhow::Result<String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
    if blob.len() < NONCE_LEN {
        anyhow::bail!("ciphertext too short");
    }
    let cipher = Aes256Gcm::new_from_slice(&master_key()).map_err(|e| anyhow::anyhow!("key: {e}"))?;
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;
    Ok(String::from_utf8(pt)?)
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn fp(value: &str) -> String {
    crate::audit::audit_hash_str(value)
}

async fn read_secret(rows: &mut turso::Rows) -> anyhow::Result<Option<(String, String, String, String)>> {
    if let Some(row) = rows.next().await? {
        let name = match row.get_value(0)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid name")),
        };
        let cipher = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid cipher")),
        };
        let fingerprint = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid fingerprint")),
        };
        let created_at = match row.get_value(3)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        Ok(Some((name, cipher, fingerprint, created_at)))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub async fn srv_secret_set(&self, board_id: &str, name: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let cipher = encrypt(value)?;
        let b = sq(board_id);
        let n = sq(&name.to_uppercase());
        let c = sq(&cipher);
        let f = sq(&fp(value));
        let sql = format!(
            "INSERT INTO wb_app_secrets (board_id, name, cipher, fingerprint) VALUES ('{b}', '{n}', '{c}', '{f}') \
             ON CONFLICT(board_id, name) DO UPDATE SET cipher = '{c}', fingerprint = '{f}'"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_secret_list(&self, board_id: &str) -> anyhow::Result<Vec<SrvSecret>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT name, cipher, fingerprint, created_at FROM wb_app_secrets WHERE board_id = ?1 ORDER BY name",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some((name, _cipher, fingerprint, created_at)) = read_secret(&mut rows).await? {
            out.push(SrvSecret { name, fingerprint: Some(fingerprint), created_at });
        }
        Ok(out)
    }

    pub async fn srv_secret_remove(&self, board_id: &str, name: &str) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let affected = conn
            .execute(
                "DELETE FROM wb_app_secrets WHERE board_id = ?1 AND name = ?2",
                Params::Positional(vec![Value::from(board_id), Value::from(name.to_uppercase())]),
            )
            .await?;
        Ok(affected > 0)
    }

    /// Load all secrets for a board into a decrypted map, handed to the recipe
    /// engine only (never serialised to clients).
    pub async fn srv_secrets_map(&self, board_id: &str) -> anyhow::Result<SecretMap> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT name, cipher, fingerprint, created_at FROM wb_app_secrets WHERE board_id = ?1 ORDER BY name",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut map = HashMap::new();
        while let Some((name, cipher, _f, _created)) = read_secret(&mut rows).await? {
            if let Ok(pt) = decrypt(&cipher) {
                map.insert(name, pt);
            }
        }
        Ok(map)
    }
}
