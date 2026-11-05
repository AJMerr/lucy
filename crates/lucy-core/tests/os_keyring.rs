//! Talks to the real OS keyring, so it only runs on demand:
//! `cargo test -p lucy-core --test os_keyring -- --ignored`

use lucy_core::secrets::{OsKeyring, SecretStore};

#[test]
#[ignore = "needs a real OS keyring"]
fn set_get_delete_round_trip() {
    let store = OsKeyring;
    let name = format!("lucy-test-{}", std::process::id());

    store.set(&name, "s3cret").unwrap();
    assert_eq!(store.get(&name).unwrap().as_deref(), Some("s3cret"));

    assert!(store.delete(&name).unwrap());
    assert_eq!(store.get(&name).unwrap(), None);
    assert!(!store.delete(&name).unwrap());
}
