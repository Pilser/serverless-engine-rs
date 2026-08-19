use std::collections::HashMap;
use std::path::PathBuf;
use serde::Deserialize;

/// Top-level config — mirrors `[zerowrapper]` in config.toml
#[derive(Debug, Clone, Deserialize)]
pub struct WrapperConfig {
    /// Unix socket path for zeroclaw daemon, e.g. /tmp/.zeroclaw/daemon.sock
    pub zeroclaw_socket: String,
    /// Path to local turso/SQLite database file, e.g. ./data/zerowrapper.db
    pub db_path: String,
    /// Directory for content-addressed cache
    pub cache_dir: PathBuf,
    /// Web channel — each channel specifies its own agent
    pub ch_web: WebChannelConfig,
    /// How zeroclaw should discover/spawn this binary as an MCP tool server
    #[serde(default)]
    pub mcp: Option<McpDiscoveryConfig>,
}

/// Config for the web channel — section `[zerowrapper.ch_web]`
#[derive(Debug, Clone, Deserialize)]
pub struct WebChannelConfig {
    /// Which zeroclaw agent this channel sends prompts to, e.g. "craw"
    pub agent: String,
    /// HTTP bind address, e.g. "0.0.0.0:9090"
    pub bind: String,
    /// CORS allowed origins
    #[serde(default = "default_origins")]
    pub allowed_origins: Vec<String>,
    /// Topcoat UI bind address, e.g. "0.0.0.0:9091". Defaults to bind port + 1.
    #[serde(default)]
    pub ui_bind: Option<String>,
    /// Pre-configured admin/owner accounts: phone -> {role, password}
    #[serde(default)]
    pub accounts: HashMap<String, AccountConfig>,
    /// Directory for file uploads
    #[serde(default = "default_upload_dir")]
    pub upload_dir: PathBuf,
    /// Directory for session whiteboard files (defaults to {AGENTS_DIR}/{agent}/workspace)
    #[serde(default)]
    pub whiteboard_dir: Option<PathBuf>,
}

impl WebChannelConfig {
    pub fn ui_bind(&self) -> String {
        if let Some(ref b) = self.ui_bind {
            return b.clone();
        }
        if let Some((host, port)) = self.bind.rsplit_once(':') {
            let port: u16 = port.parse().unwrap_or(9090);
            format!("{}:{}", host, port + 1)
        } else {
            "127.0.0.1:9091".to_string()
        }
    }
}

fn default_upload_dir() -> PathBuf {
    PathBuf::from("./data/uploads")
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    pub role: String,
    pub password: String,
}

fn default_origins() -> Vec<String> {
    vec!["*".to_string()]
}

/// Optional MCP discovery — tells zeroclaw how to spawn this binary.
/// If absent, a guidance log is printed so the user can configure it.
#[derive(Debug, Clone, Deserialize)]
pub struct McpDiscoveryConfig {
    pub server_name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

impl Default for WrapperConfig {
    fn default() -> Self {
        Self {
            zeroclaw_socket: String::new(),
            db_path: String::new(),
            cache_dir: PathBuf::new(),
            ch_web: WebChannelConfig {
                agent: String::new(),
                bind: String::new(),
                allowed_origins: vec![],
                ui_bind: None,
                accounts: HashMap::new(),
                upload_dir: default_upload_dir(),
                whiteboard_dir: None,
            },
            mcp: None,
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    zerowrapper: WrapperConfig,
}

impl WrapperConfig {
    /// Load config from config.toml, then overlay .env file, then OS env vars.
    /// config.toml is optional — .env or OS env vars alone are sufficient.
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Self::load_toml("config.toml")
            .unwrap_or_else(|| {
                tracing::info!("config.toml not found, using .env only");
                WrapperConfig::default()
            });

        if let Some(env_overlay) = Self::load_env()? {
            config = Self::merge_configs(config, env_overlay);
        }

        if let Some(os_overlay) = Self::load_os_env()? {
            config = Self::merge_configs(config, os_overlay);
        }

        let missing: Vec<&str> = {
            let mut m = Vec::new();
            if config.zeroclaw_socket.is_empty() { m.push("ZEROWRAPPER__ZEROCLAW_SOCKET"); }
            if config.db_path.is_empty() { m.push("ZEROWRAPPER__DB_PATH"); }
            if config.cache_dir.as_os_str().is_empty() { m.push("ZEROWRAPPER__CACHE_DIR"); }
            if config.ch_web.agent.is_empty() { m.push("ZEROWRAPPER_CH_WEB__AGENT"); }
            if config.ch_web.bind.is_empty() { m.push("ZEROWRAPPER_CH_WEB__BIND"); }
            m
        };
        if !missing.is_empty() {
            anyhow::bail!(
                "Missing required config. Add to .env:\n  {}",
                missing.join("\n  ")
            );
        }

        Ok(config)
    }

    /// Load config.toml. Returns None if file doesn't exist.
    fn load_toml(path: &str) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let file: ConfigFile = toml::from_str(&content).ok()?;
        Some(file.zerowrapper)
    }

