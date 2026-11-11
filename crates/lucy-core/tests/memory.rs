//! Long-term memory: facts she saves in memory.md with `remember` (sensitive ones ask, secrets
//! are refused), drops with `forget`, and finds with `recall`; every chat's prompt carries them,
//! picked by relevance once there are too many. Incognito chats neither read nor save, and
//! aren't kept.

use std::time::Duration;

use interprocess::local_socket::tokio::{Stream, prelude::*};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{ApproveParams, ChatSendResult, Decision, Event, SessionSummary, method};
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

/// A step that calls tool `name` with `arguments`.
fn calls(name: &str, arguments: Value) -> FakeResponse {
    FakeResponse::sse(
        chunk(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}}]}),
            None,
        ) + &chunk(json!({}), Some("tool_calls"))
            + "data: [DONE]\n\n",
    )
}

struct Harness {
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    chat_model: FakeServer,
}

impl Harness {
    async fn start(replies: Vec<FakeResponse>) -> Self {
        Self::with(replies, "", None).await
    }

    /// A core whose chat model gives `replies` in turn, with `extra` added to its config and
    /// memory.md holding `memory` to begin with.
    async fn with(replies: Vec<FakeResponse>, extra: &str, memory: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let chat_model = FakeServer::start(replies).await;
        let config_path = dir.path().join("config/config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        if let Some(memory) = memory {
            std::fs::write(dir.path().join("data/memory.md"), memory).unwrap();
        }
        let config = format!(
            "[defaults]\nprofile = \"chat\"\n\n\
             [profiles.chat]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"m\"\n\n{extra}",
            chat_model.url(),
        );
        std::fs::write(&config_path, config).unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-memory-{unique}"))
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
        }
    }

    async fn client(&self) -> (Client, UnboundedReceiver<Event>) {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
        let events = client.take_events().unwrap();
        (client, events)
    }

    /// Send `params` (a `chat.send`), answer any approval with `answer`, and return the
    /// session and every event of the reply.
    async fn send(
        &self,
        client: &Client,
        events: &mut UnboundedReceiver<Event>,
        params: Value,
        answer: Decision,
    ) -> (String, Vec<Event>) {
        let sent: ChatSendResult = client.call(method::CHAT_SEND, params).await.unwrap();
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
                return (sent.session_id, seen);
            }
        }
    }

    fn memory(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("data/memory.md")).unwrap_or_default()
    }

    /// The system prompt of the chat model's `n`th request.
    fn system_prompt(&self, n: usize) -> String {
        self.chat_model.requests()[n].body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The tool names offered in the chat model's `n`th request.
    fn tools(&self, n: usize) -> Vec<String> {
        self.chat_model.requests()[n].body["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn asked(events: &[Event]) -> Option<(String, Option<String>)> {
    events.iter().find_map(|e| match e {
        Event::ToolApproval {
            reason, preview, ..
        } => Some((reason.clone(), preview.clone())),
        _ => None,
    })
}

fn tool_result(events: &[Event]) -> (String, bool) {
    events
        .iter()
        .find_map(|e| match e {
            Event::ToolFinished {
                output, is_error, ..
            } => Some((output.clone(), *is_error)),
            _ => None,
        })
        .expect("a tool ran")
}

// --- remember and forget ------------------------------------------------------------------------

#[tokio::test]
async fn she_remembers_on_her_own_and_another_chat_knows() {
    let harness = Harness::start(vec![
        calls(
            "remember",
            json!({"heading": "Preferences", "fact": "Edits code in Neovim"}),
        ),
        says("Noted."),
        says("You use Neovim."),
    ])
    .await;
    let (client, mut events) = harness.client().await;

    let (_, seen) = harness
        .send(
            &client,
            &mut events,
            json!({"text": "I always use neovim btw"}),
            Decision::Deny,
        )
        .await;
    assert_eq!(asked(&seen), None, "ordinary facts don't ask");
    assert!(!tool_result(&seen).1);
    let memory = harness.memory();
    assert!(memory.contains("## Preferences"), "{memory}");
    assert!(memory.contains("- Edits code in Neovim"), "{memory}");

    harness
        .send(
            &client,
            &mut events,
            json!({"text": "what editor do I use?"}),
            Decision::Deny,
        )
        .await;
    let prompt = harness.system_prompt(2);
    assert!(prompt.contains("Edits code in Neovim"), "{prompt}");
}

#[tokio::test]
async fn a_sensitive_fact_asks_first() {
    let harness = Harness::start(vec![
        calls(
            "remember",
            json!({"heading": "About you", "fact": "Has type 1 diabetes", "sensitive": true}),
        ),
        says("Okay, I won't."),
    ])
    .await;
    let (client, mut events) = harness.client().await;

    let (_, seen) = harness
        .send(
            &client,
            &mut events,
            json!({"text": "I'm diabetic"}),
            Decision::Deny,
        )
        .await;

    let (reason, preview) = asked(&seen).expect("asked before saving");
    assert!(reason.contains("sensitive"), "{reason}");
    assert!(preview.unwrap_or_default().contains("Has type 1 diabetes"));
    assert!(!harness.memory().contains("diabetes"));
}

#[tokio::test]
async fn a_secret_is_refused_without_asking() {
    let harness = Harness::start(vec![
        calls(
            "remember",
            json!({"heading": "About you",
                   "fact": "OpenRouter key: sk-or-v1-0123456789abcdef0123456789abcdef"}),
        ),
        says("I won't keep that."),
    ])
    .await;
    let (client, mut events) = harness.client().await;

    let (_, seen) = harness
        .send(
            &client,
            &mut events,
            json!({"text": "here's my key"}),
            Decision::Once,
        )
        .await;

    assert_eq!(asked(&seen), None);
    let (output, is_error) = tool_result(&seen);
    assert!(is_error);
    assert!(output.contains("secret"), "{output}");
    assert!(!harness.memory().contains("sk-or"));
}

#[tokio::test]
async fn forget_removes_a_fact_and_remember_can_replace_one() {
    let harness = Harness::with(
        vec![
            calls("forget", json!({"fact": "Lives in Denver"})),
            calls(
                "remember",
                json!({"heading": "Projects", "fact": "Building Lucy v0.7",
                       "replaces": "Building Lucy v0.6"}),
            ),
            says("Done."),
        ],
        "",
        Some("# Memory\n\n## About you\n- Lives in Denver\n- Has a dog called Biscuit\n\n## Projects\n- Building Lucy v0.6\n"),
    )
    .await;
    let (client, mut events) = harness.client().await;

    harness
        .send(
            &client,
            &mut events,
            json!({"text": "I moved; and I'm on 0.7"}),
            Decision::Deny,
        )
        .await;

    let memory = harness.memory();
    assert!(!memory.contains("Denver"), "{memory}");
    assert!(memory.contains("- Has a dog called Biscuit"), "{memory}");
    assert!(memory.contains("- Building Lucy v0.7"), "{memory}");
    assert!(!memory.contains("v0.6"), "{memory}");
}

#[tokio::test]
async fn with_memory_off_she_neither_reads_nor_saves() {
    let harness = Harness::with(
        vec![says("Hi.")],
        "[memory]\nenabled = false\n",
        Some("## About you\n- Has a dog called Biscuit\n"),
    )
    .await;
    let (client, mut events) = harness.client().await;

    harness
        .send(&client, &mut events, json!({"text": "hi"}), Decision::Deny)
        .await;

    assert!(!harness.system_prompt(0).contains("Biscuit"));
    assert!(!harness.tools(0).contains(&"remember".to_owned()));
}

// --- incognito ----------------------------------------------------------------------------------

#[tokio::test]
async fn an_incognito_chat_has_no_memory_and_is_gone_once_you_leave() {
    let harness = Harness::with(
        vec![says("Hi, stranger.")],
        "",
        Some("## About you\n- Has a dog called Biscuit\n"),
    )
    .await;
    let (client, mut events) = harness.client().await;

    let (session, _) = harness
        .send(
            &client,
            &mut events,
            json!({"text": "hi", "incognito": true}),
            Decision::Deny,
        )
        .await;

    assert!(!harness.system_prompt(0).contains("Biscuit"));
    let tools = harness.tools(0);
    for tool in ["remember", "forget", "recall"] {
        assert!(!tools.contains(&tool.to_owned()), "{tools:?}");
    }
    let listed: Vec<SessionSummary> = client.call(method::SESSIONS_LIST, json!({})).await.unwrap();
    let mine = listed
        .iter()
        .find(|s| s.id == session)
        .expect("listed while open");
    assert!(mine.incognito);

    // Leaving: the client that started it goes away.
    drop(events);
    drop(client);
    let (other, _events) = harness.client().await;
    let mut gone = false;
    for _ in 0..100 {
        let listed: Vec<SessionSummary> =
            other.call(method::SESSIONS_LIST, json!({})).await.unwrap();
        if !listed.iter().any(|s| s.id == session) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "the incognito chat was deleted");
}

// --- seeing and editing it ----------------------------------------------------------------------

#[tokio::test]
async fn memory_can_be_read_and_replaced_over_rpc() {
    let harness = Harness::with(
        vec![],
        "",
        Some("## About you\n- Has a dog called Biscuit\n"),
    )
    .await;
    let (client, _events) = harness.client().await;

    let view: Value = client.call(method::MEMORY_GET, json!({})).await.unwrap();
    assert_eq!(view["text"], "## About you\n- Has a dog called Biscuit\n");
    assert_eq!(
        view["facts"],
        json!([{"heading": "About you", "text": "Has a dog called Biscuit"}])
    );

    let _: Value = client
        .call(
            method::MEMORY_SET,
            json!({"text": "## People\n- Sister: Maya\n"}),
        )
        .await
        .unwrap();
    assert_eq!(harness.memory(), "## People\n- Sister: Maya\n");
}

// --- recall -------------------------------------------------------------------------------------

/// A memory well past what goes in every prompt: a few facts about you, and many others.
fn big_memory() -> String {
    let mut text = String::from(
        "## About you\n- Name: Austin\n\n## Preferences\n- Likes short answers\n\n## Projects\n",
    );
    for n in 0..300 {
        text += &format!("- Side project number {n} about gardening schedules and compost\n");
    }
    text += "- Restoring a 1972 Honda motorcycle in the garage\n";
    text
}

#[tokio::test]
async fn a_big_memory_sends_the_core_facts_and_the_relevant_ones() {
    let harness = Harness::with(vec![says("Nice bike.")], "", Some(&big_memory())).await;
    let (client, mut events) = harness.client().await;

    harness
        .send(
            &client,
            &mut events,
            json!({"text": "how's my motorcycle restoration going?"}),
            Decision::Deny,
        )
        .await;

    let prompt = harness.system_prompt(0);
    assert!(prompt.contains("Name: Austin"), "always: about you");
    assert!(
        prompt.contains("Likes short answers"),
        "always: preferences"
    );
    assert!(
        prompt.contains("1972 Honda motorcycle"),
        "the relevant fact"
    );
    assert!(
        prompt.len() < 12 * 1024,
        "not the whole memory: {}",
        prompt.len()
    );
    assert!(
        prompt.contains("recall"),
        "she's told she can search the rest"
    );
}

#[tokio::test]
async fn recall_finds_facts_by_keyword() {
    let harness = Harness::with(
        vec![
            calls("recall", json!({"query": "motorcycles"})),
            says("Found it."),
        ],
        "",
        Some(&big_memory()),
    )
    .await;
    let (client, mut events) = harness.client().await;

    let (_, seen) = harness
        .send(
            &client,
            &mut events,
            json!({"text": "what bikes do I have?"}),
            Decision::Deny,
        )
        .await;

    let (output, is_error) = tool_result(&seen);
    assert!(!is_error, "{output}");
    assert!(output.contains("1972 Honda motorcycle"), "{output}");
    assert!(!output.contains("compost"), "{output}");
}

#[tokio::test]
async fn with_an_embedding_model_relevant_facts_are_found_by_meaning() {
    // The fake embedding model puts the motorcycle fact and "bike" close together, and
    // everything else far away. Facts are embedded once, then the conversation.
    let memory = big_memory();
    let facts = memory.lines().filter(|l| l.starts_with("- ")).count();
    let fact_vectors: Vec<Value> = memory
        .lines()
        .filter(|l| l.starts_with("- "))
        .enumerate()
        .map(|(i, line)| {
            let v = if line.contains("motorcycle") {
                [1.0, 0.0]
            } else {
                [0.0, 1.0]
            };
            json!({"index": i, "embedding": v})
        })
        .collect();
    assert_eq!(fact_vectors.len(), facts);
    let embedder = FakeServer::start([
        FakeResponse::json(200, json!({"data": fact_vectors}).to_string()).chunk_size(65536),
        FakeResponse::json(
            200,
            json!({"data": [{"index": 0, "embedding": [0.9, 0.1]}]}).to_string(),
        ),
    ])
    .await;
    let extra = format!(
        "[profiles.embed]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"nomic-embed-text\"\n\n\
         [memory]\nembedding_profile = \"embed\"\n",
        embedder.url()
    );
    let harness = Harness::with(vec![says("Nice bike.")], &extra, Some(&memory)).await;
    let (client, mut events) = harness.client().await;

    harness
        .send(
            &client,
            &mut events,
            json!({"text": "how's my bike?"}),
            Decision::Deny,
        )
        .await;

    let prompt = harness.system_prompt(0);
    assert!(
        prompt.contains("1972 Honda motorcycle"),
        "found by meaning, not words"
    );
    let requests = embedder.requests();
    assert_eq!(requests[0].path, "/v1/embeddings");
    assert_eq!(requests[0].body["model"], "nomic-embed-text");
}
