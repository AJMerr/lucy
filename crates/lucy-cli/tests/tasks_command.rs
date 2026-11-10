//! `lucy tasks`: lists reminders and scheduled tasks, and pauses, resumes or deletes one.

use std::time::Duration;

use lucy_cli::tasks_command::{delete, list, set_paused};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_core::sessions::{Schedule, SessionStore};
use lucy_protocol::rpc::{SchedulesList, method};
use serde_json::json;

fn schedule(id: &str, kind: &str, title: &str, spec: &str) -> Schedule {
    Schedule {
        id: id.into(),
        kind: kind.into(),
        title: title.into(),
        what: format!("{title}, please"),
        spec: spec.into(),
        level: 2,
        profile: "fake".into(),
        paused: false,
        next_run: Some(4_102_444_800_000),
        last_run: None,
    }
}

async fn core(schedules: &[Schedule]) -> (tempfile::TempDir, Client) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[defaults]\nprofile = \"fake\"\n\n[profiles.fake]\nprovider = \"openai_compat\"\n\
         base_url = \"http://127.0.0.1:9/v1\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let db_path = dir.path().join("lucy.sqlite");
    {
        let store = SessionStore::open(&db_path).unwrap();
        for s in schedules {
            store.add_schedule(s).unwrap();
        }
    }
    let endpoint = if cfg!(windows) {
        let unique = dir.path().file_name().unwrap().to_string_lossy();
        Endpoint::Pipe(format!("lucy-tasks-cmd-{unique}"))
    } else {
        Endpoint::Path(dir.path().join("core.sock"))
    };
    let options = CoreOptions {
        endpoint: endpoint.clone(),
        config_path,
        db_path,
        personas_dir: None,
        secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
        idle_timeout: Some(Duration::from_secs(3600)),
        token_store: None,
        clock: None,
        notifier: None,
    };
    tokio::spawn(async move { serve(options).await.unwrap() });
    for _ in 0..200 {
        if let Ok(client) = Client::connect(&endpoint, "test").await {
            return (dir, client);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the core didn't start");
}

async fn schedules(client: &Client) -> SchedulesList {
    client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap()
}

#[tokio::test]
async fn lucy_tasks_lists_what_is_scheduled_and_when() {
    let (_dir, client) = core(&[
        schedule(
            "01AAA",
            "task",
            "Morning summary",
            r#"{"weekdays":"08:00"}"#,
        ),
        schedule(
            "01BBB",
            "reminder",
            "Call Mom",
            r#"{"at":"2099-01-01T17:00"}"#,
        ),
    ])
    .await;
    let mut out = Vec::new();

    list(&client, &mut out).await.unwrap();

    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("Morning summary"), "{out}");
    assert!(out.contains("weekdays at 08:00"), "{out}");
    assert!(out.contains("level 2"), "{out}");
    assert!(out.contains("Call Mom"), "{out}");
    assert!(
        out.contains("lucy service install"),
        "says tasks need the core running: {out}"
    );
}

#[tokio::test]
async fn nothing_scheduled_says_how_to_schedule() {
    let (_dir, client) = core(&[]).await;
    let mut out = Vec::new();

    list(&client, &mut out).await.unwrap();

    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("Nothing is scheduled")
    );
}

#[tokio::test]
async fn a_task_is_paused_resumed_and_deleted_by_title() {
    let (_dir, client) = core(&[schedule(
        "01AAA",
        "task",
        "Morning summary",
        r#"{"weekdays":"08:00"}"#,
    )])
    .await;
    let mut out = Vec::new();

    set_paused(&client, "morning", true, &mut out)
        .await
        .unwrap();
    assert!(schedules(&client).await.schedules[0].paused);
    set_paused(&client, "Morning summary", false, &mut out)
        .await
        .unwrap();
    assert!(!schedules(&client).await.schedules[0].paused);
    delete(&client, "01AAA", &mut out).await.unwrap();

    assert!(schedules(&client).await.schedules.is_empty());
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("Paused “Morning summary”"), "{out}");
    assert!(out.contains("Deleted “Morning summary”"), "{out}");
}

#[tokio::test]
async fn an_unknown_name_is_an_error() {
    let (_dir, client) = core(&[]).await;
    let mut out = Vec::new();

    let err = delete(&client, "nope", &mut out).await.unwrap_err();

    assert!(err.to_string().contains("nope"), "{err}");
}
