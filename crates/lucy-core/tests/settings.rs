//! Settings over the core's RPC, for the GUI (and later a phone): config, keys and persona.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{
    ConfigUpdateResult, ConfigView, KeyStatus, PersonaView, ProviderPreset, method,
};
use serde_json::{Value, json};

const CONFIG: &str = r#"# Hand-written. Keep me.
[defaults]
profile = "local"

[profiles.local]
provider = "openai_compat"
base_url = "http://127.0.0.1:9/v1"
model = "qwen3:14b"

[profiles.claude]
provider = "anthropic"
model = "claude-opus-5-5"
key_name = "anthropic"
"#;

struct Harness {
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    keyring: Arc<MemoryStore>,
}

/// The keyring the core uses, shared with the test so it can look inside.
struct Shared(Arc<MemoryStore>);

impl SecretStore for Shared {
    fn get(&self, key_name: &str) -> Result<Option<String>, lucy_core::secrets::SecretsError> {
        self.0.get(key_name)
    }
    fn set(&self, key_name: &str, secret: &str) -> Result<(), lucy_core::secrets::SecretsError> {
        self.0.set(key_name, secret)
    }
    fn delete(&self, key_name: &str) -> Result<bool, lucy_core::secrets::SecretsError> {
        self.0.delete(key_name)
    }
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, CONFIG).unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-settings-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let keyring = Arc::new(MemoryStore::default());
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("data/lucy.sqlite"),
            personas_dir: Some(dir.path().join("config/personas")),
            secrets: Secrets::new(Box::new(Shared(keyring.clone())), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
            clock: None,
            notifier: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Stream::connect(endpoint.name().unwrap()).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            dir,
            endpoint,
            keyring,
        }
    }

    async fn client(&self) -> Client {
        Client::connect(&self.endpoint, "test").await.unwrap()
    }

    fn config_path(&self) -> PathBuf {
        self.dir.path().join("config/config.toml")
    }
}

#[tokio::test]
async fn config_get_shows_the_config_and_where_it_lives() {
    let harness = Harness::start().await;
    let client = harness.client().await;

    let view: ConfigView = client.call(method::CONFIG_GET, json!({})).await.unwrap();

    assert_eq!(view.path, harness.config_path().display().to_string());
    assert!(!view.managed);
    let config = view.config.expect("there is a config");
    assert_eq!(config["defaults"]["profile"], "local");
    assert_eq!(config["profiles"]["claude"]["key_name"], "anthropic");
    // Settings left out of the file come with their defaults, so a form can show them.
    assert_eq!(config["agent"]["max_steps"], 25);
    let _: Value = config;
}

#[tokio::test]
async fn config_update_writes_a_valid_change_and_refuses_a_broken_one() {
    let harness = Harness::start().await;
    let client = harness.client().await;

    let done: ConfigUpdateResult = client
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"defaults": {"profile": "claude"}, "agent": {"max_steps": 40}}}),
        )
        .await
        .unwrap();
    assert!(!done.restart_needed);
    let written = std::fs::read_to_string(harness.config_path()).unwrap();
    assert!(
        written.starts_with("# Hand-written. Keep me.\n"),
        "{written}"
    );
    assert!(written.contains("profile = \"claude\""), "{written}");
    let view: ConfigView = client.call(method::CONFIG_GET, json!({})).await.unwrap();
    assert_eq!(view.config.unwrap()["agent"]["max_steps"], 40);

    let refused = client
        .call::<_, ConfigUpdateResult>(
            method::CONFIG_UPDATE,
            json!({"patch": {"defaults": {"profile": "missing"}}}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("missing"), "{refused}");
    assert_eq!(
        std::fs::read_to_string(harness.config_path()).unwrap(),
        written
    );
}

