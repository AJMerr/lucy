//! Making pictures: she can call `generate_image` (hosted generators ask first, ComfyUI on
//! your machine doesn't), and `/image` makes one straight away. Each picture is saved to your
//! pictures folder and kept with the chat.

use std::path::PathBuf;
use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::Role;
use lucy_protocol::rpc::{ApproveParams, ChatSendResult, Decision, Event, Session, method};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

/// A tiny PNG, base64.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==";

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

/// A step that calls `generate_image` with `prompt`.
fn draws(prompt: &str) -> FakeResponse {
    let arguments = json!({"prompt": prompt}).to_string();
    FakeResponse::sse(
        chunk(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                "function": {"name": "generate_image", "arguments": arguments}}]}),
            None,
        ) + &chunk(json!({}), Some("tool_calls"))
            + "data: [DONE]\n\n",
    )
}

/// ComfyUI queueing, finishing and handing over one picture.
fn comfyui_makes_one() -> Vec<FakeResponse> {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    vec![
        FakeResponse::json(200, json!({"prompt_id": "p-1"}).to_string()),
        FakeResponse::json(
            200,
            json!({"p-1": {
                "outputs": {"9": {"images": [
                    {"filename": "Lucy_00001_.png", "subfolder": "", "type": "output"}]}},
                "status": {"status_str": "success", "completed": true, "messages": []},
            }})
            .to_string(),
        ),
        FakeResponse::body(200, "image/png", png),
    ]
}

/// An OpenRouter image model handing over one picture.
fn openrouter_makes_one() -> Vec<FakeResponse> {
    vec![FakeResponse::json(
        200,
        json!({"created": 1, "data": [{"b64_json": PNG, "media_type": "image/png"}]}).to_string(),
    )]
}

enum Maker {
    None,
    ComfyUi,
    OpenRouter,
}

struct Harness {
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    chat_model: FakeServer,
    maker: FakeServer,
    helper: FakeServer,
}

impl Harness {
    async fn start(maker: Maker, vision: bool, replies: Vec<FakeResponse>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let chat_model = FakeServer::start(replies).await;
        let helper = FakeServer::start([says("A picture.")]).await;
        let maker_server = FakeServer::start(match maker {
            Maker::None => vec![],
            Maker::ComfyUi => comfyui_makes_one(),
            Maker::OpenRouter => openrouter_makes_one(),
        })
        .await;
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let mut config = format!(
            "[defaults]\nprofile = \"chat\"\n\n\
             [profiles.chat]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"m\"\nvision = {vision}\n\n\
             [profiles.eyes]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"seer\"\nvision = true\n\n\
             [profiles.pics]\nprovider = \"openai_compat\"\nbase_url = \"https://openrouter.test/api/v1\"\nmodel = \"maker/image-1\"\n\n\
             [images]\nvision_profile = \"eyes\"\nfolder = {:?}\n",
            chat_model.url(),
            helper.url(),
            dir.path().join("Pictures").display().to_string(),
        );
        match maker {
            Maker::None => {}
            Maker::ComfyUi => {
                config += &format!(
                    "generator = \"comfyui\"\n\n[images.comfyui]\nurl = \"{}\"\ncheckpoint = \"dream.safetensors\"\n",
                    maker_server.url()
                );
            }
            Maker::OpenRouter => {
                // The image model's address is the fake server's.
                config = config.replace("https://openrouter.test", maker_server.url());
                config += "generator = \"pics\"\n";
            }
        }
        std::fs::write(&config_path, config).unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-generate-{unique}"))
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
            dir,
            endpoint,
            chat_model,
            maker: maker_server,
            helper,
        }
    }

    async fn client(&self) -> (Client, UnboundedReceiver<Event>) {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
        let events = client.take_events().unwrap();
        (client, events)
    }

    /// Wait for turn `sent` to finish, answering approval prompts with `answer`; returns
    /// every event on the way.
    async fn finish(
        &self,
        client: &Client,
        events: &mut UnboundedReceiver<Event>,
        sent: &ChatSendResult,
        answer: Decision,
    ) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
                .await
                .expect("turn timed out")
                .expect("connection closed");
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

    async fn ask(
        &self,
        client: &Client,
        events: &mut UnboundedReceiver<Event>,
        answer: Decision,
    ) -> (ChatSendResult, Vec<Event>) {
        let sent: ChatSendResult = client
            .call(method::CHAT_SEND, json!({"text": "draw me a lighthouse"}))
            .await
            .unwrap();
        let seen = self.finish(client, events, &sent, answer).await;
        (sent, seen)
    }

    fn saved_pictures(&self) -> Vec<PathBuf> {
        match std::fs::read_dir(self.dir.path().join("Pictures")) {
            Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
            Err(_) => vec![],
        }
    }

    /// The messages the chat model was sent last.
    fn last_messages(&self) -> Vec<Value> {
        let requests = self.chat_model.requests();
        let messages = &requests.last().expect("the chat model was asked").body["messages"];
        messages.as_array().unwrap().clone()
    }
}

fn asked_approval(events: &[Event]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::ToolApproval { .. }))
}

// --- generate_image -------------------------------------------------------------------------

