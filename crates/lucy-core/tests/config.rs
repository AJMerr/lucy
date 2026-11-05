use std::path::PathBuf;

use lucy_core::config::{Config, ProviderKind};
use lucy_core::protocol::{AgentLevel, Mode};

const MINIMAL: &str = r#"
[defaults]
profile = "local"

[profiles.local]
provider = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "qwen3:14b"
"#;

#[test]
fn minimal_config_fills_in_defaults() {
    let config = Config::from_toml_str(MINIMAL).unwrap();

    assert_eq!(config.defaults.profile, "local");
    assert_eq!(config.defaults.agent_profile, None);
    assert_eq!(config.defaults.persona, "lucy");
    assert_eq!(config.defaults.mode, Mode::Assist);
    assert_eq!(config.defaults.agent_level, AgentLevel::Guarded);

    let local = &config.profiles["local"];
    assert_eq!(local.provider, ProviderKind::OpenAiCompat);
    assert_eq!(local.base_url.as_deref(), Some("http://localhost:11434/v1"));
    assert_eq!(local.model, "qwen3:14b");
    assert_eq!(local.key_name, None);

    assert_eq!(config.agent.max_steps, 25);
    assert_eq!(config.agent.max_cost_usd, 1.0);
    assert_eq!(config.agent.workspace, None);

    assert!(!config.remote.enabled);
    assert_eq!(config.remote.bind.to_string(), "127.0.0.1:7077");

    assert_eq!(config.sprite.pack, "placeholder");
    assert_eq!(config.sprite.scale, 1.0);
    assert_eq!(config.sprite.bubble_timeout_secs, 12);
}

#[test]
fn example_config_in_docs_parses() {
    let example = include_str!("../../../docs/config.example.toml");
    let config = Config::from_toml_str(example).unwrap();

    assert_eq!(config.defaults.agent_profile.as_deref(), Some("claude"));
    assert_eq!(config.agent.workspace, Some(PathBuf::from("~/Projects")));
    assert_eq!(config.profiles["claude"].provider, ProviderKind::Anthropic);
    assert_eq!(config.profiles.len(), 4);
}

fn error_for(source: &str) -> String {
    Config::from_toml_str(source).unwrap_err().to_string()
}

#[test]
fn typo_in_key_is_rejected_with_its_name_and_line() {
    let err = error_for(&MINIMAL.replace("[defaults]", "[defaults]\nagnet_level = 2"));
    assert!(err.contains("agnet_level"), "{err}");
    assert!(err.contains("line 3"), "{err}");
}

#[test]
fn agent_level_out_of_range_is_rejected() {
    let err = error_for(&MINIMAL.replace("[defaults]", "[defaults]\nagent_level = 7"));
    assert!(err.contains("agent level must be 0–3, got 7"), "{err}");
}

#[test]
fn unknown_provider_is_rejected() {
    let err = error_for(&MINIMAL.replace("openai_compat", "skynet"));
    assert!(err.contains("skynet"), "{err}");
}

#[test]
fn default_profile_must_exist() {
    let err = error_for(&MINIMAL.replace(r#"profile = "local""#, r#"profile = "nope""#));
    assert!(
        err.contains("defaults.profile refers to unknown profile `nope`"),
        "{err}"
    );
}

#[test]
fn agent_profile_must_exist() {
    let err = error_for(&MINIMAL.replace("[defaults]", "[defaults]\nagent_profile = \"ghost\""));
    assert!(
        err.contains("defaults.agent_profile refers to unknown profile `ghost`"),
        "{err}"
    );
}

#[test]
fn openai_compat_profile_needs_base_url() {
    let err = error_for(&MINIMAL.replace("base_url = \"http://localhost:11434/v1\"\n", ""));
    assert!(
        err.contains("profile `local` uses openai_compat and needs a base_url"),
        "{err}"
    );
}

#[test]
fn loading_a_missing_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Config::load(&dir.path().join("config.toml")).unwrap(), None);
}

#[test]
fn saved_config_loads_back_identically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/lucy/config.toml");
    let example = include_str!("../../../docs/config.example.toml");
    let config = Config::from_toml_str(example).unwrap();

    config.save(&path).unwrap();

    assert_eq!(Config::load(&path).unwrap(), Some(config));
}

#[test]
fn load_reports_the_file_path_on_invalid_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "this is not toml").unwrap();

    let err = Config::load(&path).unwrap_err().to_string();
    assert!(err.contains(&path.display().to_string()), "{err}");
}
