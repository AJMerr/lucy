//! Scheduled tasks: reminders (just a message at a time) and tasks (a prompt she runs as an
//! agent turn at its own level), created by asking her (always with an approval card), run by
//! the core on its clock, with results in a Tasks chat and a notification.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Local, TimeZone};
use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::schedule::{FakeClock, Notifier};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{
    ApproveParams, ChatSendResult, Decision, Event, Session, SessionSummary, method,
};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}

fn says(text: &str) -> FakeResponse {
    FakeResponse::sse(
        chunk(json!({"content": text}), None)
            + &chunk(json!({}), Some("stop"))
            + "data: [DONE]\n\n",
    )
}

fn calls(name: &str, arguments: Value) -> FakeResponse {
    FakeResponse::sse(
        chunk(
            json!({"tool_calls": [{"index": 0, "id": format!("call_{name}"), "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}}]}),
            None,
        ) + &chunk(json!({}), Some("tool_calls"))
            + "data: [DONE]\n\n",
    )
}

/// 07:00 local time on 10 November 2026.
fn morning() -> chrono::DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 10, 7, 0, 0).unwrap()
}

/// OS notifications the core sent, as (title, body).
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String)>>);

impl Notifier for Recorder {
    fn notify(&self, title: &str, body: &str) {
        self.0
            .lock()
            .unwrap()
            .push((title.to_owned(), body.to_owned()));
    }
}

struct Harness {
    /// Keeps the data folder until the harness goes.
    _dir: Arc<tempfile::TempDir>,
    endpoint: Endpoint,
    clock: Arc<FakeClock>,
    notifications: Arc<Recorder>,
    chat_model: FakeServer,
}

impl Harness {
    async fn start(replies: Vec<FakeResponse>) -> Self {
        Self::start_in(Arc::new(tempfile::tempdir().unwrap()), replies, morning()).await
    }

    async fn start_in(
        dir: Arc<tempfile::TempDir>,
        replies: Vec<FakeResponse>,
        now: chrono::DateTime<Local>,
    ) -> Self {
        let chat_model = FakeServer::start(replies).await;
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"chat\"\n\n\
                 [profiles.chat]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"m\"\n",
                chat_model.url(),
            ),
        )
        .unwrap();
        let clock = Arc::new(FakeClock::new(now));
        let notifications = Arc::new(Recorder::default());
        let endpoint = endpoint(dir.path());
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("data/lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
            clock: Some(clock.clone()),
            notifier: Some(notifications.clone()),
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Stream::connect(endpoint.name().unwrap()).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            _dir: dir,
            endpoint,
            clock,
            notifications,
            chat_model,
        }
    }

    async fn client(&self) -> (Client, UnboundedReceiver<Event>) {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
        let events = client.take_events().unwrap();
        (client, events)
    }

    /// Ask her something, answering approvals with `answer`, until the reply finishes.
    async fn ask(
        &self,
        client: &Client,
        events: &mut UnboundedReceiver<Event>,
        text: &str,
        answer: Decision,
    ) -> Vec<Event> {
        let sent: ChatSendResult = client
            .call(method::CHAT_SEND, json!({"text": text}))
            .await
            .unwrap();
        let mut seen = Vec::new();
        loop {
            let event = next(events).await;
            if let Event::ToolApproval { call, .. } = &event {
                let _: Value = client
                    .call(
                        method::TOOL_APPROVE,
                        ApproveParams {
                            session_id: sent.session_id.clone(),
                            call_id: call.id.clone(),
                            decision: answer,
                        },
                    )
                    .await
                    .unwrap();
            }
            let done =
                matches!(&event, Event::TurnFinished { turn_id, .. } if *turn_id == sent.turn_id);
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    /// Move the clock to `hour`:`minute` today and collect events until a notification.
    async fn at(
        &self,
        events: &mut UnboundedReceiver<Event>,
        hour: u32,
        minute: u32,
    ) -> Vec<Event> {
        let when = Local
            .with_ymd_and_hms(2026, 11, 10, hour, minute, 0)
            .unwrap();
        self.clock.set(when);
        until_notification(events).await
    }

    async fn tasks_chat(&self, client: &Client) -> Session {
        let listed: Vec<SessionSummary> =
            client.call(method::SESSIONS_LIST, json!({})).await.unwrap();
        let tasks = listed
            .iter()
            .find(|s| s.title.as_deref() == Some("Tasks"))
            .expect("a Tasks chat");
        client
            .call(method::SESSIONS_LOAD, json!({"session_id": tasks.id}))
            .await
            .unwrap()
    }
}

fn endpoint(dir: &Path) -> Endpoint {
    if cfg!(windows) {
        let unique = dir.file_name().unwrap().to_string_lossy();
        Endpoint::Pipe(format!("lucy-schedules-{unique}"))
    } else {
        Endpoint::Path(dir.join("core.sock"))
    }
}

async fn next(events: &mut UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(20), events.recv())
        .await
        .expect("timed out")
        .expect("connection closed")
}

