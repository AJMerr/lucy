//! `lucy remote`: the token remote clients must present to this machine's core, and the
//! remote core this machine's `lucy` talks to instead of its own.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use dialoguer::Password;
use lucy_client::{Client, Endpoint};
use lucy_core::config::{self, Config};
use lucy_core::secrets::{self, KeySource, OsKeyring, SecretStore, Secrets, SecretsError};
use lucy_core::server::REMOTE_TOKEN_KEY;

/// The keyring entry for the token this machine sends to a remote core.
pub const CLIENT_TOKEN_KEY: &str = "remote-client";
/// The port `[remote] bind` uses by default.
const DEFAULT_PORT: u16 = 7077;

/// The core's token: `created` when this call made it (first use, or `rotate`).
pub struct Token {
    pub value: String,
    pub created: bool,
}

/// The token remote clients must present, created on first use. `rotate` replaces it.
pub fn token(store: &dyn SecretStore, rotate: bool) -> Result<Token> {
    if !rotate && let Some(value) = store.get(REMOTE_TOKEN_KEY)? {
        return Ok(Token {
            value,
            created: false,
        });
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).context("couldn't get random bytes for a token")?;
    let value: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    store.set(REMOTE_TOKEN_KEY, &value)?;
    Ok(Token {
        value,
        created: true,
    })
}

/// `desktop`, `desktop:8000` or `ws://desktop:7077` as a full WebSocket URL
/// (`ws://desktop:7077/rpc`). `wss://` is kept for cores behind something that adds TLS.
pub fn normalize_url(input: &str) -> Result<String> {
    let text = input.trim();
    let (scheme, rest) = text.split_once("://").unwrap_or(("ws", text));
    if !matches!(scheme, "ws" | "wss") {
        bail!(
            "`{input}` isn't a WebSocket address; use ws://host:{DEFAULT_PORT} (or wss:// when \
             something in front adds TLS)"
        );
    }
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) if !path.is_empty() => (authority, format!("/{path}")),
        Some((authority, _)) => (authority, "/rpc".to_owned()),
        None => (rest, "/rpc".to_owned()),
    };
    if authority.is_empty() {
        bail!("`{input}` has no host; use ws://host:{DEFAULT_PORT}");
    }
    let has_port = authority.rsplit_once(':').is_some_and(|(host, port)| {
        !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && (!host.contains(':') || host.ends_with(']'))
    });
    let authority = match (has_port, scheme) {
        (false, "ws") => format!("{authority}:{DEFAULT_PORT}"),
        _ => authority.to_owned(),
    };
    Ok(format!("{scheme}://{authority}{path}"))
}

/// Remember `url` as the core to use (in `remote.toml`, keeping its comments).
pub fn save_connection(file: &Path, url: &str) -> Result<()> {
    let mut doc = match std::fs::read_to_string(file) {
        Ok(text) => text
            .parse::<toml_edit::DocumentMut>()
            .with_context(|| format!("couldn't read {}", file.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            "# The Lucy core this machine talks to; `lucy remote disconnect` removes this file.\n"
                .parse()
                .expect("a comment is valid TOML")
        }
        Err(err) => return Err(err).with_context(|| format!("couldn't read {}", file.display())),
    };
    doc["url"] = toml_edit::value(url);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    std::fs::write(file, doc.to_string())
        .with_context(|| format!("couldn't write {}", file.display()))
}

/// The remembered remote core, if any.
pub fn saved_connection(file: &Path) -> Result<Option<String>> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("couldn't read {}", file.display())),
    };
    let doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("couldn't read {}", file.display()))?;
    Ok(doc
        .get("url")
        .and_then(|url| url.as_str())
        .map(str::to_owned))
}

/// Forget the remote core. False if none was remembered.
pub fn forget_connection(file: &Path) -> Result<bool> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("couldn't remove {}", file.display())),
    }
}

/// `remote.toml`, next to the config file.
pub fn connection_file() -> Result<PathBuf> {
    let config = config::default_path().context("no config directory; set LUCY_CONFIG")?;
    Ok(config.parent().map_or_else(
        || PathBuf::from("remote.toml"),
        |dir| dir.join("remote.toml"),
    ))
}

/// `--remote <url>` for this run, set once from the command line.
static OVERRIDE: OnceLock<String> = OnceLock::new();

pub fn set_override(url: String) {
    let _ = OVERRIDE.set(url);
}

/// Whether this `lucy` talks to a remote core (so it needs no config of its own).
pub fn selected() -> Result<bool> {
    Ok(OVERRIDE.get().is_some()
        || std::env::var("LUCY_REMOTE").is_ok_and(|url| !url.is_empty())
        || saved_connection(&connection_file()?)?.is_some())
}

/// The remote core this `lucy` talks to, if any: `--remote`, then `LUCY_REMOTE`, then the one
/// saved by `lucy remote connect`. Its token comes from the keyring (`LUCY_KEY_REMOTE_CLIENT`
/// overrides it).
pub fn target() -> Result<Option<Endpoint>> {
    let given = OVERRIDE.get().cloned().or_else(|| {
        std::env::var("LUCY_REMOTE")
            .ok()
            .filter(|url| !url.is_empty())
    });
    let url = match given {
        Some(url) => normalize_url(&url)?,
        None => match saved_connection(&connection_file()?)? {
            Some(url) => url,
            None => return Ok(None),
        },
    };
    let missing = format!(
        "no token for the Lucy core at {url}; run `lucy remote connect {url}` (or set {})",
        secrets::env_var_name(CLIENT_TOKEN_KEY)
    );
    let token = match Secrets::system().resolve(CLIENT_TOKEN_KEY) {
        Ok(Some(key)) => key.value,
        Ok(None) => bail!(missing),
        Err(err) => bail!("{missing} ({err})"),
    };
    Ok(Some(Endpoint::Remote { url, token }))
}

