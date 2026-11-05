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
    pub user: UserSettings,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            max_steps: 25,
            max_cost_usd: 1.0,
            workspace: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub bind: SocketAddr,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: SocketAddr::from(([127, 0, 0, 1], 7077)),
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
    pub pack: String,
    pub scale: f64,
    pub bubble_timeout_secs: u32,
}

impl Default for SpriteSettings {
    fn default() -> Self {
        Self {
            pack: "placeholder".into(),
            scale: 1.0,
            bubble_timeout_secs: 12,
        }
    }
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
            user: UserSettings::default(),
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
        Ok(())
    }
}