#[tokio::test]
async fn she_makes_a_picture_with_comfyui_without_asking() {
    let harness = Harness::start(
        Maker::ComfyUi,
        true,
        vec![draws("a lighthouse at dusk"), says("Here it is.")],
    )
    .await;
    let (client, mut events) = harness.client().await;

    let (sent, seen) = harness.ask(&client, &mut events, Decision::Deny).await;

    assert!(!asked_approval(&seen), "local generation runs by itself");
    let queued = &harness.maker.requests()[0].body["prompt"];
    assert!(
        queued.to_string().contains("a lighthouse at dusk"),
        "{queued}"
    );
    let saved = harness.saved_pictures();
    assert_eq!(saved.len(), 1, "saved to the pictures folder");
    let name = saved[0].file_name().unwrap().to_string_lossy();
    assert!(name.ends_with("a-lighthouse-at-dusk.png"), "{name}");

    let finished = seen
        .iter()
        .find_map(|e| match e {
            Event::ToolFinished {
                output,
                images,
                is_error,
                ..
            } => Some((output.clone(), images.clone(), *is_error)),
            _ => None,
        })
        .unwrap();
    assert!(!finished.2, "{}", finished.0);
    assert!(
        finished.0.contains(&saved[0].display().to_string()),
        "{}",
        finished.0
    );
    assert_eq!(finished.1.len(), 1, "the picture comes with the result");

    // A model that can see gets to look at what it made.
    let messages = harness.last_messages();
    let result = messages.iter().position(|m| m["role"] == "tool").unwrap();
    assert_eq!(
        messages[result + 1]["content"][1]["image_url"]["url"],
        format!("data:image/png;base64,{PNG}")
    );

    let chat: Session = client
        .call(
            method::SESSIONS_LOAD,
            json!({"session_id": sent.session_id}),
        )
        .await
        .unwrap();
    let kept = chat.messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(kept.images.len(), 1, "kept with the chat");
    assert!(kept.images[0].made);
}

#[tokio::test]
async fn a_model_that_cant_see_isnt_sent_or_told_about_the_picture_it_made() {
    let harness = Harness::start(
        Maker::ComfyUi,
        false,
        vec![draws("a lighthouse"), says("Here it is.")],
    )
    .await;
    let (client, mut events) = harness.client().await;

    harness.ask(&client, &mut events, Decision::Deny).await;

    let messages = harness.last_messages();
    assert!(!messages.iter().any(|m| m["content"].is_array()));
    let result = messages.iter().find(|m| m["role"] == "tool").unwrap();
    assert!(!result["content"].as_str().unwrap().contains("can't see"));
    assert!(harness.helper.requests().is_empty(), "nothing to describe");
}

#[tokio::test]
async fn a_hosted_picture_asks_first() {
    let harness = Harness::start(
        Maker::OpenRouter,
        true,
        vec![draws("a lighthouse"), says("Here it is.")],
    )
    .await;
    let (client, mut events) = harness.client().await;

    let (_, seen) = harness.ask(&client, &mut events, Decision::Once).await;

    let reason = seen
        .iter()
        .find_map(|e| match e {
            Event::ToolApproval {
                reason, preview, ..
            } => Some((reason.clone(), preview.clone())),
            _ => None,
        })
        .expect("asked before paying for a picture");
    assert!(reason.0.contains("pics"), "{}", reason.0);
    assert!(reason.1.unwrap_or_default().contains("a lighthouse"));
    assert_eq!(harness.maker.requests()[0].body["model"], "maker/image-1");
    assert_eq!(harness.saved_pictures().len(), 1);
}

#[tokio::test]
async fn declining_a_hosted_picture_makes_nothing() {
    let harness = Harness::start(
        Maker::OpenRouter,
        true,
        vec![draws("a lighthouse"), says("Okay, I won't.")],
    )
    .await;
    let (client, mut events) = harness.client().await;

    harness.ask(&client, &mut events, Decision::Deny).await;

    assert!(harness.maker.requests().is_empty());
    assert!(harness.saved_pictures().is_empty());
}

#[tokio::test]
async fn without_a_generator_she_isnt_offered_the_tool() {
    let harness = Harness::start(Maker::None, true, vec![says("I can't draw.")]).await;
    let (client, mut events) = harness.client().await;

    harness.ask(&client, &mut events, Decision::Deny).await;

    let request = &harness.chat_model.requests()[0].body;
    let tools = request["tools"].to_string();
    assert!(!tools.contains("generate_image"), "{tools}");
}

// --- /image ---------------------------------------------------------------------------------

#[tokio::test]
async fn slash_image_makes_a_picture_straight_away_without_asking() {
    let harness = Harness::start(Maker::OpenRouter, true, vec![]).await;
    let (client, mut events) = harness.client().await;

    let sent: ChatSendResult = client
        .call(method::IMAGES_GENERATE, json!({"prompt": "a red bicycle"}))
        .await
        .unwrap();
    let seen = harness
        .finish(&client, &mut events, &sent, Decision::Deny)
        .await;

    assert!(!asked_approval(&seen), "you typed it, so it doesn't ask");
    assert!(harness.chat_model.requests().is_empty(), "no model turn");
    let sent_prompt = harness.maker.requests()[0].body["prompt"].clone();
    assert_eq!(sent_prompt, "a red bicycle");
    assert_eq!(harness.saved_pictures().len(), 1);
    assert!(seen.iter().any(|e| matches!(e,
        Event::ToolFinished { images, is_error: false, .. } if images.len() == 1)));
    assert!(seen.iter().any(|e| matches!(e,
        Event::TurnFinished { outcome, .. } if *outcome == lucy_protocol::rpc::TurnOutcome::Complete)));

    let chat: Session = client
        .call(
            method::SESSIONS_LOAD,
            json!({"session_id": sent.session_id}),
        )
        .await
        .unwrap();
    assert_eq!(chat.messages[0].content, "/image a red bicycle");
    let kept = chat.messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(kept.images.len(), 1);
}

#[tokio::test]
async fn slash_image_without_a_generator_says_how_to_set_one_up() {
    let harness = Harness::start(Maker::None, true, vec![]).await;
    let (client, _events) = harness.client().await;

    let err = client
        .call::<_, ChatSendResult>(method::IMAGES_GENERATE, json!({"prompt": "a fox"}))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("generator"), "{err}");
}
