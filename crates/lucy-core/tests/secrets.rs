use std::collections::HashMap;

use lucy_core::secrets::{KeySource, MemoryStore, SecretStore, Secrets, SecretsError};

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| vars.get(name).cloned()
}

fn keyring_with(entries: &[(&str, &str)]) -> MemoryStore {
    let store = MemoryStore::default();
    for (name, secret) in entries {
        store.set(name, secret).unwrap();
    }
    store
}

#[test]
fn lucy_env_var_overrides_keyring() {
    let secrets = Secrets::new(
        keyring_with(&[("anthropic", "from-keyring")]),
        env(&[("LUCY_KEY_ANTHROPIC", "from-env")]),
    );

    let key = secrets.resolve("anthropic").unwrap().unwrap();
    assert_eq!(key.value, "from-env");
    assert_eq!(key.source, KeySource::Env("LUCY_KEY_ANTHROPIC".into()));
}

#[test]
fn keyring_is_used_when_no_env_var_is_set() {
    let secrets = Secrets::new(keyring_with(&[("anthropic", "from-keyring")]), env(&[]));

    let key = secrets.resolve("anthropic").unwrap().unwrap();
    assert_eq!(key.value, "from-keyring");
    assert_eq!(key.source, KeySource::Keyring);
}

#[test]
fn vendor_env_var_is_the_last_resort() {
    let secrets = Secrets::new(
        MemoryStore::default(),
        env(&[("ANTHROPIC_API_KEY", "from-vendor-var")]),
    );

    let key = secrets.resolve("anthropic").unwrap().unwrap();
    assert_eq!(key.value, "from-vendor-var");
    assert_eq!(key.source, KeySource::Env("ANTHROPIC_API_KEY".into()));
}

#[test]
fn keyring_beats_vendor_env_var() {
    let secrets = Secrets::new(
        keyring_with(&[("openrouter", "from-keyring")]),
        env(&[("OPENROUTER_API_KEY", "from-vendor-var")]),
    );

    assert_eq!(
        secrets.resolve("openrouter").unwrap().unwrap().source,
        KeySource::Keyring
    );
}

#[test]
fn every_preset_key_name_has_a_vendor_fallback() {
    for (key_name, var) in [
        ("anthropic", "ANTHROPIC_API_KEY"),
        ("openai", "OPENAI_API_KEY"),
        ("openrouter", "OPENROUTER_API_KEY"),
        ("dashscope", "DASHSCOPE_API_KEY"),
    ] {
        let secrets = Secrets::new(MemoryStore::default(), env(&[(var, "k")]));
        assert_eq!(
            secrets.resolve(key_name).unwrap().unwrap().source,
            KeySource::Env(var.into()),
            "{key_name}"
        );
    }
}

#[test]
fn key_names_with_punctuation_map_to_a_valid_env_var() {
    let secrets = Secrets::new(
        MemoryStore::default(),
        env(&[("LUCY_KEY_HOME_LAB_2", "from-env")]),
    );

    assert_eq!(
        secrets.resolve("home-lab.2").unwrap().unwrap().value,
        "from-env"
    );
}

#[test]
fn empty_env_var_counts_as_unset() {
    let secrets = Secrets::new(
        keyring_with(&[("anthropic", "from-keyring")]),
        env(&[("LUCY_KEY_ANTHROPIC", "")]),
    );

    assert_eq!(
        secrets.resolve("anthropic").unwrap().unwrap().source,
        KeySource::Keyring
    );
}

#[test]
fn missing_key_resolves_to_none() {
    let secrets = Secrets::new(MemoryStore::default(), env(&[]));
    assert_eq!(secrets.resolve("anthropic").unwrap(), None);
}

/// A machine with no OS keyring (e.g. no Secret Service running).
struct NoKeyring;

impl SecretStore for NoKeyring {
    fn get(&self, _: &str) -> Result<Option<String>, SecretsError> {
        Err(SecretsError::Unavailable(
            "no Secret Service on the session bus".into(),
        ))
    }
    fn set(&self, _: &str, _: &str) -> Result<(), SecretsError> {
        Err(SecretsError::Unavailable(
            "no Secret Service on the session bus".into(),
        ))
    }
    fn delete(&self, _: &str) -> Result<bool, SecretsError> {
        Err(SecretsError::Unavailable(
            "no Secret Service on the session bus".into(),
        ))
    }
}

#[test]
fn env_vars_still_work_without_a_keyring() {
    let lucy_var = Secrets::new(NoKeyring, env(&[("LUCY_KEY_ANTHROPIC", "a")]));
    assert_eq!(lucy_var.resolve("anthropic").unwrap().unwrap().value, "a");

    let vendor_var = Secrets::new(NoKeyring, env(&[("ANTHROPIC_API_KEY", "b")]));
    assert_eq!(vendor_var.resolve("anthropic").unwrap().unwrap().value, "b");
}

#[test]
fn missing_keyring_error_suggests_the_env_var() {
    let secrets = Secrets::new(NoKeyring, env(&[]));

    let err = secrets.resolve("anthropic").unwrap_err().to_string();
    assert!(
        err.contains("no Secret Service on the session bus"),
        "{err}"
    );
    assert!(err.contains("LUCY_KEY_ANTHROPIC"), "{err}");
}
