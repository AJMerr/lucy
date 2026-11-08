//! Lucy's user configuration (`config.toml`). Format: `docs/config.example.toml`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::{fs, io};

use lucy_protocol::{AgentLevel, Mode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub defaults: Defaults,
    #[serde(default)]
    pub agent: AgentSettings,
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub remote: RemoteSettings,
    #[serde(default)]
    pub sprite: SpriteSettings,
    #[serde(default)]
    pub app: AppSettings,
    #[serde(default)]
    pub user: UserSettings,
    #[serde(default)]
    pub tools: ToolSettings,
    #[serde(default, skip_serializing_if = "McpSettings::is_empty")]
    pub mcp: McpSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Profile used by Assist mode and `lucy ask`.
    pub profile: String,
    /// Profile used by Agent mode; falls back to `profile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(default = "default_persona")]
    pub persona: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub agent_level: AgentLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub model: String,
    /// Keyring entry holding this profile's API key (see ADR 0006).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    /// Offer tools to this model. Turn off for models that can't call them.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub tools: bool,
    /// USD per million input tokens, for agent-mode cost caps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_in: Option<f64>,
    /// USD per million output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_out: Option<f64>,
}

impl Profile {
    /// Prices per million tokens (in, out). Unpriced models on this machine are free;
    /// `None` means unknown, so agent tasks fall back to `[agent] max_tokens`.
    pub fn prices(&self) -> Option<(f64, f64)> {
        match (self.price_in, self.price_out) {
            (Some(i), Some(o)) => Some((i, o)),
            (Some(i), None) => Some((i, i)),
            (None, Some(o)) => Some((o, o)),
            (None, None) if self.is_local() => Some((0.0, 0.0)),
            (None, None) => None,
        }
    }

    fn is_local(&self) -> bool {
        let host = self
            .base_url
            .as_deref()
            .and_then(|base| url::Url::parse(base).ok())
            .and_then(|url| url.host_str().map(str::to_owned));
        matches!(host.as_deref(), Some("localhost" | "127.0.0.1" | "[::1]"))
    }
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Tools (ADR 0004 and 0008).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolSettings {
    /// Lucy's own tools: files, search, fetch and shell.
    pub builtin: bool,
    /// Tool rounds allowed per message in Assist mode before Lucy has to answer.
    pub max_rounds: u32,
    /// Glob patterns Lucy never reads without asking. `~/` means home; `**/` matches anywhere.
    pub private_paths: Vec<String>,
    /// Per-tool overrides of the usual rule, e.g. `run_command = "deny"`.
    pub rules: BTreeMap<String, ToolRule>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            builtin: true,
            max_rounds: 10,
            private_paths: lucy_tools::paths::DEFAULT_PRIVATE_PATHS
                .iter()
                .map(|p| p.to_string())
                .collect(),
            rules: BTreeMap::new(),
        }
    }
}

/// Whether a tool runs by itself, asks first, or never runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolRule {
    Auto,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(rename = "openai_compat")]
    OpenAiCompat,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    pub max_steps: u32,
    pub max_cost_usd: f64,
    /// Token cap per task for models without a price.
    pub max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            max_steps: 25,
            max_cost_usd: 1.0,
            max_tokens: 1_000_000,
            workspace: None,
        }
    }
}

/// Access to the core over WebSocket (`[remote]`), for `lucy remote connect` on other machines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub bind: SocketAddr,
    /// Needed to listen beyond loopback (LAN, Tailscale). There's no TLS, so only on networks
    /// you trust, or behind something that adds it.
    pub allow_network: bool,
    /// Let remote clients change settings, API keys and the persona (reading is always
    /// allowed). Off, a leaked token can chat but not rewrite Lucy.
    pub allow_config: bool,
    /// The highest agent level a chat driven from a remote client runs at.
    pub max_level: AgentLevel,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: SocketAddr::from(([127, 0, 0, 1], 7077)),
            allow_network: false,
            allow_config: false,
            max_level: AgentLevel::Guarded,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserSettings {
    /// What Lucy calls you.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpriteSettings {
    /// Show her when the app starts. Off, the app is just its window and tray icon, and
    /// `lucy sprite` still shows her on demand.
    pub enabled: bool,
    pub pack: String,
    pub scale: f64,
    /// Seconds a finished reply stays up before fading; 0 keeps it until your next message.
    pub bubble_timeout_secs: u32,
    /// Opens her input box where apps may register shortcuts (X11, macOS, Windows); empty
    /// turns it off. On Wayland, bind `lucy sprite ask` in the compositor instead.
    pub hotkey: String,
    /// Older name for `[app] autostart` (v0.4.0), still honoured.
    pub autostart: bool,
}

impl Default for SpriteSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            pack: "placeholder".into(),
            scale: 1.0,
            bubble_timeout_secs: 0,
            hotkey: "Ctrl+Shift+Space".into(),
            autostart: false,
        }
    }
}

