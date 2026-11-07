//! Loading sprite packs: the built-in placeholder, user packs, and the mistakes a pack author
//! can make.

use std::path::Path;

use lucy_packs::{Pack, PackError, load};

/// The smallest thing that passes for a PNG: its 8-byte signature.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A minimal valid manifest: the three required states, one frame each.
const MINIMAL: &str = r#"
name = "Test"
format_version = 1
kind = "frames"
size = [64, 96]
anchor = "bottom-center"

[states.idle]
frames = ["idle.png"]
fps = 2

[states.thinking]
frames = ["think.png"]
fps = 3

[states.talking]
frames = ["talk-0.png", "talk-1.png"]
fps = 6
"#;

/// Write a pack called `name` into `packs`, with `manifest` and a PNG for every frame it
/// names in MINIMAL (plus `extra` files).
fn write_pack(packs: &Path, name: &str, manifest: &str, extra: &[(&str, &[u8])]) {
    let dir = packs.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pack.toml"), manifest).unwrap();
    for frame in ["idle.png", "think.png", "talk-0.png", "talk-1.png"] {
        std::fs::write(dir.join(frame), PNG).unwrap();
    }
    for (file, bytes) in extra {
        std::fs::write(dir.join(file), bytes).unwrap();
    }
}

fn error(packs: &Path, name: &str) -> String {
    load(name, Some(packs)).unwrap_err().to_string()
}

#[test]
fn the_built_in_placeholder_loads() {
    let pack = load("placeholder", None).unwrap();

    assert_eq!(pack.name, "Placeholder");
    assert_eq!(pack.size, (128, 128));
    let idle = &pack.states["idle"];
    assert_eq!(idle.frames.len(), 4);
    assert_eq!(idle.fps, 2.0);
    assert!(idle.frames.iter().all(|frame| frame.png.starts_with(PNG)));
    let happy = &pack.states["happy"];
    assert!(!happy.looping);
    assert_eq!(happy.then.as_deref(), Some("idle"));
}

#[test]
fn optional_states_fall_back_to_idle() {
    let packs = tempfile::tempdir().unwrap();
    write_pack(packs.path(), "mine", MINIMAL, &[]);

    let pack: Pack = load("mine", Some(packs.path())).unwrap();

    for optional in ["happy", "error", "listening"] {
        assert_eq!(pack.states[optional], pack.states["idle"], "{optional}");
    }
    assert_eq!(pack.states["talking"].frames.len(), 2);
}

#[test]
fn a_user_pack_wins_over_a_built_in_one_of_the_same_name() {
    let packs = tempfile::tempdir().unwrap();
    write_pack(packs.path(), "placeholder", MINIMAL, &[]);

    let pack = load("placeholder", Some(packs.path())).unwrap();

    assert_eq!(pack.name, "Test");
}

#[test]
fn a_missing_pack_says_where_it_looked() {
    let packs = tempfile::tempdir().unwrap();

    let err = load("nobody", Some(packs.path())).unwrap_err();

    assert!(matches!(err, PackError::NotFound { .. }));
    let message = err.to_string();
    assert!(message.contains("nobody"), "{message}");
    assert!(
        message.contains(&packs.path().display().to_string()),
        "{message}"
    );
    assert!(message.contains("placeholder"), "{message}");
}

#[test]
fn a_required_state_must_be_there() {
    let packs = tempfile::tempdir().unwrap();
    let without_talking = MINIMAL.split("[states.talking]").next().unwrap();
    write_pack(packs.path(), "mine", without_talking, &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("talking"), "{message}");
    assert!(message.contains("required"), "{message}");
}

#[test]
fn a_missing_frame_is_named_with_its_state() {
    let packs = tempfile::tempdir().unwrap();
    let manifest = MINIMAL.replace("\"think.png\"", "\"thinking/0.png\"");
    write_pack(packs.path(), "mine", &manifest, &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("thinking/0.png"), "{message}");
    assert!(message.contains("[states.thinking]"), "{message}");
}

#[test]
fn frames_must_be_pngs() {
    let packs = tempfile::tempdir().unwrap();
    let manifest = MINIMAL.replace("\"idle.png\"", "\"idle.gif\"");
    write_pack(packs.path(), "mine", &manifest, &[("idle.gif", b"GIF89a")]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("idle.gif"), "{message}");
    assert!(message.contains("PNG"), "{message}");
}

#[test]
fn frames_stay_inside_the_pack_folder() {
    let packs = tempfile::tempdir().unwrap();
    std::fs::write(packs.path().join("outside.png"), PNG).unwrap();
    for sneaky in ["../outside.png", "/etc/passwd"] {
        let manifest = MINIMAL.replace("\"idle.png\"", &format!("{sneaky:?}"));
        write_pack(packs.path(), "mine", &manifest, &[]);

        let message = error(packs.path(), "mine");

        assert!(message.contains("inside the pack"), "{sneaky}: {message}");
    }
}

#[test]
fn then_must_name_a_real_state() {
    let packs = tempfile::tempdir().unwrap();
    let manifest = format!(
        "{MINIMAL}\n[states.happy]\nframes = [\"idle.png\"]\nfps = 1\nloop = false\nthen = \"idel\"\n"
    );
    write_pack(packs.path(), "mine", &manifest, &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("idel"), "{message}");
    assert!(message.contains("[states.happy]"), "{message}");
}

#[test]
fn unknown_state_names_are_typos() {
    let packs = tempfile::tempdir().unwrap();
    let manifest = format!("{MINIMAL}\n[states.tlaking]\nframes = [\"idle.png\"]\nfps = 1\n");
    write_pack(packs.path(), "mine", &manifest, &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("tlaking"), "{message}");
    assert!(
        message.contains("listening"),
        "lists the known states: {message}"
    );
}

#[test]
fn fps_must_be_positive() {
    let packs = tempfile::tempdir().unwrap();
    let manifest = MINIMAL.replace("fps = 3", "fps = 0");
    write_pack(packs.path(), "mine", &manifest, &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("fps"), "{message}");
    assert!(message.contains("[states.thinking]"), "{message}");
}

#[test]
fn live2d_packs_are_not_supported_yet() {
    let packs = tempfile::tempdir().unwrap();
    write_pack(
        packs.path(),
        "mine",
        &MINIMAL.replace("kind = \"frames\"", "kind = \"live2d\""),
        &[],
    );

    let message = error(packs.path(), "mine");

    assert!(message.contains("live2d"), "{message}");
}

#[test]
fn a_broken_manifest_names_its_file() {
    let packs = tempfile::tempdir().unwrap();
    write_pack(packs.path(), "mine", "name = \n", &[]);

    let message = error(packs.path(), "mine");

    assert!(message.contains("pack.toml"), "{message}");
}
