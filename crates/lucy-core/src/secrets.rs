//! API keys: OS keyring with environment-variable override. See ADR 0006.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

/// Where secrets are persisted. The OS keyring in production, in memory in tests.
pub trait SecretStore {
    fn get(&self, key_name: &str) -> Result<Option<String>, SecretsError>;
    fn set(&self, key_name: &str, secret: &str) -> Result<(), SecretsError>;
    /// Returns whether a secret was there to delete.
    fn delete(&self, key_name: &str) -> Result<bool, SecretsError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("keyring error: {0}")]
    Store(String),
    /// No usable OS keyring on this machine.
    #[error("OS keyring unavailable: {0}")]
    Unavailable(String),
    #[error(
        "OS keyring unavailable ({reason}). Set {env_var} instead, \
         or run a Secret Service provider such as gnome-keyring."
    )]
    UnavailableFor { reason: String, env_var: String },
}

/// The platform keyring: Secret Service, macOS Keychain, or Windows Credential Manager.
/// Entries are stored under service `lucy`, with the key name as the user.
pub struct OsKeyring;

impl OsKeyring {
    const SERVICE: &str = "lucy";

    fn entry(key_name: &str) -> Result<keyring::Entry, SecretsError> {
        keyring::Entry::new(Self::SERVICE, key_name).map_err(keyring_error)
    }
}

impl SecretStore for OsKeyring {
    fn get(&self, key_name: &str) -> Result<Option<String>, SecretsError> {
        match Self::entry(key_name)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn set(&self, key_name: &str, secret: &str) -> Result<(), SecretsError> {
        Self::entry(key_name)?
            .set_password(secret)
            .map_err(keyring_error)
    }

    fn delete(&self, key_name: &str) -> Result<bool, SecretsError> {
        match Self::entry(key_name)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(keyring_error(err)),
        }
    }
}

fn keyring_error(err: keyring::Error) -> SecretsError {
    match err {
        keyring::Error::NoDefaultStore => {
            let reason = match keyring::Entry::store_status() {
                Err(init_err) => init_err.to_string(),
                Ok(()) => err.to_string(),
            };
            SecretsError::Unavailable(reason)
        }
        keyring::Error::NoStorageAccess(_) => SecretsError::Unavailable(err.to_string()),
        err => SecretsError::Store(err.to_string()),
    }
}

/// An in-memory [`SecretStore`] for tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key_name: &str) -> Result<Option<String>, SecretsError> {
        Ok(self.0.lock().unwrap().get(key_name).cloned())
    }

    fn set(&self, key_name: &str, secret: &str) -> Result<(), SecretsError> {
        self.0
            .lock()
            .unwrap()
            .insert(key_name.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key_name: &str) -> Result<bool, SecretsError> {
        Ok(self.0.lock().unwrap().remove(key_name).is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// An environment variable, by name.
    Env(String),
    Keyring,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedKey {
    pub value: String,
    pub source: KeySource,
}

impl fmt::Debug for ResolvedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedKey")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

/// Looks up API keys by the `key_name` a profile references.
pub struct Secrets<S, E> {
    store: S,
    env: E,
}

pub type SystemSecrets = Secrets<OsKeyring, fn(&str) -> Option<String>>;

impl SystemSecrets {
    /// The OS keyring and the process environment.
    pub fn system() -> Self {
        Secrets::new(OsKeyring, |var| std::env::var(var).ok())
    }
}

impl<S, E> Secrets<S, E>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    pub fn new(store: S, env: E) -> Self {
        Self { store, env }
    }

    /// Precedence: `LUCY_KEY_<NAME>`, then the keyring, then the vendor's own variable
    /// (e.g. `ANTHROPIC_API_KEY`). Empty variables count as unset.
    pub fn resolve(&self, key_name: &str) -> Result<Option<ResolvedKey>, SecretsError> {
        let lucy_var = env_var_name(key_name);
        if let Some(key) = self.env_key(lucy_var.clone()) {
            return Ok(Some(key));
        }
        let stored = self.store.get(key_name);
        if let Ok(Some(value)) = stored {
            return Ok(Some(ResolvedKey {
                value,
                source: KeySource::Keyring,
            }));
        }
        if let Some(key) = vendor_env_var(key_name).and_then(|var| self.env_key(var.to_owned())) {
            return Ok(Some(key));
        }
        match stored {
            Err(SecretsError::Unavailable(reason)) => Err(SecretsError::UnavailableFor {
                reason,
                env_var: lucy_var,
            }),
            Err(err) => Err(err),
            Ok(_) => Ok(None),
        }
    }

    /// Store a key in the keyring (what `lucy auth set` does).
    pub fn set(&self, key_name: &str, secret: &str) -> Result<(), SecretsError> {
        self.store
            .set(key_name, secret)
            .map_err(|err| for_key(err, key_name))
    }

    /// Remove a key from the keyring; false if it wasn't there.
    pub fn remove(&self, key_name: &str) -> Result<bool, SecretsError> {
        self.store
            .delete(key_name)
            .map_err(|err| for_key(err, key_name))
    }

    fn env_key(&self, var: String) -> Option<ResolvedKey> {
        (self.env)(&var)
            .filter(|value| !value.is_empty())
            .map(|value| ResolvedKey {
                value,
                source: KeySource::Env(var),
            })
    }
}

/// The `LUCY_KEY_*` variable that overrides `key_name`.
pub fn env_var_name(key_name: &str) -> String {
    let suffix: String = key_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("LUCY_KEY_{suffix}")
}

/// The variable a vendor's own tools use for the same key, if Lucy knows it.
fn vendor_env_var(key_name: &str) -> Option<&'static str> {
    match key_name {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "dashscope" => Some("DASHSCOPE_API_KEY"),
        _ => None,
    }
}

impl<T: SecretStore + ?Sized> SecretStore for Box<T> {
    fn get(&self, key_name: &str) -> Result<Option<String>, SecretsError> {
        (**self).get(key_name)
    }

    fn set(&self, key_name: &str, secret: &str) -> Result<(), SecretsError> {
        (**self).set(key_name, secret)
    }

    fn delete(&self, key_name: &str) -> Result<bool, SecretsError> {
        (**self).delete(key_name)
    }
}

/// Turn "no keyring" into an error that says which variable to set instead.
fn for_key(err: SecretsError, key_name: &str) -> SecretsError {
    match err {
        SecretsError::Unavailable(reason) => SecretsError::UnavailableFor {
            reason,
            env_var: env_var_name(key_name),
        },
        err => err,
    }
}

/// Keyring entries Lucy keeps for herself (remote access tokens, MCP sign-ins) are never used
/// as an API key (by a profile, a model listing or an MCP server) and aren't set or removed as
/// one. Compared by their `LUCY_KEY_*` name, so `remote_token` counts too.
pub fn not_reserved(key_name: &str) -> Result<(), String> {
    let name = env_var_name(key_name);
    let reserved = ["LUCY_KEY_REMOTE_", "LUCY_KEY_MCP_OAUTH_"];
    if reserved.iter().any(|prefix| name.starts_with(prefix)) {
        return Err(format!(
            "`{key_name}` is one of Lucy's own keyring entries (remote access or an MCP sign-in), \
             not an API key"
        ));
    }
    Ok(())
}
