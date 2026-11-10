use std::path::PathBuf;
use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{
    BusyChat, CancelResult, ChatSendParams, ChatSendResult, Event, ProfileInfo, RenameParams,
    ResolveParams, Session, SessionParams, SessionSettings, SessionSummary, SetPlainParams,
    SetProfileParams, TurnOutcome,
};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedReceiver;

struct Harness {
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    /// The default profile's model server.
    model: FakeServer,
    /// The `other` profile's model server.
    other: FakeServer,
    core: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn start(responses: Vec<FakeResponse>) -> Self {
        Self::start_with(responses, vec![], Some(Duration::from_secs(3600))).await
    }

    async fn start_with(
        responses: Vec<FakeResponse>,
        other_responses: Vec<FakeResponse>,
        idle_timeout: Option<Duration>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeServer::start(responses).await;
        let other = FakeServer::start(other_responses).await;
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"fake\"\n\n\
                 [profiles.fake]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n\
                 [profiles.other]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"other-model\"\n\n\
                 [profiles.keyed]\nprovider = \"anthropic\"\nbase_url = \"{}\"\nmodel = \"claude-x\"\nkey_name = \"anthropic\"\n",
                model.url(),
                other.url(),
                other.url()
            ),
        )
        .unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-test-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("run/core.sock"))
        };
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("data/lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout,
            token_store: None,
        };
        let core = tokio::spawn(async move { serve(options).await.unwrap() });
        let harness = Self {
            dir,
            endpoint,
            model,
            other,
            core,
        };
        harness.wait_until_listening().await;
        harness
    }

    async fn wait_until_listening(&self) {
        for _ in 0..200 {
            if Stream::connect(self.endpoint.name().unwrap()).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("core never started listening");
    }

    async fn client(&self) -> Client {
        Client::connect(&self.endpoint, "test").await.unwrap()
    }

    #[allow(dead_code)]
    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }
}

/// Send raw JSON lines and collect one reply per line, bypassing the client's handshake.
async fn raw_exchange(endpoint: &Endpoint, lines: &[Value]) -> Vec<Value> {
    let stream = Stream::connect(endpoint.name().unwrap()).await.unwrap();
    let mut reader = BufReader::new(&stream);
    let mut replies = Vec::new();
    for line in lines {
        (&stream)
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        let mut reply = String::new();
        reader.read_line(&mut reply).await.unwrap();
        replies.push(serde_json::from_str(&reply).unwrap());
    }
    replies
}

#[tokio::test]
async fn hello_reports_the_core_version_and_pid() {
    let harness = Harness::start(vec![]).await;

    let client = harness.client().await;

    assert_eq!(client.core_version(), env!("CARGO_PKG_VERSION"));
    assert_eq!(client.core_pid(), std::process::id());
}

#[tokio::test]
async fn mismatched_protocol_version_is_refused() {
    let harness = Harness::start(vec![]).await;

    let replies = raw_exchange(
        &harness.endpoint,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "hello",
                 "params": {"protocol_version": 999, "client": "future"}})],
    )
    .await;

    let message = replies[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("protocol v999"), "{message}");
}

#[tokio::test]
async fn methods_before_hello_are_refused() {
    let harness = Harness::start(vec![]).await;

    let replies = raw_exchange(
        &harness.endpoint,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "sessions.list"})],
    )
    .await;

    let message = replies[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("hello"), "{message}");
}

/// An OpenAI-compatible stream that replies with `parts`.
fn reply(parts: &[&str]) -> FakeResponse {
    let mut body = String::new();
    for part in parts {
        body += &format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": part}, "finish_reason": null}]})
        );
    }
    body += "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    FakeResponse::sse(body)
}

async fn send(client: &Client, session: Option<&str>, text: &str) -> ChatSendResult {
    client
        .call(
            "chat.send",
            ChatSendParams {
                images: Vec::new(),
                session_id: session.map(Into::into),
                text: text.into(),
                cwd: None,
            },
        )
        .await
        .unwrap()
}