async fn until_notification(events: &mut UnboundedReceiver<Event>) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = next(events).await;
        let done = matches!(event, Event::Notification { .. });
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn approval(events: &[Event]) -> Option<(String, String)> {
    events.iter().find_map(|e| match e {
        Event::ToolApproval {
            reason, preview, ..
        } => Some((reason.clone(), preview.clone().unwrap_or_default())),
        _ => None,
    })
}

fn tool_results(events: &[Event]) -> Vec<(String, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToolFinished {
                output, is_error, ..
            } => Some((output.clone(), *is_error)),
            _ => None,
        })
        .collect()
}

fn notification(events: &[Event]) -> (String, String) {
    events
        .iter()
        .find_map(|e| match e {
            Event::Notification { title, body, .. } => Some((title.clone(), body.clone())),
            _ => None,
        })
        .expect("a notification")
}

// --- Reminders ----------------------------------------------------------------------------------

#[tokio::test]
async fn a_reminder_asks_first_then_fires_on_time() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "reminder", "what": "Call Mom", "when": {"at": "2026-11-10T17:00"}}),
        ),
        says("I'll remind you at 5."),
    ])
    .await;
    let (client, mut events) = harness.client().await;

    let seen = harness
        .ask(
            &client,
            &mut events,
            "remind me at 5 to call mom",
            Decision::Once,
        )
        .await;

    let (reason, preview) = approval(&seen).expect("scheduling always asks");
    assert!(reason.contains("reminder"), "{reason}");
    assert!(preview.contains("Call Mom"), "{preview}");
    assert!(preview.contains("17:00"), "{preview}");
    let (result, is_error) = tool_results(&seen).remove(0);
    assert!(!is_error, "{result}");
    assert!(
        result.contains("lucy service install"),
        "an on-demand core offers the login service: {result}"
    );
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);
    assert_eq!(listed["always_on"], false);

    let fired = harness.at(&mut events, 17, 0).await;

    let (title, body) = notification(&fired);
    assert!(title.contains("Reminder"), "{title}");
    assert_eq!(body, "Call Mom");
    let tasks = harness.tasks_chat(&client).await;
    assert!(
        tasks
            .messages
            .iter()
            .any(|m| m.content.contains("Call Mom"))
    );
    // Nobody said they'd show notifications, so the core did.
    assert_eq!(
        harness.notifications.0.lock().unwrap().clone(),
        vec![(title, body)]
    );
    // A one-off reminder is done once it's fired.
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    assert!(
        listed["schedules"].as_array().unwrap().is_empty(),
        "{listed}"
    );
    assert_eq!(
        harness.chat_model.requests().len(),
        2,
        "no model runs for a reminder"
    );
}

#[tokio::test]
async fn declining_schedules_nothing() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "reminder", "what": "Call Mom", "when": {"at": "2026-11-10T17:00"}}),
        ),
        says("Okay."),
    ])
    .await;
    let (client, mut events) = harness.client().await;

    harness
        .ask(&client, &mut events, "remind me", Decision::Deny)
        .await;

    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    assert!(listed["schedules"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn an_app_that_shows_notifications_stops_the_core_sending_its_own() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "reminder", "what": "Stretch", "when": {"at": "2026-11-10T09:00"}}),
        ),
        says("Okay."),
    ])
    .await;
    let mut app = Client::connect(&harness.endpoint, "lucy-desktop")
        .await
        .unwrap();
    let _: Value = app
        .call(method::SUBSCRIBE, json!({"notifications": true}))
        .await
        .unwrap();
    let mut app_events = app.take_events().unwrap();
    let (client, mut events) = harness.client().await;
    harness
        .ask(
            &client,
            &mut events,
            "remind me to stretch at 9",
            Decision::Once,
        )
        .await;

    harness.at(&mut events, 9, 0).await;
    until_notification(&mut app_events).await;

    assert!(harness.notifications.0.lock().unwrap().is_empty());
}

// --- Tasks --------------------------------------------------------------------------------------