/// Refuse to write a config that home-manager put there: a link into the read-only Nix store.
pub fn check_writable(path: &Path) -> Result<(), ConfigError> {
    let managed = fs::read_link(path).is_ok_and(|target| target.starts_with("/nix/store"));
    if managed {
        return Err(ConfigError::Managed {
            path: path.to_owned(),
        });
    }
    Ok(())
}

/// Apply a JSON merge patch (RFC 7396) to config.toml's text: objects merge into tables,
/// `null` removes a key, anything else replaces it. Comments, order and the rest of the file
/// stay as they were. Fails if the result isn't a valid config.
pub fn apply_patch(text: &str, patch: &serde_json::Value) -> Result<String, ConfigError> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| ConfigError::Invalid(format!("{err}")))?;
    let serde_json::Value::Object(changes) = patch else {
        return Err(ConfigError::Invalid(
            "a config change must be an object".into(),
        ));
    };
    merge_into(doc.as_table_mut(), changes)?;
    let text = doc.to_string();
    Config::from_toml_str(&text)?;
    Ok(text)
}

/// Apply a change ([`apply_patch`]) to the config file at `path`, creating it if there's none
/// yet. The file is only written if the result is valid, and never if home-manager owns it.
pub fn update_file(path: &Path, patch: &serde_json::Value) -> Result<(), ConfigError> {
    let io_err = |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    };
    check_writable(path)?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(io_err(err)),
    };
    let updated = apply_patch(&text, patch)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    fs::write(path, updated).map_err(io_err)
}

fn merge_into(
    table: &mut toml_edit::Table,
    changes: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ConfigError> {
    use serde_json::Value as Json;
    use toml_edit::Item;

    for (key, change) in changes {
        match change {
            Json::Null => {
                table.remove(key);
            }
            Json::Object(inner) => match table.get_mut(key) {
                Some(Item::Table(existing)) => merge_into(existing, inner)?,
                Some(Item::Value(toml_edit::Value::InlineTable(existing))) => {
                    let mut merged = existing.clone().into_table();
                    merge_into(&mut merged, inner)?;
                    *existing = merged.into_inline_table();
                }
                _ => {
                    let mut new = toml_edit::Table::new();
                    // A table holding only tables shows as `[a.b]`, not an empty `[a]`.
                    new.set_implicit(inner.values().all(Json::is_object));
                    merge_into(&mut new, inner)?;
                    table.insert(key, Item::Table(new));
                }
            },
            value => {
                let mut new = toml_value(value)?;
                match table.get_mut(key).and_then(Item::as_value_mut) {
                    // Keep the spacing and any comment around the old value.
                    Some(old) => {
                        *new.decor_mut() = old.decor().clone();
                        *old = new;
                    }
                    None => {
                        table.insert(key, Item::Value(new));
                    }
                }
            }
        }
    }
    Ok(())
}

fn toml_value(value: &serde_json::Value) -> Result<toml_edit::Value, ConfigError> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Bool(b) => (*b).into(),
        Json::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n
                .as_f64()
                .ok_or_else(|| ConfigError::Invalid(format!("{n} is out of range")))?
                .into(),
        },
        Json::String(s) => s.as_str().into(),
        Json::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_value(item)?);
            }
            toml_edit::Value::Array(array)
        }
        Json::Object(fields) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, field) in fields {
                inline.insert(key, toml_value(field)?);
            }
            toml_edit::Value::InlineTable(inline)
        }
        Json::Null => return Err(ConfigError::Invalid("null inside a list".into())),
    })
}

/// Lucy's desktop app (its window, tray icon and sprite).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    /// Start the app at login: `lucy service install` adds it to the login items.
    pub autostart: bool,
}

fn default_persona() -> String {
    "lucy".into()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
    #[error("could not serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("could not access {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{} is managed by home-manager (programs.lucy.settings); change it in your Nix \
         config instead",
        .path.display()
    )]
    Managed { path: PathBuf },
    #[error("{}: {source}", .path.display())]
    InFile {
        path: PathBuf,
        #[source]
        source: Box<ConfigError>,
    },
}

/// `$LUCY_CONFIG` if set, else `<platform config dir>/lucy/config.toml`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("LUCY_CONFIG")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|dir| dir.join("lucy").join("config.toml")))
}

impl Config {
    /// Whether the desktop app should start at login (`[app] autostart`, or the older
    /// `[sprite] autostart`).
    pub fn starts_at_login(&self) -> bool {
        self.app.autostart || self.sprite.autostart
    }

    /// Every API key name the config refers to, with the profiles and MCP servers using it.
    pub fn key_names(&self) -> BTreeMap<String, Vec<String>> {
        let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (profile, settings) in &self.profiles {
            if let Some(key_name) = &settings.key_name {
                names
                    .entry(key_name.clone())
                    .or_default()
                    .push(profile.clone());
            }
        }
        for (server, settings) in &self.mcp.servers {
            let keys = settings
                .env_keys
                .values()
                .chain(settings.header_keys.values())
                .chain(&settings.bearer_key);
            for key_name in keys {
                names
                    .entry(key_name.clone())
                    .or_default()
                    .push(server.clone());
            }
        }
        names
    }