/// Remote access follows its settings straight away, so nothing waits for a restart.
#[tokio::test]
async fn changing_remote_access_needs_no_restart() {
    let harness = Harness::start().await;
    let client = harness.client().await;

    let done: ConfigUpdateResult = client
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"remote": {"max_level": 2}}}),
        )
        .await
        .unwrap();

    assert!(!done.restart_needed);
}

#[tokio::test]
async fn keys_can_be_set_and_removed_but_never_read_back() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    const SECRET: &str = "sk-ant-very-secret-123";

    let before: Vec<KeyStatus> = client.call(method::KEYS_STATUS, json!({})).await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].name, "anthropic");
    assert_eq!(before[0].used_by, ["claude"]);
    assert_eq!(before[0].source, None);

    let set: Value = client
        .call(
            method::KEYS_SET,
            json!({"name": "anthropic", "value": SECRET}),
        )
        .await
        .unwrap();
    assert!(!set.to_string().contains(SECRET));
    assert_eq!(
        harness.keyring.get("anthropic").unwrap().as_deref(),
        Some(SECRET)
    );

    let after: Value = client.call(method::KEYS_STATUS, json!({})).await.unwrap();
    assert!(!after.to_string().contains(SECRET), "{after}");
    assert_eq!(after[0]["source"], "keyring");

    let _: Value = client
        .call(method::KEYS_REMOVE, json!({"name": "anthropic"}))
        .await
        .unwrap();
    assert_eq!(harness.keyring.get("anthropic").unwrap(), None);
    let gone: Vec<KeyStatus> = client.call(method::KEYS_STATUS, json!({})).await.unwrap();
    assert_eq!(gone[0].source, None);
}

#[tokio::test]
async fn the_persona_can_be_edited_and_reset_to_the_built_in_one() {
    let harness = Harness::start().await;
    let client = harness.client().await;
    let override_file = harness.dir.path().join("config/personas/lucy.md");

    let builtin: PersonaView = client.call(method::PERSONA_GET, json!({})).await.unwrap();
    assert_eq!(builtin.name, "lucy");
    assert!(!builtin.overridden);
    assert!(builtin.text.contains("Rules that outrank the character"));

    let _: Value = client
        .call(
            method::PERSONA_SET,
            json!({"text": "# Persona: Lucy\n\nBe brief.\n"}),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&override_file).unwrap(),
        "# Persona: Lucy\n\nBe brief.\n"
    );
    let edited: PersonaView = client.call(method::PERSONA_GET, json!({})).await.unwrap();
    assert!(edited.overridden);
    assert_eq!(edited.text, "# Persona: Lucy\n\nBe brief.\n");

    let _: Value = client.call(method::PERSONA_RESET, json!({})).await.unwrap();
    assert!(!override_file.exists());
    let reset: PersonaView = client.call(method::PERSONA_GET, json!({})).await.unwrap();
    assert_eq!(reset, builtin);
}

#[tokio::test]
async fn setup_offers_the_same_providers_as_the_terminal_wizard() {
    let harness = Harness::start().await;
    let client = harness.client().await;

    let presets: Vec<ProviderPreset> = client.call(method::SETUP_PRESETS, json!({})).await.unwrap();

    let names: Vec<&str> = presets.iter().map(|p| p.profile_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "local-qwen",
            "qwen-cloud",
            "openrouter",
            "claude",
            "openai",
            "custom"
        ]
    );
    let local = &presets[0];
    assert_eq!(local.provider, "openai_compat");
    assert_eq!(local.base_url.as_deref(), Some("http://localhost:11434/v1"));
    assert_eq!(local.model.as_deref(), Some("qwen3:14b"));
    assert_eq!(local.key_name, None);
    let claude = &presets[3];
    assert_eq!(claude.provider, "anthropic");
    assert_eq!(claude.key_name.as_deref(), Some("anthropic"));
    // The last one is for any other endpoint: you fill in the URL and model.
    let custom = presets.last().unwrap();
    assert_eq!(
        (custom.base_url.as_deref(), custom.model.as_deref()),
        (None, None)
    );
}