/// `lucy remote token [--rotate]`: print the token the core checks (on stdout, alone) and
/// what to do with it. Rotating goes through the running core, which disconnects every remote
/// client; with no core running, the keyring's token is replaced.
pub fn token_command(rotate: bool) -> Result<()> {
    let keyring_hint = || {
        format!(
            "couldn't keep the token in the keyring; set {} to a long random value instead",
            secrets::env_var_name(REMOTE_TOKEN_KEY)
        )
    };
    // The environment wins over the keyring, for the core as here.
    let from_env = Secrets::system()
        .resolve(REMOTE_TOKEN_KEY)
        .ok()
        .flatten()
        .and_then(|key| match key.source {
            KeySource::Env(var) => Some((var, key.value)),
            KeySource::Keyring => None,
        });
    let (token, disconnected) = match (&from_env, rotate) {
        (Some((var, _)), true) => bail!(
            "the token comes from {var}, which the core uses over the keyring; change {var}              (or unset it), then restart the core"
        ),
        (Some((_, value)), false) => (
            Token {
                value: value.clone(),
                created: false,
            },
            false,
        ),
        (None, true) => match rotate_on_core()? {
            Some(value) => (
                Token {
                    value,
                    created: true,
                },
                true,
            ),
            None => (token(&OsKeyring, true).with_context(keyring_hint)?, false),
        },
        (None, false) => (token(&OsKeyring, false).with_context(keyring_hint)?, false),
    };
    println!("{}", token.value);
    match (token.created, rotate, disconnected) {
        (true, true, true) => eprintln!(
            "That's the new token. Remote clients were disconnected and need it to connect again."
        ),
        (true, true, false) => eprintln!("That's the new token; new connections need it."),
        (true, false, _) => eprintln!("Created a token for remote clients."),
        (false, ..) => {}
    }
    eprintln!(
        "On the other machine: `lucy remote connect <this machine's address>`, and paste it."
    );
    let config_path = config::default_path().context("no config directory; set LUCY_CONFIG")?;
    let enabled = Config::load(&config_path)
        .ok()
        .flatten()
        .is_some_and(|config| config.remote.enabled);
    if !enabled {
        eprintln!(
            "Remote access is off: set `enabled = true` under [remote] in {}, then restart the \
             core (`lucy service restart`, or `lucy core stop`).",
            config_path.display()
        );
    }
    Ok(())
}

/// Ask this machine's core, if it's running, for a new token (`remote.rotate`).
fn rotate_on_core() -> Result<Option<String>> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let endpoint = Endpoint::default_for_user();
        if crate::core_process::status(&endpoint).await?.is_none() {
            return Ok(None);
        }
        let client = Client::connect(&endpoint, crate::core_process::CLIENT_NAME).await?;
        let rotated: serde_json::Value = client
            .call(
                lucy_protocol::rpc::method::REMOTE_ROTATE,
                serde_json::json!({}),
            )
            .await?;
        match rotated["token"].as_str() {
            Some(token) => Ok(Some(token.to_owned())),
            None => bail!("the core didn't return a new token"),
        }
    })
}

/// `lucy remote connect <url>`: check the token works, then remember the core and the token.
pub async fn connect_command(url: &str) -> Result<()> {
    let url = normalize_url(url)?;
    let token = if std::io::stdin().is_terminal() {
        Password::new()
            .with_prompt("Token (from `lucy remote token` on that machine)")
            .interact()?
    } else {
        let mut token = String::new();
        std::io::stdin().read_to_string(&mut token)?;
        token.trim().to_owned()
    };
    if token.is_empty() {
        bail!("no token given");
    }
    let endpoint = Endpoint::Remote {
        url: url.clone(),
        token: token.clone(),
    };
    let client = Client::connect(&endpoint, crate::core_process::CLIENT_NAME)
        .await
        .with_context(|| format!("couldn't connect to the Lucy core at {url}"))?;
    save_connection(&connection_file()?, &url)?;
    println!(
        "Connected to Lucy's core at {url} (v{}). `lucy` here now talks to it; its tools run \
         on that machine, in {}.",
        client.core_version(),
        client.folder().unwrap_or("its home folder")
    );
    println!("`lucy remote disconnect` goes back to this machine's own core.");
    if let Err(err) = keyring_hint(OsKeyring.set(CLIENT_TOKEN_KEY, &token), CLIENT_TOKEN_KEY) {
        eprintln!("Couldn't keep the token, though: {err:#}");
    }
    Ok(())
}

/// `lucy remote disconnect`: forget the remote core and its token.
pub fn disconnect_command() -> Result<()> {
    let forgot = forget_connection(&connection_file()?)?;
    let _ = OsKeyring.delete(CLIENT_TOKEN_KEY);
    if forgot {
        println!("Disconnected. `lucy` uses this machine's own core again.");
    } else {
        println!("This machine wasn't connected to a remote core.");
    }
    Ok(())
}

/// Turn "no keyring" into an error that says which env var to use instead.
fn keyring_hint<T>(result: Result<T, SecretsError>, key_name: &str) -> Result<T> {
    result.map_err(|err| match err {
        SecretsError::Unavailable(reason) => SecretsError::UnavailableFor {
            reason,
            env_var: secrets::env_var_name(key_name),
        }
        .into(),
        err => err.into(),
    })
}