#[tokio::test]
async fn a_task_runs_as_a_turn_in_the_tasks_chat_and_says_how_it_went() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "task", "title": "Morning summary",
                   "what": "Summarise my notes for today", "when": {"daily": "08:00"}, "level": 2}),
        ),
        says("Every morning at 8, then."),
        says("Three things today: dentist, groceries, call Maya."),
    ])
    .await;
    let (client, mut events) = harness.client().await;
    let seen = harness
        .ask(
            &client,
            &mut events,
            "summarise my notes every morning",
            Decision::Once,
        )
        .await;
    let (_, preview) = approval(&seen).unwrap();
    assert!(preview.contains("every day at 08:00"), "{preview}");
    assert!(preview.contains("level 2"), "{preview}");

    let ran = harness.at(&mut events, 8, 0).await;

    let (title, body) = notification(&ran);
    assert!(title.contains("Morning summary"), "{title}");
    assert!(body.contains("dentist"), "{body}");
    let tasks = harness.tasks_chat(&client).await;
    let said: Vec<&str> = tasks.messages.iter().map(|m| m.content.as_str()).collect();
    assert!(
        said.iter()
            .any(|m| m.contains("Summarise my notes for today")),
        "{said:?}"
    );
    assert!(said.iter().any(|m| m.contains("dentist")), "{said:?}");
    let system = harness.chat_model.requests()[2].body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        system.contains("level 2"),
        "runs at the task's level: {system}"
    );
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    let next = listed["schedules"][0]["next_run_ms"].as_i64().unwrap();
    let tomorrow_8 = Local.with_ymd_and_hms(2026, 11, 11, 8, 0, 0).unwrap();
    assert_eq!(next, tomorrow_8.timestamp_millis(), "repeats tomorrow");
}

#[tokio::test]
async fn a_call_the_tasks_level_doesnt_allow_is_refused_and_reported() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "task", "title": "Cleanup", "what": "Delete old logs",
                   "when": {"daily": "08:00"}, "level": 1}),
        ),
        says("Okay."),
        calls("run_command", json!({"command": "rm -rf ~/old-logs"})),
        says("I couldn't delete them: that needs your approval."),
    ])
    .await;
    let (client, mut events) = harness.client().await;
    harness
        .ask(&client, &mut events, "clean my logs daily", Decision::Once)
        .await;

    let ran = harness.at(&mut events, 8, 0).await;

    assert_eq!(approval(&ran), None, "nobody's there to ask");
    let (output, is_error) = tool_results(&ran).remove(0);
    assert!(is_error);
    assert!(output.contains("away"), "{output}");
    let (_, body) = notification(&ran);
    assert!(body.contains("couldn't delete"), "{body}");
}

#[tokio::test]
async fn a_run_missed_while_the_core_was_stopped_runs_once_when_it_starts() {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    {
        let harness = Harness::start_in(
            dir.clone(),
            vec![
                calls(
                    "schedule",
                    json!({"kind": "reminder", "what": "Water the plants",
                           "when": {"daily": "08:00"}}),
                ),
                says("Okay."),
            ],
            morning(),
        )
        .await;
        let (client, mut events) = harness.client().await;
        harness
            .ask(&client, &mut events, "remind me daily", Decision::Once)
            .await;
        let _: Value = client.call(method::CORE_SHUTDOWN, json!({})).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The computer was off past 08:00; the core starts again at 10:00.
    let later = Local.with_ymd_and_hms(2026, 11, 10, 10, 0, 0).unwrap();
    let harness = Harness::start_in(dir, vec![], later).await;
    let (client, _events) = harness.client().await;

    // It fires as the core starts, before anyone connects, so the core notifies by itself.
    let mut sent = Vec::new();
    for _ in 0..100 {
        sent = harness.notifications.0.lock().unwrap().clone();
        if !sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let bodies: Vec<&str> = sent.iter().map(|(_, body)| body.as_str()).collect();
    assert_eq!(
        bodies,
        ["Water the plants"],
        "once, not once per missed day"
    );
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    let tomorrow_8 = Local.with_ymd_and_hms(2026, 11, 11, 8, 0, 0).unwrap();
    assert_eq!(
        listed["schedules"][0]["next_run_ms"].as_i64().unwrap(),
        tomorrow_8.timestamp_millis(),
        "once, then on schedule"
    );
}

// --- Managing them ------------------------------------------------------------------------------

#[tokio::test]
async fn schedules_can_be_paused_changed_and_deleted() {
    let harness = Harness::start(vec![
        calls(
            "schedule",
            json!({"kind": "reminder", "what": "Stretch", "when": {"weekdays": "09:00"}}),
        ),
        says("Okay."),
    ])
    .await;
    let (client, mut events) = harness.client().await;
    harness
        .ask(&client, &mut events, "remind me to stretch", Decision::Once)
        .await;
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    let schedule = &listed["schedules"][0];
    assert_eq!(schedule["when"], "weekdays at 09:00");
    let id = schedule["id"].as_str().unwrap().to_owned();

    let changed: Value = client
        .call(
            method::SCHEDULES_UPDATE,
            json!({"id": id, "paused": true, "what": "Stretch your back"}),
        )
        .await
        .unwrap();
    assert_eq!(changed["paused"], true);
    assert_eq!(changed["what"], "Stretch your back");

    // Paused: 09:00 passes quietly.
    harness
        .clock
        .set(Local.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(harness.notifications.0.lock().unwrap().is_empty());

    let _: Value = client
        .call(method::SCHEDULES_DELETE, json!({"id": id}))
        .await
        .unwrap();
    let listed: Value = client
        .call(method::SCHEDULES_LIST, json!({}))
        .await
        .unwrap();
    assert!(listed["schedules"].as_array().unwrap().is_empty());
}
