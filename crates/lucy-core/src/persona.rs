//! Personas: the system-prompt files that give Lucy (or anyone else) a voice.
//!
//! Built-in personas are compiled from the repo's `persona/` folder. Files in the user's
//! persona folder (`<config dir>/lucy/personas/<name>.md`) override or add to them.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};

use crate::sessions::Origin;

const BUILTIN: &[(&str, &str)] = &[("lucy", include_str!("../../../persona/lucy.md"))];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaSource {
    Builtin,
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Persona {
    pub name: String,
    pub text: String,
    pub source: PersonaSource,
}

#[derive(Debug, thiserror::Error)]
pub enum PersonaError {
    #[error("invalid persona name `{0}`: use lowercase letters, digits, `-` and `_`")]
    InvalidName(String),
    #[error("no persona named `{name}` (available: {})", .available.join(", "))]
    NotFound {
        name: String,
        available: Vec<String>,
    },
    #[error("could not read {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub struct Personas {
    user_dir: Option<PathBuf>,
}

impl Personas {
    pub fn new(user_dir: Option<PathBuf>) -> Self {
        Self { user_dir }
    }

    pub fn load(&self, name: &str) -> Result<Persona, PersonaError> {
        if !valid_name(name) {
            return Err(PersonaError::InvalidName(name.to_owned()));
        }
        if let Some(dir) = &self.user_dir {
            let path = dir.join(format!("{name}.md"));
            match fs::read_to_string(&path) {
                Ok(text) => {
                    return Ok(Persona {
                        name: name.to_owned(),
                        text,
                        source: PersonaSource::File(path),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(PersonaError::Io { path, source }),
            }
        }
        BUILTIN
            .iter()
            .find(|(builtin, _)| *builtin == name)
            .map(|(_, text)| Persona {
                name: name.to_owned(),
                text: (*text).to_owned(),
                source: PersonaSource::Builtin,
            })
            .ok_or_else(|| PersonaError::NotFound {
                name: name.to_owned(),
                available: self.available(),
            })
    }

    /// Names of every loadable persona, sorted.
    pub fn available(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = BUILTIN.iter().map(|(n, _)| (*n).to_owned()).collect();
        let entries = self
            .user_dir
            .as_ref()
            .and_then(|dir| fs::read_dir(dir).ok());
        for entry in entries.into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && valid_name(stem)
            {
                names.insert(stem.to_owned());
            }
        }
        names.into_iter().collect()
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Everything besides the persona that goes into a system prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptContext {
    /// Drop `<!-- voice -->` blocks and keep `<!-- plain -->` blocks.
    pub plain: bool,
    pub now: DateTime<FixedOffset>,
    /// e.g. "NixOS 26.11 (Yarara), Hyprland, x86_64".
    pub machine: Option<String>,
    pub user_name: Option<String>,
    /// The profile and model answering, so Lucy can say what she runs on.
    pub origin: Option<Origin>,
    /// The folder the user is working in; tools resolve relative paths against it.
    pub cwd: Option<String>,
}

/// The persona text for this mode, followed by a context section. Context goes last so the
/// persona prefix stays identical between requests (prompt caching).
pub fn build_system_prompt(persona: &Persona, ctx: &PromptContext) -> String {
    let mut lines = vec![
        "## Context".to_owned(),
        String::new(),
        format!(
            "- Current date and time: {}",
            ctx.now.format("%A %-d %B %Y, %H:%M (UTC%:z)")
        ),
    ];
    if let Some(machine) = &ctx.machine {
        lines.push(format!("- Machine: {machine}"));
    }
    if let Some(name) = &ctx.user_name {
        lines.push(format!("- The user's name is {name}."));
    }
    if let Some(origin) = &ctx.origin {
        lines.push(format!(
            "- You are running as profile `{}` (model `{}`).",
            origin.profile, origin.model
        ));
    }
    if let Some(cwd) = &ctx.cwd {
        lines.push(format!("- Working folder: {cwd}"));
    }
    format!(
        "{}\n\n{}",
        persona_for_mode(&persona.text, ctx.plain),
        lines.join("\n")
    )
}

/// Strip the blocks that don't apply to this mode, drop the markers, tidy blank lines.
fn persona_for_mode(text: &str, plain: bool) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        match line.trim() {
            VOICE_START => skipping = plain,
            PLAIN_START => skipping = !plain,
            VOICE_END | PLAIN_END => skipping = false,
            _ if skipping => {}
            // Other full-line comments are notes for persona authors, not the model.
            trimmed if trimmed.starts_with("<!--") && trimmed.ends_with("-->") => {}
            _ => {
                let blank = line.trim().is_empty();
                if !(blank && kept.last().is_none_or(|prev| prev.trim().is_empty())) {
                    kept.push(line);
                }
            }
        }
    }
    kept.join("\n").trim().to_owned()
}

const VOICE_START: &str = "<!-- voice -->";
const VOICE_END: &str = "<!-- /voice -->";
const PLAIN_START: &str = "<!-- plain -->";
const PLAIN_END: &str = "<!-- /plain -->";

/// The user's persona folder: `personas/` next to `config.toml`.
pub fn default_user_dir() -> Option<PathBuf> {
    crate::config::default_path()?
        .parent()
        .map(|dir| dir.join("personas"))
}

/// Best-effort description of this machine, e.g. "NixOS 26.11 (Yarara), Hyprland, x86_64".
pub fn detect_machine() -> String {
    let mut parts = vec![os_name()];
    if cfg!(target_os = "linux")
        && let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP")
        && !desktop.is_empty()
    {
        parts.push(desktop);
    }
    parts.push(std::env::consts::ARCH.to_owned());
    parts.join(", ")
}

fn os_name() -> String {
    if cfg!(target_os = "linux")
        && let Ok(release) = fs::read_to_string("/etc/os-release")
        && let Some(name) = release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
    {
        return name.trim_matches('"').to_owned();
    }
    match std::env::consts::OS {
        "macos" => "macOS".to_owned(),
        "windows" => "Windows".to_owned(),
        other => other.to_owned(),
    }
}
