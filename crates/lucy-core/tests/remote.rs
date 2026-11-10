//! Remote access: the core over WebSocket, behind a token, in its own folder and at a capped
//! agent level.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};
use lucy_core::server::{CoreOptions, REMOTE_TOKEN_KEY, serve};
use lucy_protocol::rpc::{
    ApproveParams, ChatSendParams, ChatSendResult, Decision, Event, SessionSettings,
    SetLevelParams, SetModeParams, TurnOutcome, method,
};
use lucy_protocol::{AgentLevel, Mode};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

const TOKEN: &str = "test-token-0123456789abcdef";

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}

/// A model step that calls one tool.
fn calls(name: &str, args: Value) -> FakeResponse {
    let body = chunk(
        json!({"tool_calls": [{"index": 0, "id": format!("call_{name}"), "type": "function",
               "function": {"name": name, "arguments": args.to_string()}}]}),
        None,
    ) + &chunk(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

/// A model step that answers in text.
fn says(text: &str) -> FakeResponse {
    let body = chunk(json!({"content": text}), None)
        + &chunk(json!({}), Some("stop"))
        + "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Harness {
    dir: tempfile::TempDir,
    local: Endpoint,
    port: u16,
    _model: FakeServer,
}

impl Harness {
    /// A core whose `[remote]` section is `remote` (`{port}` is replaced by a free port).
    async fn start(script: Vec<FakeResponse>, remote: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("marker.txt"), "the core's folder").unwrap();
        std::fs::create_dir_all(dir.path().join("laptop")).unwrap();
        std::fs::write(dir.path().join("laptop/decoy.txt"), "the client's folder").unwrap();
        let model = FakeServer::start(script).await;
        let port = free_port();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"fake\"\n\n\
                 [profiles.fake]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n\
                 [agent]\nworkspace = {:?}\n\n[remote]\n{}\n",
                model.url(),
                workspace.display().to_string(),
                remote.replace("{port}", &port.to_string()),
            ),
        )
        .unwrap();
        let local = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-remote-test-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let keyring = MemoryStore::default();
        keyring.set(REMOTE_TOKEN_KEY, TOKEN).unwrap();
        let options = CoreOptions {
            endpoint: local.clone(),
            config_path,
            db_path: dir.path().join("lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(keyring), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Client::connect(&local, "probe").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            dir,
            local,
            port,
            _model: model,
        }
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/rpc", self.port)
    }

    fn remote(&self, token: &str) -> Endpoint {
        Endpoint::Remote {
            url: self.url(),
            token: token.into(),
        }
    }

    /// Connect over WebSocket, waiting for the listener to come up.
    async fn remote_client(&self) -> Client {
        for _ in 0..200 {
            if let Ok(client) = Client::connect(&self.remote(TOKEN), "test").await {
                return client;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("remote listener never came up");
    }

    /// Whether anything accepts TCP connections on the remote port (after giving it time).
    async fn listening(&self) -> bool {
        tokio::time::sleep(Duration::from_millis(300)).await;
        tokio::net::TcpStream::connect(("127.0.0.1", self.port))
            .await
            .is_ok()
    }

    fn workspace(&self) -> PathBuf {
        self.dir.path().join("workspace")
    }

    fn laptop(&self) -> PathBuf {
        self.dir.path().join("laptop")
    }
}

const ON: &str = "enabled = true\nbind = \"127.0.0.1:{port}\"";

async fn session(client: &Client, mode: Mode, level: AgentLevel) -> String {
    let id: String = client
        .call(method::SESSIONS_CREATE, json!({}))
        .await
        .unwrap();
    let _: SessionSettings = client
        .call(
            method::SESSION_SET_MODE,
            SetModeParams {
                session_id: id.clone(),
                mode,
            },
        )
        .await
        .unwrap();
    let _: SessionSettings = client
        .call(
            method::SESSION_SET_LEVEL,
            SetLevelParams {
                session_id: id.clone(),
                level,
            },
        )
        .await
        .unwrap();
    id
}

async fn send(client: &Client, session: Option<&str>, text: &str, cwd: &Path) -> ChatSendResult {
    client
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                incognito: false,
                images: Vec::new(),
                session_id: session.map(str::to_owned),
                text: text.into(),
                cwd: Some(cwd.display().to_string()),
            },
        )
        .await
        .unwrap()
}

/// One turn's events until it finishes, declining anything that asks.
async fn run_turn(
    client: &Client,
    events: &mut UnboundedReceiver<Event>,
    sent: &ChatSendResult,
) -> (Vec<Event>, TurnOutcome) {
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
                        decision: Decision::Deny,
                    },
                )
                .await
                .unwrap();
        }
        if let Event::TurnFinished {
            turn_id, outcome, ..
        } = &event
            && *turn_id == sent.turn_id
        {
            return (seen, *outcome);
        }
        seen.push(event);
    }
}

