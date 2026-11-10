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
    assert_eq!(
        config.sprite.bubble_timeout_secs, 0,
        "replies stay up until the next message"
    );
}

#[test]
fn example_config_in_docs_parses() {
    let example = include_str!("../../../docs/config.example.toml");
    let config = Config::from_toml_str(example).unwrap();

    assert_eq!(config.defaults.agent_profile.as_deref(), Some("claude"));
    assert_eq!(config.agent.workspace, Some(PathBuf::from("~/Projects")));
    assert_eq!(config.profiles["claude"].provider, ProviderKind::Anthropic);
    assert_eq!(config.profiles.len(), 5);
    assert!(config.profiles["claude"].vision);
    assert_eq!(config.images.generator.as_deref(), Some("comfyui"));
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

#[test]
fn user_name_is_optional() {
    assert_eq!(Config::from_toml_str(MINIMAL).unwrap().user.name, None);

    let named = Config::from_toml_str(&format!("{MINIMAL}\n[user]\nname = \"Sam\"\n")).unwrap();
    assert_eq!(named.user.name.as_deref(), Some("Sam"));
}

#[test]
fn the_app_starts_at_login_with_app_autostart_or_the_older_sprite_setting() {
    let off = Config::from_toml_str(MINIMAL).unwrap();
    assert!(!off.starts_at_login());
    assert!(off.sprite.enabled, "the sprite shows unless turned off");

    let new = Config::from_toml_str(&format!("{MINIMAL}\n[app]\nautostart = true\n")).unwrap();
    assert!(new.starts_at_login());

    // Configs from v0.4.0 said it under [sprite].
    let old = Config::from_toml_str(&format!("{MINIMAL}\n[sprite]\nautostart = true\n")).unwrap();
    assert!(old.starts_at_login());

    let hidden = Config::from_toml_str(&format!("{MINIMAL}\n[sprite]\nenabled = false\n")).unwrap();
    assert!(!hidden.sprite.enabled);
}

// --- Pictures ----------------------------------------------------------------------------------

#[test]
fn pictures_are_off_until_a_generator_is_set() {
    let config = Config::from_toml_str(MINIMAL).unwrap();

    assert_eq!(config.images.generator, None);
    assert_eq!(config.images.vision_profile, None);
    assert_eq!(config.images.comfyui.url, "http://127.0.0.1:8188");
    assert!(!config.profiles["local"].vision);
}

#[test]
fn comfyui_or_an_openai_compatible_profile_can_make_pictures() {
    let comfyui = format!("{MINIMAL}\n[images]\ngenerator = \"comfyui\"\n");
    let profile = format!("{MINIMAL}\n[images]\ngenerator = \"local\"\n");

    assert!(Config::from_toml_str(&comfyui).is_ok());
    assert!(Config::from_toml_str(&profile).is_ok());
}

#[test]
fn a_generator_must_be_a_profile_that_can_make_pictures() {
    let unknown = error_for(&format!("{MINIMAL}\n[images]\ngenerator = \"dalle\"\n"));
    let claude = error_for(&format!(
        "{MINIMAL}\n[profiles.claude]\nprovider = \"anthropic\"\nmodel = \"claude-opus-5-5\"\n\
         key_name = \"anthropic\"\n\n[images]\ngenerator = \"claude\"\n"
    ));

    assert!(unknown.contains("unknown profile `dalle`"), "{unknown}");
    assert!(claude.contains("can't make pictures"), "{claude}");
}

#[test]
fn the_vision_helper_must_be_able_to_see() {
    let err = error_for(&format!(
        "{MINIMAL}\n[images]\nvision_profile = \"local\"\n"
    ));

    assert!(err.contains("vision = true"), "{err}");
}