    /// A config with one profile, used as the default, and everything else at its defaults.
    pub fn with_profile(name: impl Into<String>, profile: Profile) -> Self {
        let name = name.into();
        Self {
            defaults: Defaults {
                profile: name.clone(),
                agent_profile: None,
                persona: default_persona(),
                mode: Mode::default(),
                agent_level: AgentLevel::default(),
            },
            agent: AgentSettings::default(),
            profiles: BTreeMap::from([(name, profile)]),
            remote: RemoteSettings::default(),
            sprite: SpriteSettings::default(),
            app: AppSettings::default(),
            user: UserSettings::default(),
            tools: ToolSettings::default(),
            mcp: McpSettings::default(),
        }
    }

    /// Read the config at `path`. Returns `Ok(None)` if the file doesn't exist yet.
    pub fn load(path: &Path) -> Result<Option<Self>, ConfigError> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        Self::from_toml_str(&source)
            .map(Some)
            .map_err(|source| ConfigError::InFile {
                path: path.to_owned(),
                source: Box::new(source),
            })
    }

    /// Write the config to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io_err = |source| ConfigError::Io {
            path: path.to_owned(),
            source,
        };
        check_writable(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        fs::write(path, toml::to_string_pretty(self)?).map_err(io_err)
    }

    pub fn from_toml_str(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let references = [
            ("defaults.profile", Some(&self.defaults.profile)),
            (
                "defaults.agent_profile",
                self.defaults.agent_profile.as_ref(),
            ),
        ];
        for (field, name) in references {
            if let Some(name) = name
                && !self.profiles.contains_key(name)
            {
                return Err(ConfigError::Invalid(format!(
                    "{field} refers to unknown profile `{name}`"
                )));
            }
        }

        for (name, profile) in &self.profiles {
            if profile.provider == ProviderKind::OpenAiCompat && profile.base_url.is_none() {
                return Err(ConfigError::Invalid(format!(
                    "profile `{name}` uses openai_compat and needs a base_url"
                )));
            }
        }
        for (name, server) in &self.mcp.servers {
            validate_server_name(name).map_err(ConfigError::Invalid)?;
            let command = server
                .command
                .as_deref()
                .is_some_and(|c| !c.trim().is_empty());
            let url = server
                .url
                .as_deref()
                .is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"));
            if command == url {
                return Err(ConfigError::Invalid(format!(
                    "MCP server `{name}` needs either a command or an http(s) url, not both"
                )));
            }
        }
        Ok(())
    }
}

/// External MCP servers (ADR 0008), started on first use and kept running.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    pub servers: BTreeMap<String, McpServer>,
}

impl McpSettings {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

/// A server Lucy either starts as a program (`command`, talking over stdin/stdout) or
/// reaches at a URL (`url`, streamable HTTP). Exactly one of the two is set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    /// The program, e.g. `npx`. Looked up on `PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Plain environment variables (nothing secret).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Environment variables filled from keyring entries: `GITHUB_TOKEN = "github"` uses the
    /// key stored with `lucy auth set github` (or `LUCY_KEY_GITHUB`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_keys: BTreeMap<String, String>,
    /// Folder to start in; defaults to home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// A remote server's address, e.g. `https://mcp.linear.app/mcp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Keyring entry sent as `Authorization: Bearer <key>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_key: Option<String>,
    /// Headers filled from keyring entries, e.g. `{ X-Api-Key = "acme" }`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub header_keys: BTreeMap<String, String>,
    /// Plain headers (nothing secret).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// For sign-in with an app you registered yourself, instead of automatic registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    /// Fixed port for the sign-in redirect (`http://127.0.0.1:<port>/callback`), when your
    /// registered app needs one. Default: any free port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_redirect_port: Option<u16>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl McpServer {
    /// A server started as `command args…`, enabled.
    pub fn program(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: Some(command.into()),
            args,
            enabled: true,
            ..Self::default()
        }
    }

    /// A remote server at `url`, enabled.
    pub fn remote(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            enabled: true,
            ..Self::default()
        }
    }

    /// Where it runs, for listings: the command line or the URL.
    pub fn describe(&self) -> String {
        match (&self.command, &self.url) {
            (Some(command), _) => std::iter::once(command)
                .chain(&self.args)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            (None, Some(url)) => url.clone(),
            (None, None) => "(no command or url)".into(),
        }
    }
}

/// Server names become tool-name prefixes (`github__create_issue`), so keep them simple.
pub fn validate_server_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !name.contains("__")
        && name != crate::tools::BUILTIN;
    if valid {
        Ok(())
    } else {
        Err(format!(
            "`{name}` isn't a valid MCP server name: use up to 32 lowercase letters, digits, `-` \
             or `_` (no `__`, and not `builtin`)"
        ))
    }
}
