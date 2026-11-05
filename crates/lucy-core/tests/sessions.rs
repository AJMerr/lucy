use std::path::PathBuf;

use lucy_core::protocol::{Message, Role};
use lucy_core::sessions::{NewMessage, Origin, SessionId, SessionStore, StoreError, Usage};

struct TempDb {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn temp_db() -> TempDb {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data/lucy/lucy.sqlite");
    TempDb { _dir: dir, path }
}

#[test]
fn new_session_is_listed_untitled_and_survives_reopening() {
    let db = temp_db();
    let id = SessionStore::open(&db.path).unwrap().create().unwrap();

    let store = SessionStore::open(&db.path).unwrap();
    let sessions = store.list().unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, id);
    assert_eq!(sessions[0].title, None);
    assert_eq!(sessions[0].message_count, 0);
    assert_eq!(id.to_string().len(), 26, "ULID string");
}

fn qwen() -> Origin {
    Origin {
        profile: "local-qwen".into(),
        model: "qwen3:14b".into(),
    }
}

#[test]
fn appended_messages_load_back_in_order_with_their_details() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let id = store.create().unwrap();

    store.append(&id, NewMessage::user("hi")).unwrap();
    store
        .append(
            &id,
            NewMessage::assistant("Hey, choom.", qwen()).with_usage(Usage {
                input_tokens: 21,
                output_tokens: 4,
            }),
        )
        .unwrap();
    store
        .append(&id, NewMessage::user("tell me a story"))
        .unwrap();
    store
        .append(
            &id,
            NewMessage::assistant("Once, in Night", qwen()).incomplete(),
        )
        .unwrap();

    let session = SessionStore::open(&db.path).unwrap().load(&id).unwrap();
    let got: Vec<_> = session
        .messages
        .iter()
        .map(|m| (m.message.clone(), m.origin.clone(), m.incomplete, m.usage))
        .collect();

    assert_eq!(
        got,
        vec![
            (Message::new(Role::User, "hi"), None, false, None),
            (
                Message::new(Role::Assistant, "Hey, choom."),
                Some(qwen()),
                false,
                Some(Usage {
                    input_tokens: 21,
                    output_tokens: 4
                })
            ),
            (
                Message::new(Role::User, "tell me a story"),
                None,
                false,
                None
            ),
            (
                Message::new(Role::Assistant, "Once, in Night"),
                Some(qwen()),
                true,
                None
            ),
        ]
    );
    assert_eq!(session.summary.message_count, 4);
}

#[test]
fn title_comes_from_the_first_user_message() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let short = store.create().unwrap();
    let long = store.create().unwrap();

    store
        .append(&short, NewMessage::user("  fix my\nflake.nix  "))
        .unwrap();
    store
        .append(&short, NewMessage::user("something else"))
        .unwrap();
    store
        .append(
            &long,
            NewMessage::user(
                "Explain how the borrow checker decides when a mutable reference ends, with examples",
            ),
        )
        .unwrap();

    assert_eq!(
        store.load(&short).unwrap().summary.title.as_deref(),
        Some("fix my")
    );
    assert_eq!(
        store.load(&long).unwrap().summary.title.as_deref(),
        Some("Explain how the borrow checker decides when a mutable refer…")
    );
}

fn ids(store: &SessionStore) -> Vec<SessionId> {
    store.list().unwrap().into_iter().map(|s| s.id).collect()
}

#[test]
fn list_puts_the_most_recently_active_session_first() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let older = store.create().unwrap();
    let newer = store.create().unwrap();
    assert_eq!(ids(&store), vec![newer.clone(), older.clone()]);

    store
        .append(&older, NewMessage::user("back to this one"))
        .unwrap();

    assert_eq!(ids(&store), vec![older, newer]);
}

#[test]
fn rename_replaces_the_title_and_sticks() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let id = store.create().unwrap();
    store.append(&id, NewMessage::user("hi")).unwrap();

    store.rename(&id, "Netrunning 101").unwrap();
    store.append(&id, NewMessage::user("another")).unwrap();

    assert_eq!(
        store.load(&id).unwrap().summary.title.as_deref(),
        Some("Netrunning 101")
    );
}

#[test]
fn delete_removes_the_session_and_its_messages() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let keep = store.create().unwrap();
    let gone = store.create().unwrap();
    store
        .append(&gone, NewMessage::user("forget this"))
        .unwrap();

    store.delete(&gone).unwrap();

    assert_eq!(ids(&store), vec![keep]);
    assert!(matches!(store.load(&gone), Err(StoreError::NotFound(_))));
    // A fresh session reusing nothing still starts empty.
    let fresh = store.create().unwrap();
    assert!(store.load(&fresh).unwrap().messages.is_empty());
}

#[test]
fn operations_on_unknown_sessions_are_not_found() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let ghost: SessionId = "01JC8ZZZZZZZZZZZZZZZZZZZZZ".parse().unwrap();

    assert!(matches!(store.load(&ghost), Err(StoreError::NotFound(_))));
    assert!(matches!(
        store.append(&ghost, NewMessage::user("x")),
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        store.rename(&ghost, "x"),
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(store.delete(&ghost), Err(StoreError::NotFound(_))));
}

#[test]
fn resolve_finds_a_session_by_unique_case_insensitive_prefix() {
    let db = temp_db();
    let store = SessionStore::open(&db.path).unwrap();
    let a = store.create().unwrap();
    let b = store.create().unwrap();
    let (a_text, b_text) = (a.to_string(), b.to_string());
    // Length of the shared prefix (both were created this millisecond or the next).
    let shared = a_text
        .chars()
        .zip(b_text.chars())
        .take_while(|(x, y)| x == y)
        .count();

    let unique_a = a_text[..shared + 1].to_lowercase();
    assert_eq!(store.resolve(&unique_a).unwrap(), a);
    assert_eq!(store.resolve(&b_text).unwrap(), b);

    let err = store.resolve(&a_text[..shared]).unwrap_err().to_string();
    assert!(err.contains("matches 2 sessions"), "{err}");

    assert!(matches!(
        store.resolve("ZZZZ"),
        Err(StoreError::NotFound(_))
    ));
}