/// Events for `turn_id` (and the user message before it), through `TurnFinished`.
async fn turn_events(events: &mut UnboundedReceiver<Event>, turn_id: u64) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timed out waiting for events")
            .expect("event stream closed");
        let finished = matches!(&event, Event::TurnFinished { turn_id: t, .. } if *t == turn_id);
        seen.push(event);
        if finished {
            return seen;
        }
    }
}

fn deltas(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Delta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn outcome(events: &[Event]) -> (TurnOutcome, Option<String>) {
    match events.last() {
        Some(Event::TurnFinished { outcome, error, .. }) => (*outcome, error.clone()),
        other => panic!("expected TurnFinished, got {other:?}"),
    }
}

async fn load(client: &Client, session_id: &str) -> Session {
    client
        .call(
            "sessions.load",
            SessionParams {
                session_id: session_id.into(),
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn chat_streams_the_reply_and_saves_both_messages() {
    let harness = Harness::start(vec![reply(&["Wake up,", " choom."])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();

    let sent = send(&client, None, "hi").await;
    let seen = turn_events(&mut events, sent.turn_id).await;

    assert_eq!(deltas(&seen), vec!["Wake up,", " choom."]);
    assert_eq!(outcome(&seen), (TurnOutcome::Complete, None));
    assert!(matches!(&seen[0], Event::UserMessage { text, .. } if text == "hi"));
    assert!(seen.iter().any(|e| matches!(e,
        Event::TurnStarted { profile, model, .. } if profile == "fake" && model == "fake-model")));

    let session = load(&client, &sent.session_id).await;
    let saved: Vec<_> = session
        .messages
        .iter()
        .map(|m| (m.content.as_str(), m.model.as_deref(), m.incomplete))
        .collect();
    assert_eq!(
        saved,
        vec![("hi", None), ("Wake up, choom.", Some("fake-model"))]
            .into_iter()
            .map(|(c, m)| (c, m, false))
            .collect::<Vec<_>>()
    );

    let sent_to_model = &harness.model.requests()[0].body["messages"];
    assert_eq!(sent_to_model[0]["role"], "system");
    assert!(
        sent_to_model[0]["content"]
            .as_str()
            .unwrap()
            .contains("You are Lucy")
    );
    assert_eq!(sent_to_model[1], json!({"role": "user", "content": "hi"}));
}

#[tokio::test]
async fn follow_up_sends_the_whole_conversation() {
    let harness = Harness::start(vec![reply(&["First."]), reply(&["Second."])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();

    let first = send(&client, None, "one").await;
    turn_events(&mut events, first.turn_id).await;
    let second = send(&client, Some(&first.session_id), "two").await;
    turn_events(&mut events, second.turn_id).await;

    assert_eq!(second.session_id, first.session_id);
    let roles_and_text: Vec<(String, String)> = harness.model.requests()[1].body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1) // system prompt
        .map(|m| {
            (
                m["role"].as_str().unwrap().into(),
                m["content"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        roles_and_text,
        vec![
            ("user".into(), "one".into()),
            ("assistant".into(), "First.".into()),
            ("user".into(), "two".into()),
        ]
    );
}

#[tokio::test]
async fn sessions_can_be_listed_resolved_renamed_and_deleted() {
    let harness = Harness::start(vec![reply(&["ok"])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let sent = send(&client, None, "remember the milk").await;
    turn_events(&mut events, sent.turn_id).await;

    let listed: Vec<SessionSummary> = client.call("sessions.list", json!({})).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, sent.session_id);
    assert_eq!(listed[0].title.as_deref(), Some("remember the milk"));
    assert_eq!(listed[0].message_count, 2);

    let prefix = sent.session_id[..20].to_lowercase();
    let resolved: String = client
        .call("sessions.resolve", ResolveParams { prefix })
        .await
        .unwrap();
    assert_eq!(resolved, sent.session_id);

    let _: Value = client
        .call(
            "sessions.rename",
            RenameParams {
                session_id: sent.session_id.clone(),
                title: "Groceries".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        load(&client, &sent.session_id)
            .await
            .summary
            .title
            .as_deref(),
        Some("Groceries")
    );

    let _: Value = client
        .call(
            "sessions.delete",
            SessionParams {
                session_id: sent.session_id.clone(),
            },
        )
        .await
        .unwrap();
    let err = client
        .call::<_, Session>(
            "sessions.load",
            SessionParams {
                session_id: sent.session_id,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no session matching"), "{err}");
}

#[tokio::test]
async fn profiles_are_listed_from_config() {
    let harness = Harness::start(vec![]).await;
    let client = harness.client().await;

    let profiles: Vec<ProfileInfo> = client.call("profiles.list", json!({})).await.unwrap();

    let summary: Vec<_> = profiles
        .iter()
        .map(|p| (p.name.as_str(), p.model.as_str(), p.is_default))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("fake", "fake-model", true),
            ("keyed", "claude-x", false),
            ("other", "other-model", false)
        ]
    );
}

#[tokio::test]
async fn set_profile_switches_the_model_for_that_session_only() {
    let harness = Harness::start_with(
        vec![reply(&["from fake"]), reply(&["fake again"])],
        vec![reply(&["from other"])],
        Some(Duration::from_secs(3600)),
    )
    .await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let first = send(&client, None, "hi").await;
    turn_events(&mut events, first.turn_id).await;

    let settings: SessionSettings = client
        .call(
            "session.set_profile",
            SetProfileParams {
                session_id: first.session_id.clone(),
                profile: "other".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        settings,
        SessionSettings {
            profile: "other".into(),
            plain: false,
            mode: lucy_protocol::Mode::Assist,
            level: lucy_protocol::AgentLevel::Guarded,
        }
    );

    let switched = send(&client, Some(&first.session_id), "and now?").await;
    let seen = turn_events(&mut events, switched.turn_id).await;
    assert_eq!(deltas(&seen), vec!["from other"]);

    // A different session still uses the default.
    let fresh = send(&client, None, "new chat").await;
    let seen = turn_events(&mut events, fresh.turn_id).await;
    assert_eq!(deltas(&seen), vec!["fake again"]);
    assert_eq!(harness.other.requests().len(), 1);

    let err = client
        .call::<_, SessionSettings>(
            "session.set_profile",
            SetProfileParams {
                session_id: first.session_id,
                profile: "nope".into(),
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no profile named `nope`"), "{err}");
}

#[tokio::test]
async fn set_plain_drops_the_voice_from_the_system_prompt() {
    let harness = Harness::start(vec![reply(&["one"]), reply(&["two"])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let first = send(&client, None, "hi").await;
    turn_events(&mut events, first.turn_id).await;

    let settings: SessionSettings = client
        .call(
            "session.set_plain",
            SetPlainParams {
                session_id: first.session_id.clone(),
                plain: true,
            },
        )
        .await
        .unwrap();
    assert!(settings.plain);
    let again: SessionSettings = client
        .call(
            "session.settings",
            SessionParams {
                session_id: first.session_id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(again, settings);

    let second = send(&client, Some(&first.session_id), "plain please").await;
    turn_events(&mut events, second.turn_id).await;

    let system = |i: usize| {
        harness.model.requests()[i].body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(system(0).contains("## Who you are"));
    assert!(!system(1).contains("## Who you are"));
}

/// A reply that streams `parts` and then never finishes.
fn stalled(parts: &[&str]) -> FakeResponse {
    let mut body = String::new();
    for part in parts {
        body += &format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": part}, "finish_reason": null}]})
        );
    }
    FakeResponse::sse(body).stall()
}

#[tokio::test]
async fn subscribers_see_other_clients_chats_and_others_do_not() {
    let harness = Harness::start(vec![reply(&["Hey."])]).await;
    let mut watcher = harness.client().await;
    let mut watched = watcher.take_events().unwrap();
    let _: Value = watcher.call("subscribe", json!({})).await.unwrap();
    let mut bystander = harness.client().await;
    let mut unwatched = bystander.take_events().unwrap();
    let chatter = harness.client().await;

    let sent = send(&chatter, None, "hello?").await;
    let seen = turn_events(&mut watched, sent.turn_id).await;

    assert!(matches!(&seen[0], Event::UserMessage { text, .. } if text == "hello?"));
    assert_eq!(deltas(&seen), vec!["Hey."]);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), unwatched.recv())
            .await
            .is_err(),
        "a client that neither subscribed nor chatted got an event"
    );
}

#[tokio::test]
async fn cancel_stops_the_reply_and_keeps_the_partial_text() {
    let harness = Harness::start(vec![stalled(&["Once upon", " a time"])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let sent = send(&client, None, "story?").await;

    // Wait until both parts have streamed.
    let mut streamed = String::new();
    while streamed != "Once upon a time" {
        if let Some(Event::Delta { text, .. }) = events.recv().await {
            streamed += &text;
        }
    }
    let busy = client
        .call::<_, ChatSendResult>(
            "chat.send",
            ChatSendParams {
                images: Vec::new(),
                session_id: Some(sent.session_id.clone()),
                text: "hurry".into(),
                cwd: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(busy.contains("still replying"), "{busy}");

    let cancel: CancelResult = client
        .call(
            "chat.cancel",
            SessionParams {
                session_id: sent.session_id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(cancel.cancelled);
    let seen = turn_events(&mut events, sent.turn_id).await;
    assert_eq!(outcome(&seen), (TurnOutcome::Cancelled, None));

    let session = load(&client, &sent.session_id).await;
    let last = session.messages.last().unwrap();
    assert_eq!(
        (last.content.as_str(), last.incomplete),
        ("Once upon a time", true)
    );

    let again: CancelResult = client
        .call(
            "chat.cancel",
            SessionParams {
                session_id: sent.session_id,
            },
        )
        .await
        .unwrap();
    assert!(!again.cancelled);
}

#[tokio::test]
async fn provider_failure_finishes_the_turn_with_the_error() {
    let harness = Harness::start(vec![FakeResponse::json(
        500,
        r#"{"error":{"message":"GPU on fire"}}"#,
    )])
    .await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();

    let sent = send(&client, None, "hi").await;
    let seen = turn_events(&mut events, sent.turn_id).await;

    let (result, error) = outcome(&seen);
    assert_eq!(result, TurnOutcome::Failed);
    assert!(error.unwrap().contains("GPU on fire"));
    let session = load(&client, &sent.session_id).await;
    assert_eq!(session.messages.len(), 1, "only the user message is kept");
}

#[tokio::test]
async fn missing_api_key_is_reported_before_anything_is_saved() {
    let harness = Harness::start(vec![reply(&["ok"])]).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let first = send(&client, None, "hi").await;
    turn_events(&mut events, first.turn_id).await;
    let _: SessionSettings = client
        .call(
            "session.set_profile",
            SetProfileParams {
                session_id: first.session_id.clone(),
                profile: "keyed".into(),
            },
        )
        .await
        .unwrap();

    let err = client
        .call::<_, ChatSendResult>(
            "chat.send",
            ChatSendParams {
                images: Vec::new(),
                session_id: Some(first.session_id.clone()),
                text: "again".into(),
                cwd: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("lucy auth set anthropic"), "{err}");
    assert_eq!(load(&client, &first.session_id).await.messages.len(), 2);
}

#[tokio::test]
async fn core_exits_once_idle_and_removes_its_socket() {
    let harness = Harness::start_with(vec![], vec![], Some(Duration::from_millis(300))).await;
    let client = harness.client().await;

    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        !harness.core.is_finished(),
        "exited while a client was connected"
    );

    drop(client);
    let exited = tokio::time::timeout(Duration::from_secs(3), harness.core).await;
    assert!(exited.is_ok(), "core did not exit after going idle");
    if let Endpoint::Path(socket) = &harness.endpoint {
        assert!(!socket.exists(), "socket file left behind");
    }
}

#[tokio::test]
async fn a_reply_in_progress_keeps_the_core_awake() {
    let harness = Harness::start_with(
        vec![stalled(&["thinking"])],
        vec![],
        Some(Duration::from_millis(300)),
    )
    .await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    send(&client, None, "long question").await;
    events.recv().await; // the turn has started

    drop(client);
    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(!harness.core.is_finished(), "exited mid-reply");
}

#[tokio::test]
async fn sessions_create_makes_an_empty_session_that_settings_can_target() {
    let harness = Harness::start(vec![]).await;
    let client = harness.client().await;

    let id: String = client.call("sessions.create", json!({})).await.unwrap();

    assert_eq!(load(&client, &id).await.messages.len(), 0);
    let settings: SessionSettings = client
        .call(
            "session.set_plain",
            SetPlainParams {
                session_id: id,
                plain: true,
            },
        )
        .await
        .unwrap();
    assert!(settings.plain);
}

#[tokio::test]
async fn status_answers_before_hello() {
    let harness = Harness::start(vec![]).await;

    let replies = raw_exchange(
        &harness.endpoint,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "core.status"})],
    )
    .await;

    let status = &replies[0]["result"];
    assert_eq!(
        status["protocol_version"],
        lucy_protocol::rpc::PROTOCOL_VERSION
    );
    assert_eq!(status["core_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["pid"], std::process::id());
    assert_eq!(status["service"], false);
    assert_eq!(status["busy"], json!([]));
}

#[tokio::test]
async fn a_service_core_never_goes_idle() {
    let harness = Harness::start_with(vec![], vec![], None).await;
    drop(harness.client().await);

    tokio::time::sleep(Duration::from_millis(700)).await;

    assert!(
        !harness.core.is_finished(),
        "a service core exited when idle"
    );
    let status = Client::status(&harness.endpoint).await.unwrap();
    assert!(status.service);
}

/// Wait for the core task to end, then check it cleaned up its socket.
async fn assert_core_exits(harness: Harness) {
    let exited = tokio::time::timeout(Duration::from_secs(3), harness.core).await;
    assert!(exited.is_ok(), "core did not exit");
    if let Endpoint::Path(socket) = &harness.endpoint {
        assert!(!socket.exists(), "socket file left behind");
    }
}

#[tokio::test]
async fn shutdown_stops_an_idle_core() {
    let harness = Harness::start_with(vec![], vec![], None).await;

    let result = Client::shutdown(&harness.endpoint, false).await.unwrap();

    assert!(result.stopping);
    assert_core_exits(harness).await;
}

#[tokio::test]
async fn shutdown_waits_for_replies_unless_forced() {
    let harness = Harness::start_with(vec![stalled(&["Once upon"])], vec![], None).await;
    let mut client = harness.client().await;
    let mut events = client.take_events().unwrap();
    let sent = send(&client, None, "tell me a story").await;
    while !matches!(events.recv().await, Some(Event::Delta { .. })) {}

    let refused = Client::shutdown(&harness.endpoint, false).await.unwrap();
    assert!(!refused.stopping);
    assert_eq!(
        refused.busy,
        vec![BusyChat {
            session_id: sent.session_id.clone(),
            title: Some("tell me a story".into()),
        }]
    );
    let status = Client::status(&harness.endpoint).await.unwrap();
    assert_eq!(status.busy, refused.busy, "still running and still busy");

    let forced = Client::shutdown(&harness.endpoint, true).await.unwrap();
    assert!(forced.stopping);
    let seen = turn_events(&mut events, sent.turn_id).await;
    assert_eq!(outcome(&seen), (TurnOutcome::Cancelled, None));
    drop(client);
    assert_core_exits(harness).await;
}