    /// Parse .env into a loose toml::Value so partial overrides work.
    fn load_env() -> anyhow::Result<Option<toml::Value>> {
        let content = match std::fs::read_to_string(".env") {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };

        let toml_str = env_lines_to_toml(&content);
        if toml_str.trim().is_empty() {
            return Ok(None);
        }

        let value: toml::Value = toml::from_str(&toml_str)?;
        Ok(Some(value))
    }

    /// Read ZEROWRAPPER_* vars from actual OS environment (Docker -e / export).
    fn load_os_env() -> anyhow::Result<Option<toml::Value>> {
        let pairs: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("ZEROWRAPPER_"))
            .collect();
        if pairs.is_empty() {
            return Ok(None);
        }
        let mut content = String::new();
        for (k, v) in &pairs {
            content.push_str(&format!("{}={}\n", k, v));
        }
        let toml_str = env_lines_to_toml(&content);
        if toml_str.trim().is_empty() {
            return Ok(None);
        }
        let value: toml::Value = toml::from_str(&toml_str)?;
        Ok(Some(value))
    }

    /// Merge a partial env overlay into the base config, field by field.
    fn merge_configs(mut base: Self, overlay: toml::Value) -> Self {
        let Some(ow) = overlay.get("zerowrapper") else { return base };
        let Some(table) = ow.as_table() else { return base };

        for (key, val) in table {
            match (key.as_str(), val) {
                ("zeroclaw_socket", toml::Value::String(v)) => base.zeroclaw_socket = v.clone(),
                ("db_path", toml::Value::String(v)) => base.db_path = v.clone(),
                ("cache_dir", toml::Value::String(v)) => base.cache_dir = PathBuf::from(v),
                ("ch_web", toml::Value::Table(web)) => {
                    for (wk, wv) in web {
                        match (wk.as_str(), wv) {
                            ("agent", toml::Value::String(v)) => base.ch_web.agent = v.clone(),
                            ("bind", toml::Value::String(v)) => base.ch_web.bind = v.clone(),
                            ("ui_bind", toml::Value::String(v)) => base.ch_web.ui_bind = Some(v.clone()),
                            ("allowed_origins", toml::Value::Array(arr)) => {
                                let origins: Vec<String> = arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect();
                                if !origins.is_empty() {
                                    base.ch_web.allowed_origins = origins;
                                }
                            }
                            ("upload_dir", toml::Value::String(v)) => {
                                base.ch_web.upload_dir = PathBuf::from(v);
                            }
                            ("whiteboard_dir", toml::Value::String(v)) => {
                                base.ch_web.whiteboard_dir = Some(PathBuf::from(v));
                            }
                            ("accounts", val) => {
                                if let Ok(accts) = val.clone().try_into::<HashMap<String, AccountConfig>>() {
                                    base.ch_web.accounts = accts;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ("mcp", toml::Value::Table(_)) => {
                    if let Ok(mcp) = val.clone().try_into::<McpDiscoveryConfig>() {
                        base.mcp = Some(mcp);
                    }
                }
                _ => {}
            }
        }

        base
    }
}

/// Convert env lines to nested TOML matching the ConfigFile struct.
///
/// Format:
///   ZEROWRAPPER__{KEY}=value            →  [zerowrapper] key = value
///   ZEROWRAPPER_{SECTION}__{KEY}=value  →  [zerowrapper.{section}] key = value
fn env_lines_to_toml(content: &str) -> String {
    use std::collections::BTreeMap;

    let mut sections: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let keystr = key.trim();
            let valstr = value.trim().trim_matches('"').trim_matches('\'');
            if let Some(rest) = keystr.strip_prefix("ZEROWRAPPER_") {
                if rest.starts_with('_') {
                    let field = rest[1..].to_lowercase();
                    sections
                        .entry(String::new())
                        .or_default()
                        .push((field, valstr.to_string()));
                } else if let Some(pos) = rest.rfind("__") {
                    let subsection = rest[..pos].to_lowercase();
                    let field = rest[pos + 2..].to_lowercase();
                    sections
                        .entry(subsection)
                        .or_default()
                        .push((field, valstr.to_string()));
                }
            }
        }
    }

    let mut toml = String::new();
    for (section, fields) in &sections {
        if section.is_empty() {
            toml.push_str("[zerowrapper]\n");
        } else {
            toml.push_str(&format!("[zerowrapper.{}]\n", section));
        }
        for (field, value) in fields {
            toml.push_str(&format!("{} = {}\n", field, escape_toml_value(value)));
        }
    }
    toml
}

fn escape_toml_value(value: &str) -> String {
    if value.parse::<f64>().is_ok()
        || value == "true"
        || value == "false"
        || (value.starts_with('[') && value.ends_with(']'))
        || (value.starts_with('{') && value.ends_with('}'))
    {
        return value.to_string();
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}
