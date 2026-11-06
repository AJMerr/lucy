use std::fs;

use chrono::{DateTime, FixedOffset};
use lucy_core::persona::{Persona, PersonaSource, Personas, PromptContext, build_system_prompt};
use lucy_core::sessions::Origin;

#[test]
fn builtin_lucy_loads_without_a_user_folder() {
    let personas = Personas::new(None);

    let lucy = personas.load("lucy").unwrap();

    assert_eq!(lucy.name, "lucy");
    assert_eq!(lucy.source, PersonaSource::Builtin);
    assert!(lucy.text.contains("You are Lucy"));
}

#[test]
fn user_file_overrides_the_builtin_and_adds_new_personas() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lucy.md"), "You are my custom Lucy.").unwrap();
    fs::write(dir.path().join("judy.md"), "You are Judy.").unwrap();
    fs::write(dir.path().join("notes.txt"), "not a persona").unwrap();
    let personas = Personas::new(Some(dir.path().to_owned()));

    let lucy = personas.load("lucy").unwrap();
    assert_eq!(lucy.text, "You are my custom Lucy.");
    assert_eq!(lucy.source, PersonaSource::File(dir.path().join("lucy.md")));
    assert_eq!(personas.load("judy").unwrap().text, "You are Judy.");
    assert_eq!(personas.available(), vec!["judy", "lucy"]);
}

#[test]
fn missing_user_folder_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let personas = Personas::new(Some(dir.path().join("does-not-exist")));

    assert_eq!(
        personas.load("lucy").unwrap().source,
        PersonaSource::Builtin
    );
    assert_eq!(personas.available(), vec!["lucy"]);
}

#[test]
fn unknown_persona_lists_the_available_ones() {
    let err = Personas::new(None).load("adam").unwrap_err().to_string();

    assert!(err.contains("no persona named `adam`"), "{err}");
    assert!(err.contains("available: lucy"), "{err}");
}

#[test]
fn names_that_could_escape_the_folder_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let personas = Personas::new(Some(dir.path().join("personas")));
    fs::write(dir.path().join("secret.md"), "outside").unwrap();

    for name in ["../secret", "a/b", "", "Lucy.md"] {
        let err = personas.load(name).unwrap_err().to_string();
        assert!(err.contains("invalid persona name"), "{name:?}: {err}");
    }
}

const TEST_PERSONA: &str = "\
# Persona: Test

<!-- voice -->
Talk like a pirate.
<!-- /voice -->

<!-- plain -->
Be neutral.
<!-- /plain -->

## Rules
Be accurate.
";

fn test_persona() -> Persona {
    Persona {
        name: "test".into(),
        text: TEST_PERSONA.into(),
        source: PersonaSource::Builtin,
    }
}

fn context(plain: bool) -> PromptContext {
    PromptContext {
        plain,
        now: DateTime::<FixedOffset>::parse_from_rfc3339("2026-11-05T07:58:00-05:00").unwrap(),
        machine: Some("NixOS 26.11 (Yarara), Hyprland, x86_64".into()),
        user_name: Some("Sam".into()),
        origin: Some(Origin {
            profile: "local-qwen".into(),
            model: "qwen3:14b".into(),
        }),
        cwd: None,
        agent: None,
    }
}

const FULL_CONTEXT: &str = "\
## Context

- Current date and time: Thursday 5 November 2026, 07:58 (UTC-05:00)
- Machine: NixOS 26.11 (Yarara), Hyprland, x86_64
- The user's name is Sam.
- You are running as profile `local-qwen` (model `qwen3:14b`).";

#[test]
fn full_prompt_keeps_the_voice_and_appends_context() {
    let prompt = build_system_prompt(&test_persona(), &context(false));

    assert_eq!(
        prompt,
        format!(
            "# Persona: Test\n\nTalk like a pirate.\n\n## Rules\nBe accurate.\n\n{FULL_CONTEXT}"
        )
    );
}

#[test]
fn plain_prompt_swaps_the_voice_for_the_plain_block() {
    let prompt = build_system_prompt(&test_persona(), &context(true));

    assert_eq!(
        prompt,
        format!("# Persona: Test\n\nBe neutral.\n\n## Rules\nBe accurate.\n\n{FULL_CONTEXT}")
    );
}

#[test]
fn unknown_context_is_left_out() {
    let ctx = PromptContext {
        machine: None,
        user_name: None,
        origin: None,
        ..context(false)
    };

    let prompt = build_system_prompt(&test_persona(), &ctx);

    assert!(prompt.ends_with(
        "## Context\n\n- Current date and time: Thursday 5 November 2026, 07:58 (UTC-05:00)"
    ));
}

#[test]
fn builtin_lucy_is_marked_up_for_plain_mode() {
    let lucy = Personas::new(None).load("lucy").unwrap();
    let full = build_system_prompt(&lucy, &context(false));
    let plain = build_system_prompt(&lucy, &context(true));

    for section in ["## Who you are", "## Staying in character"] {
        assert!(full.contains(section), "full prompt lacks {section}");
        assert!(!plain.contains(section), "plain prompt still has {section}");
    }
    assert!(plain.contains("neutral, concise assistant"));
    assert!(!full.contains("neutral, concise assistant"));
    for prompt in [&full, &plain] {
        assert!(prompt.contains("## Rules that outrank the character"));
        assert!(!prompt.contains("<!--"), "marker leaked into prompt");
    }
}
