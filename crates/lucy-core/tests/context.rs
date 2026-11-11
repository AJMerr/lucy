//! Long chats that fit: near the model's context size, older turns are replaced by a summary
//! (recent ones stay word for word); `/clear` starts her afresh and `/compress` summarises now,
//! both leaving the chat visible. Also: `/model --default`, and finding and trying models for
//! the add-a-model flow.

use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{ChatSendResult, Event, Session, method};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

fn says(text: &str) -> FakeResponse {
    let chunk = |delta: Value, finish: Option<&str>| {
        format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        )
    };
    FakeResponse::sse(
        chunk(json!({"content": text}), None)
            + &chunk(json!({}), Some("stop"))
            + "data: [DONE]\n\n",
    )
}

struct Harness {
    _dir: tempfile::TempDir,
    endpoint: Endpoint,
    model: FakeServer,
}

impl Harness {
    /// A core whose `chat` profile (on a fake server giving `replies`) has `extra` profile lines.
    async fn start(replies: Vec<FakeResponse>, profile_extra: &str, config_extra: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let model = FakeServer::start(replies).await;
        // A known size, so nothing asks the fake server for one.
        let profile_extra = if profile_extra.contains("context_tokens") {
            profile_extra.to_owned()
        } else {
            format!("{profile_extra}\ncontext_tokens = 100000")
        };
        // A one-line persona, so the prompt's size is mostly the chat.
        let personas = dir.path().join("personas");
        std::fs::create_dir_all(&personas).unwrap();
        std::fs::write(personas.join("lucy.md"), "You are Lucy.").unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"chat\"\n\n\
                 [profiles.chat]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"m\"\ntools = false\n{profile_extra}\n\
                 [profiles.other]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"o\"\n\n{config_extra}",
                model.url(),
                model.url(),
            ),
        )
        .unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-context-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("lucy.sqlite"),
            personas_dir: Some(personas),
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
            clock: None,
            notifier: None,
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
            model,
        }
    }

    async fn client(&self) -> (Client, UnboundedReceiver<Event>) {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
        let events = client.take_events().unwrap();
        (client, events)
    }

    async fn send(
        &self,
        client: &Client,
        events: &mut UnboundedReceiver<Event>,
        session: Option<&str>,
        text: &str,
    ) -> String {
        let params = match session {
            Some(id) => json!({"session_id": id, "text": text}),
            None => json!({"text": text}),
        };
        let sent: ChatSendResult = client.call(method::CHAT_SEND, params).await.unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
                .await
                .expect("timed out")
                .expect("closed");
            if matches!(&event, Event::TurnFinished { turn_id, .. } if *turn_id == sent.turn_id) {
                return sent.session_id;
            }
        }
    }

    /// Everything in the model's `n`th request, as text.
    fn request(&self, n: usize) -> String {
        self.model.requests()[n].body["messages"].to_string()
    }
}

/// About `n` thousand characters of distinct words starting with `tag`.
fn long(tag: &str, n: usize) -> String {
    (0..n * 125).map(|i| format!("{tag}{i} ")).collect()
}

#[tokio::test]
async fn near_the_context_size_older_turns_become_a_summary() {
    let harness = Harness::start(
        vec![
            says("First answer."),
            says("SUMMARY: they asked about apples and pears."),
            says("Second answer."),
        ],
        "context_tokens = 500",
        "",
    )
    .await;
    let (client, mut events) = harness.client().await;

    // About 350 tokens, then a short one: together past three quarters of 500.
    let session = harness
        .send(
            &client,
            &mut events,
            None,
            &format!("apples and pears {}", long("aa", 2)),
        )
        .await;
    harness
        .send(&client, &mut events, Some(&session), "and plums?")
        .await;

    let summarising = harness.request(1);
    assert!(summarising.contains("Summarise"), "{summarising}");
    assert!(summarising.contains("aa0"), "the oldest turn is summarised");
    let replying = harness.request(2);
    assert!(
        replying.contains("SUMMARY: they asked about apples and pears."),
        "{replying}"
    );
    assert!(
        !replying.contains("aa0"),
        "the oldest turn isn't sent word for word"
    );
    assert!(replying.contains("and plums?"));
    // The chat itself still has everything.
    let chat: Session = client
        .call(method::SESSIONS_LOAD, json!({"session_id": session}))
        .await
        .unwrap();
    assert!(chat.messages.iter().any(|m| m.content.contains("aa0")));
    assert!(
        chat.messages
            .iter()
            .any(|m| m.marker.as_deref() == Some("summary"))
    );
}