#[tokio::test]
async fn a_client_with_the_token_chats_over_websocket() {
    let h = Harness::start(vec![says("Hello from the desktop")], ON).await;
    let mut client = h.remote_client().await;
    let mut events = client.take_events().unwrap();

    let sent = send(&client, None, "hi", &h.laptop()).await;
    let (seen, outcome) = run_turn(&client, &mut events, &sent).await;

    assert_eq!(outcome, TurnOutcome::Complete);
    let text: String = seen
        .iter()
        .filter_map(|e| match e {
            Event::Delta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello from the desktop");
}

#[tokio::test]
async fn a_missing_or_wrong_token_is_refused() {
    let h = Harness::start(vec![], ON).await;
    h.remote_client().await; // the listener is up

    for token in ["", "not-the-token"] {
        let err = Client::connect(&h.remote(token), "test")
            .await
            .err()
            .expect("connected without the right token")
            .to_string();
        assert!(err.contains("token"), "{err}");
    }
}

#[tokio::test]
async fn browsers_can_send_the_token_as_a_subprotocol() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let h = Harness::start(vec![], ON).await;
    h.remote_client().await;
    let mut request = h.url().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("lucy, lucy-token.{TOKEN}").parse().unwrap(),
    );

    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();

    assert_eq!(response.headers()["sec-websocket-protocol"], "lucy");
    socket
        .send(Message::text(
            json!({"jsonrpc": "2.0", "id": 1, "method": "core.status"}).to_string(),
        ))
        .await
        .unwrap();
    let reply = socket.next().await.unwrap().unwrap().into_text().unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(
        reply["result"]["protocol_version"],
        lucy_protocol::rpc::PROTOCOL_VERSION
    );
}

#[tokio::test]
async fn remote_chats_work_in_the_cores_workspace() {
    let h = Harness::start(
        vec![calls("list_dir", json!({"path": "."})), says("Listed it.")],
        ON,
    )
    .await;
    let mut client = h.remote_client().await;
    let mut events = client.take_events().unwrap();
    assert_eq!(
        client.folder(),
        Some(h.workspace().display().to_string().as_str())
    );
    let id = session(&client, Mode::Agent, AgentLevel::Guarded).await;

    let sent = send(&client, Some(&id), "what's here?", &h.laptop()).await;
    let (seen, _) = run_turn(&client, &mut events, &sent).await;

    let listing = seen
        .iter()
        .find_map(|e| match e {
            Event::ToolFinished { output, .. } => Some(output.clone()),
            _ => None,
        })
        .expect("list_dir ran without asking");
    assert!(listing.contains("marker.txt"), "{listing}");
    assert!(!listing.contains("decoy.txt"), "{listing}");
}

#[tokio::test]
async fn remote_chats_are_capped_at_max_level() {
    let h = Harness::start(
        vec![
            calls("run_command", json!({"command": "echo hi"})),
            says("Okay, I won't."),
        ],
        ON,
    )
    .await;
    let mut client = h.remote_client().await;
    let mut events = client.take_events().unwrap();
    let id = session(&client, Mode::Agent, AgentLevel::Unrestricted).await;

    let sent = send(&client, Some(&id), "run it", &h.laptop()).await;
    let (seen, _) = run_turn(&client, &mut events, &sent).await;

    let asked = seen
        .iter()
        .any(|e| matches!(e, Event::ToolApproval { call, .. } if call.name == "run_command"));
    assert!(asked, "level 3 ran a command for a remote client: {seen:?}");
}

#[tokio::test]
async fn a_network_bind_needs_allow_network() {
    let h = Harness::start(vec![], "enabled = true\nbind = \"0.0.0.0:{port}\"").await;

    assert!(
        !h.listening().await,
        "listened beyond loopback without allow_network"
    );
    Client::connect(&h.local, "test")
        .await
        .expect("the local socket still works");

    let allowed = Harness::start(
        vec![],
        "enabled = true\nbind = \"0.0.0.0:{port}\"\nallow_network = true",
    )
    .await;
    assert!(allowed.listening().await);
}

#[tokio::test]
async fn remote_access_is_off_by_default() {
    let h = Harness::start(vec![], "").await;

    assert!(!h.listening().await);
}

#[tokio::test]
async fn remote_clients_can_read_settings_but_not_change_them_unless_allowed() {
    let h = Harness::start(vec![], ON).await;
    let remote = h.remote_client().await;

    let view: Value = remote.call(method::CONFIG_GET, json!({})).await.unwrap();
    assert_eq!(view["config"]["defaults"]["profile"], "fake");
    for (name, params) in [
        (
            method::CONFIG_UPDATE,
            json!({"patch": {"agent": {"max_steps": 3}}}),
        ),
        (
            method::KEYS_SET,
            json!({"name": "anthropic", "value": "sk-x"}),
        ),
        (method::KEYS_REMOVE, json!({"name": "anthropic"})),
        (method::PERSONA_SET, json!({"text": "hi"})),
        (method::PERSONA_RESET, json!({})),
    ] {
        let refused = remote
            .call::<_, Value>(name, params)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("allow_config"), "{name}: {refused}");
    }
    let written = std::fs::read_to_string(h.dir.path().join("config.toml")).unwrap();
    assert!(!written.contains("max_steps = 3"), "{written}");
}

#[tokio::test]
async fn allow_config_lets_remote_clients_change_settings() {
    let h = Harness::start(vec![], &format!("{ON}\nallow_config = true")).await;
    let remote = h.remote_client().await;

    let _: Value = remote
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"agent": {"max_steps": 3}}}),
        )
        .await
        .unwrap();

    let written = std::fs::read_to_string(h.dir.path().join("config.toml")).unwrap();
    assert!(written.contains("max_steps = 3"), "{written}");
}

#[tokio::test]
async fn mcp_sign_in_only_happens_on_the_cores_machine() {
    // The browser would open over there, so even allow_config doesn't make it work remotely.
    let h = Harness::start(vec![], &format!("{ON}\nallow_config = true")).await;
    let remote = h.remote_client().await;

    let refused = remote
        .call::<_, Value>(method::MCP_LOGIN, json!({"name": "linear"}))
        .await
        .unwrap_err()
        .to_string();

    assert!(refused.contains("machine that runs Lucy"), "{refused}");
}
