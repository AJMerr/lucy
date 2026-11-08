//! Changing config.toml from a client (the GUI): a JSON merge patch applied to the file,
//! keeping what the user wrote around it.

use lucy_core::config::{Config, ConfigError, apply_patch, update_file};
use serde_json::json;

const WRITTEN_BY_HAND: &str = r#"# My Lucy config. Keep this comment.
[defaults]
profile = "local" # the one I use most

[profiles.local]
provider = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "qwen3:14b"

# Agent caps, tuned by hand.
[agent]
max_steps = 25
"#;

#[test]
fn a_change_keeps_comments_order_and_everything_else() {
    let text = apply_patch(WRITTEN_BY_HAND, &json!({"agent": {"max_steps": 40}})).unwrap();

    assert_eq!(
        text,
        WRITTEN_BY_HAND.replace("max_steps = 25", "max_steps = 40")
    );
}

#[test]
fn a_changed_value_keeps_its_comment() {
    let patch = json!({
        "defaults": {"profile": "claude"},
        "profiles": {"claude": {"provider": "anthropic", "model": "claude-opus-5-5"}},
    });

    let text = apply_patch(WRITTEN_BY_HAND, &patch).unwrap();

    assert!(
        text.contains("profile = \"claude\" # the one I use most"),
        "{text}"
    );
}

#[test]
fn a_new_profile_becomes_its_own_table() {
    let patch = json!({"profiles": {"claude": {
        "provider": "anthropic",
        "model": "claude-opus-5-5",
        "key_name": "anthropic",
    }}});

    let text = apply_patch(WRITTEN_BY_HAND, &patch).unwrap();

    // Its own `[profiles.claude]` section, after the existing profile (key order inside a new
    // table isn't kept: JSON objects arrive sorted).
    let section = text
        .split("[profiles.claude]\n")
        .nth(1)
        .expect("a [profiles.claude] section")
        .split("\n\n")
        .next()
        .unwrap();
    let mut lines: Vec<&str> = section.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "key_name = \"anthropic\"",
            "model = \"claude-opus-5-5\"",
            "provider = \"anthropic\""
        ]
    );
    assert!(text.find("[profiles.local]") < text.find("[profiles.claude]"));
    let config = Config::from_toml_str(&text).unwrap();
    assert_eq!(config.profiles.len(), 2);
}

#[test]
fn null_removes_a_setting_or_a_whole_table() {
    let with_two = apply_patch(
        WRITTEN_BY_HAND,
        &json!({"profiles": {"spare": {"provider": "anthropic", "model": "m"}}}),
    )
    .unwrap();

    let text = apply_patch(
        &with_two,
        &json!({"profiles": {"spare": null}, "agent": {"max_steps": null}}),
    )
    .unwrap();

    assert!(!text.contains("spare"), "{text}");
    assert!(!text.contains("max_steps"), "{text}");
    assert!(text.contains("# Agent caps, tuned by hand."), "{text}");
}

#[test]
fn a_change_that_would_break_the_config_is_refused() {
    let wrong_type = apply_patch(WRITTEN_BY_HAND, &json!({"agent": {"max_steps": "lots"}}));
    assert!(wrong_type.is_err());

    let typo = apply_patch(WRITTEN_BY_HAND, &json!({"agent": {"max_stpes": 3}}));
    let message = typo.unwrap_err().to_string();
    assert!(message.contains("max_stpes"), "{message}");

    let missing_profile = apply_patch(WRITTEN_BY_HAND, &json!({"defaults": {"profile": "nope"}}));
    assert!(missing_profile.is_err());
}

#[test]
fn a_first_config_can_be_written_from_nothing() {
    let patch = json!({
        "defaults": {"profile": "local"},
        "profiles": {"local": {
            "provider": "openai_compat",
            "base_url": "http://localhost:11434/v1",
            "model": "qwen3:14b",
        }},
        "user": {"name": "Sam"},
    });

    let text = apply_patch("", &patch).unwrap();

    let config = Config::from_toml_str(&text).unwrap();
    assert_eq!(config.defaults.profile, "local");
    assert_eq!(config.user.name.as_deref(), Some("Sam"));
    assert!(text.contains("[profiles.local]"), "{text}");
    assert!(!text.contains("[profiles]\n"), "{text}");
}

#[test]
fn updating_the_file_writes_the_change_and_leaves_it_alone_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, WRITTEN_BY_HAND).unwrap();

    update_file(&path, &json!({"agent": {"max_steps": 40}})).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("max_steps = 40"), "{written}");

    assert!(update_file(&path, &json!({"agent": {"max_steps": "lots"}})).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
}

#[cfg(unix)]
#[test]
fn a_config_home_manager_owns_is_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::os::unix::fs::symlink("/nix/store/0000-lucy-config.toml", &path).unwrap();

    let refused = update_file(&path, &json!({"agent": {"max_steps": 40}}));

    assert!(
        matches!(refused, Err(ConfigError::Managed { .. })),
        "{refused:?}"
    );
}