#[tokio::test]
async fn a_small_chat_is_sent_as_it_is() {
    let harness = Harness::start(
        vec![says("Hi."), says("Hello again.")],
        "context_tokens = 1000",
        "",
    )
    .await;
    let (client, mut events) = harness.client().await;

    let session = harness.send(&client, &mut events, None, "hi").await;
    harness
        .send(&client, &mut events, Some(&session), "hello")
        .await;

    assert_eq!(harness.model.requests().len(), 2, "no summary needed");
    assert!(harness.request(1).contains("hi"));
}

#[tokio::test]
async fn clear_starts_her_afresh_but_keeps_the_chat() {
    let harness = Harness::start(vec![says("Noted."), says("Fresh start.")], "", "").await;
    let (client, mut events) = harness.client().await;
    let session = harness
        .send(&client, &mut events, None, "my code word is heron")
        .await;

    let _: Value = client
        .call(method::CHAT_CLEAR, json!({"session_id": session}))
        .await
        .unwrap();
    harness
        .send(&client, &mut events, Some(&session), "what's my code word?")
        .await;

    assert!(
        !harness.request(1).contains("heron"),
        "{}",
        harness.request(1)
    );
    let chat: Session = client
        .call(method::SESSIONS_LOAD, json!({"session_id": session}))
        .await
        .unwrap();
    assert!(
        chat.messages.iter().any(|m| m.content.contains("heron")),
        "still shown"
    );
    assert!(
        chat.messages
            .iter()
            .any(|m| m.marker.as_deref() == Some("cleared"))
    );
}

#[tokio::test]
async fn compress_summarises_now() {
    let harness = Harness::start(
        vec![
            says("Noted."),
            says("SUMMARY: code word heron."),
            says("It's heron."),
        ],
        "",
        "",
    )
    .await;
    let (client, mut events) = harness.client().await;
    let session = harness
        .send(&client, &mut events, None, "my code word is heron")
        .await;

    let compressed: Value = client
        .call(method::CHAT_COMPRESS, json!({"session_id": session}))
        .await
        .unwrap();
    harness
        .send(&client, &mut events, Some(&session), "what's my code word?")
        .await;

    assert_eq!(compressed["summary"], "SUMMARY: code word heron.");
    let replying = harness.request(2);
    assert!(replying.contains("SUMMARY: code word heron."), "{replying}");
    assert!(!replying.contains("my code word is heron"), "{replying}");
}

#[tokio::test]
async fn model_with_default_changes_new_chats_too() {
    let harness = Harness::start(vec![says("Hi.")], "", "").await;
    let (client, mut events) = harness.client().await;
    let session = harness.send(&client, &mut events, None, "hi").await;

    let _: Value = client
        .call(
            method::SESSION_SET_PROFILE,
            json!({"session_id": session, "profile": "other", "make_default": true}),
        )
        .await
        .unwrap();

    let config: Value = client.call(method::CONFIG_GET, json!({})).await.unwrap();
    assert_eq!(config["config"]["defaults"]["profile"], "other");
}

#[tokio::test]
async fn models_can_be_listed_and_tried_before_a_profile_exists() {
    let ollama = FakeServer::start([
        FakeResponse::json(200, json!({"models": [{"name": "qwen3:14b"}]}).to_string()),
        says("OK"),
    ])
    .await;
    let harness = Harness::start(vec![], "", "").await;
    let (client, _events) = harness.client().await;
    let base_url = format!("{}/v1", ollama.url());

    let models: Value = client
        .call(
            method::MODELS_LIST,
            json!({"provider": "openai_compat", "base_url": base_url}),
        )
        .await
        .unwrap();
    let tried: Value = client
        .call(
            method::MODELS_TRY,
            json!({"provider": "openai_compat", "base_url": base_url, "model": "qwen3:14b"}),
        )
        .await
        .unwrap();

    assert_eq!(models[0]["id"], "qwen3:14b");
    assert_eq!(tried["reply"], "OK");
    assert_eq!(ollama.requests()[1].body["model"], "qwen3:14b");
}
