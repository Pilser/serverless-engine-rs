use crate::context::EventContext;
use crate::rule::EventKind;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const PREFIX: &str = "secret:";

fn hex(d: &[u8]) -> String {
    const MAP: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(d.len() * 2);
    for b in d {
        out.push(MAP[(b >> 4) as usize] as char);
        out.push(MAP[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn fingerprint(secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    hex(&h.finalize())
}

#[derive(Debug, Clone, Default)]
pub struct Secrets {
    map: HashMap<String, String>,
}

impl Secrets {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    pub fn insert(&mut self, name: &str, value: &str) {
        self.map.insert(name.to_uppercase(), value.to_string());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.map.get(&name.to_uppercase()).map(|s| s.as_str())
    }

    pub fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = self.map.keys().cloned().collect();
        n.sort();
        n
    }

    /// Mask for read responses: names only, never values.
    pub fn masked(&self) -> Value {
        let mut obj = serde_json::Map::new();
        for name in self.names() {
            obj.insert(name, Value::String(PREFIX.to_string()));
        }
        Value::Object(obj)
    }

    /// Substitute `{NAME}` / `${NAME}` / `{{NAME}}` in a string with the secret value.
    /// Unknown names are left verbatim. Longest pattern wins so `${X}`/`{{X}}` are
    /// not half-consumed by the bare `{X}` replacement.
    pub fn substitute(&self, input: &str) -> String {
        let mut out = input.to_string();
        for name in self.names().iter() {
            if let Some(val) = self.get(name) {
                for pat in [format!("{{{{{name}}}}}"), format!("${{{name}}}"), format!("{{{name}}}")] {
                    out = out.replace(&pat, val);
                }
            }
        }
        out
    }
}

/// Recursively substitute secrets in a JSON value (strings only, keys untouched).
pub fn substitute_value(secrets: &Secrets, v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(secrets.substitute(s)),
        Value::Array(arr) => Value::Array(arr.iter().map(|x| substitute_value(secrets, x)).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), substitute_value(secrets, val));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn event_scalar(ctx: &EventContext, name: &str) -> Option<Value> {
    match name {
        "seq" => ctx.seq.map(Value::from),
        "kind" => Some(Value::String(match ctx.kind {
            EventKind::Created => "created",
            EventKind::Updated => "updated",
            EventKind::Deleted => "deleted",
            EventKind::Inbound => "inbound",
            EventKind::Cron => "cron",
        }.to_string())),
        "board" => Some(Value::String(ctx.board_id.clone())),
        "writer" => ctx.writer.clone().map(Value::String),
        "cron" => ctx.cron_name.clone().map(Value::String),
        _ => None,
    }
}

fn resolve_event_token(secrets: &Secrets, ctx: &EventContext, token: &str) -> anyhow::Result<Value> {
    let t = token.trim();
    if let Some(name) = t.strip_prefix("secrets.") {
        let v = secrets
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown secret '{name}' in template"))?;
        return Ok(Value::String(v.to_string()));
    }
    if let Some(path) = t.strip_prefix("payload.") {
        let v = zw_expr::get_path(&ctx.payload, &format!("$.{path}"));
        if v.is_null() {
            anyhow::bail!("unknown payload field '{{{{payload.{path}}}}}' in template");
        }
        return Ok(v);
    }
    if let Some(v) = event_scalar(ctx, t) {
        return Ok(v);
    }
    if let Some(v) = secrets.get(t) {
        return Ok(Value::String(v.to_string()));
    }
    if t.starts_with("secrets.") {
        anyhow::bail!("unknown secret '{}' in template", &t["secrets.".len()..]);
    }
    anyhow::bail!("unknown template key '{{{t}}}'")
}

/// Render a single template string against secrets + event context. When the
/// whole string is one template token whose value is not a string, the raw JSON
/// value is returned so numeric payload fields stay numeric.
fn render_event_string(secrets: &Secrets, ctx: &EventContext, input: &str) -> anyhow::Result<Value> {
    if let Some(inner) = input.trim().strip_prefix("{{").and_then(|s| s.strip_suffix("}}")) {
        if !inner.contains("{{") && !inner.contains("}}") {
            let v = resolve_event_token(secrets, ctx, inner)?;
            if !v.is_string() {
                return Ok(v);
            }
        }
    }
    let input = secrets.substitute(input);
    let mut out = String::new();
    let mut rest = input.as_str();
    while let Some(start) = rest.find("{{") {
        let Some(end_rel) = rest[start..].find("}}") else {
            out.push_str(rest);
            break;
        };
        let end = start + end_rel + 2;
        out.push_str(&rest[..start]);
        let token = &rest[start + 2..start + end_rel];
        let v = resolve_event_token(secrets, ctx, token)?;
        out.push_str(&scalar_string(&v));
        rest = &rest[end..];
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

fn scalar_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Recursively substitute secrets + event context (`{{payload.x}}`,
/// `{{seq}}`, `{{board}}`, `{{writer}}`, `{{cron}}`, `{{kind}}`,
/// `{{secrets.NAME}}`) in a JSON value. Unknown template keys are an error.
pub fn substitute_event(secrets: &Secrets, ctx: &EventContext, v: &Value) -> anyhow::Result<Value> {
    match v {
        Value::String(s) => render_event_string(secrets, ctx, s),
        Value::Array(arr) => arr
            .iter()
            .map(|x| substitute_event(secrets, ctx, x))
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), substitute_event(secrets, ctx, val)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_all_syntaxes() {
        let mut s = Secrets::new();
        s.insert("STRIPE", "sk_test_123");
        let out = s.substitute("Bearer ${STRIPE} and {STRIPE} and {{STRIPE}}");
        assert_eq!(out, "Bearer sk_test_123 and sk_test_123 and sk_test_123");
    }

    #[test]
    fn masked_never_leaks() {
        let mut s = Secrets::new();
        s.insert("TOKEN", "super-secret");
        assert!(!serde_json::to_string(&s.masked()).unwrap().contains("super-secret"));
        assert!(s.masked().as_object().unwrap().contains_key("TOKEN"));
    }

    #[test]
    fn case_insensitive_lookup() {
        let mut s = Secrets::new();
        s.insert("TOKEN", "abc");
        assert_eq!(s.get("token"), Some("abc"));
    }
}
