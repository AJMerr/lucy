//! `lucy remote token` and `lucy remote connect|disconnect`: the core's token in the keyring,
//! and where a client keeps the core it talks to.

use lucy_cli::remote::{
    forget_connection, normalize_url, save_connection, saved_connection, token,
};
use lucy_core::secrets::{MemoryStore, SecretStore};
use lucy_core::server::REMOTE_TOKEN_KEY;

#[test]
fn the_token_is_created_once_and_then_kept() {
    let keyring = MemoryStore::default();

    let first = token(&keyring, false).unwrap();
    let second = token(&keyring, false).unwrap();

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.value, second.value);
    assert_eq!(first.value.len(), 64);
    assert!(first.value.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(keyring.get(REMOTE_TOKEN_KEY).unwrap(), Some(first.value));
}

#[test]
fn rotating_replaces_the_token() {
    let keyring = MemoryStore::default();
    let old = token(&keyring, false).unwrap();

    let new = token(&keyring, true).unwrap();

    assert!(new.created);
    assert_ne!(new.value, old.value);
    assert_eq!(keyring.get(REMOTE_TOKEN_KEY).unwrap(), Some(new.value));
}

#[test]
fn addresses_become_websocket_urls() {
    for (given, expected) in [
        ("desktop", "ws://desktop:7077/rpc"),
        ("desktop:8000", "ws://desktop:8000/rpc"),
        ("ws://desktop:7077", "ws://desktop:7077/rpc"),
        ("ws://100.64.0.7:7077/rpc", "ws://100.64.0.7:7077/rpc"),
        (
            "wss://desktop.tail1234.ts.net/rpc",
            "wss://desktop.tail1234.ts.net/rpc",
        ),
    ] {
        assert_eq!(normalize_url(given).unwrap(), expected, "{given}");
    }
    let err = normalize_url("http://desktop:7077")
        .unwrap_err()
        .to_string();
    assert!(err.contains("ws://"), "{err}");
}

#[test]
fn a_connection_is_saved_and_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lucy/remote.toml");

    assert_eq!(saved_connection(&file).unwrap(), None);
    save_connection(&file, "ws://desktop:7077/rpc").unwrap();
    assert_eq!(
        saved_connection(&file).unwrap().as_deref(),
        Some("ws://desktop:7077/rpc")
    );

    assert!(forget_connection(&file).unwrap());
    assert_eq!(saved_connection(&file).unwrap(), None);
    assert!(!forget_connection(&file).unwrap(), "nothing left to forget");
}

#[test]
fn reconnecting_keeps_comments() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("remote.toml");
    std::fs::write(
        &file,
        "# the desktop under the desk\nurl = \"ws://old:7077/rpc\" # via Tailscale\n",
    )
    .unwrap();

    save_connection(&file, "ws://desktop:7077/rpc").unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("# the desktop under the desk"), "{text}");
    assert_eq!(
        saved_connection(&file).unwrap().as_deref(),
        Some("ws://desktop:7077/rpc")
    );
}
