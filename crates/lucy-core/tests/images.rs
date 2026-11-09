//! Pictures in chats: what you attach is kept with the chat, sent to models that can see, and
//! described by a vision helper for models that can't.

use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{ChatSendResult, Event, Session, method};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

/// A tiny PNG, base64 (what a client sends).
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==";

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
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    chat_model: FakeServer,
    helper: FakeServer,
}

impl Harness {
    /// A core whose default profile `chat` can see or not (`vision`), with a helper profile
    /// `eyes` set as `[images] vision_profile` when `helper` is true.
    async fn start(vision: bool, helper: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let chat_model = FakeServer::start([says("Nice picture.")]).await;
        let helper_model = FakeServer::start([says("A red bicycle against a brick wall.")]).await;
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let mut config = format!(
            "[defaults]\nprofile = \"chat\"\n\n\
             [profiles.chat]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"m\"\nvision = {vision}\n\n\
             [profiles.eyes]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"seer\"\nvision = true\n",
            chat_model.url(),
            helper_model.url(),
        );
        if helper {
            config += "\n[images]\nvision_profile = \"eyes\"\n";
        }
        std::fs::write(&config_path, config).unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-images-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("data/lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Stream::connect(endpoint.name().unwrap()).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            dir,
            endpoint,
            chat_model,
            helper: helper_model,
        }
    }

    async fn client(&self) -> (Client, UnboundedReceiver<Event>) {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
        let events = client.take_events().unwrap();
        (client, events)
    }

    /// Send a message with one picture and wait for her reply to finish.
    async fn send_picture(&self, client: &Client, events: &mut UnboundedReceiver<Event>) -> String {
        let sent: ChatSendResult = client
            .call(
                method::CHAT_SEND,
                json!({
                    "text": "what's in this?",
                    "images": [{"media_type": "image/png", "data": PNG}],
                }),
            )
            .await
            .unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
                .await
                .expect("turn timed out")
                .expect("connection closed");
            if matches!(&event, Event::TurnFinished { turn_id, .. } if *turn_id == sent.turn_id) {
                return sent.session_id;
            }
        }
    }

    /// The user message the chat model was sent last.
    fn last_user_message(&self) -> Value {
        let requests = self.chat_model.requests();
        let messages = requests.last().expect("the chat model was asked").body["messages"].clone();
        messages
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .cloned()
            .unwrap()
    }
}

#[tokio::test]
async fn a_picture_goes_to_a_model_that_can_see_and_stays_with_the_chat() {
    let harness = Harness::start(true, false).await;
    let (client, mut events) = harness.client().await;

    let session = harness.send_picture(&client, &mut events).await;

    let sent = harness.last_user_message();
    assert_eq!(
        sent["content"][0],
        json!({"type": "text", "text": "what's in this?"})
    );
    assert_eq!(
        sent["content"][1]["image_url"]["url"],
        format!("data:image/png;base64,{PNG}")
    );
    let saved: Vec<_> = std::fs::read_dir(harness.dir.path().join("data/images"))
        .unwrap()
        .collect();
    assert_eq!(saved.len(), 1, "the picture is kept as a file");

    let chat: Session = client
        .call(method::SESSIONS_LOAD, json!({"session_id": session}))
        .await
        .unwrap();
    let mine = chat
        .messages
        .iter()
        .find(|m| m.content == "what's in this?")
        .unwrap();
    assert_eq!(mine.images.len(), 1);
    assert_eq!(mine.images[0].media_type, "image/png");
    let back: Value = client
        .call(method::IMAGES_GET, json!({"id": mine.images[0].id}))
        .await
        .unwrap();
    assert_eq!(back["data"], PNG);
    assert!(harness.helper.requests().is_empty(), "no helper needed");
}

#[tokio::test]
async fn a_model_that_cant_see_gets_the_vision_helpers_description() {
    let harness = Harness::start(false, true).await;
    let (client, mut events) = harness.client().await;

    harness.send_picture(&client, &mut events).await;

    let [asked] = harness.helper.requests().try_into().unwrap();
    let helper_saw = asked.body["messages"].to_string();
    assert!(
        helper_saw.contains(PNG),
        "the helper got the picture: {helper_saw}"
    );
    let sent = harness.last_user_message();
    let text = sent["content"]
        .as_str()
        .expect("plain text for a model that can't see");
    assert!(text.contains("what's in this?"), "{text}");
    assert!(
        text.contains("A red bicycle against a brick wall."),
        "{text}"
    );
}

#[tokio::test]
async fn without_a_helper_the_model_is_told_it_cant_see_the_picture() {
    let harness = Harness::start(false, false).await;
    let (client, mut events) = harness.client().await;

    harness.send_picture(&client, &mut events).await;

    let sent = harness.last_user_message();
    let text = sent["content"].as_str().unwrap();
    assert!(text.contains("can't see"), "{text}");
    assert!(!text.contains(PNG));
}
