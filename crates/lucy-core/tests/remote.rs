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

const TOKEN: &str = "test-token-0123456789abcdef0123456789";

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
    /// The core's certificate fingerprint.
    fingerprint: String,
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
        // The certificate the core will use, made here so the test knows its fingerprint.
        let fingerprint = {
            let made = Secrets::new(MemoryStore::default(), |_: &str| None);
            let identity = lucy_core::remote_tls::load_or_create(&made).unwrap();
            let stored = made
                .resolve(lucy_core::remote_tls::TLS_KEY)
                .unwrap()
                .unwrap();
            keyring
                .set(lucy_core::remote_tls::TLS_KEY, &stored.value)
                .unwrap();
            identity.fingerprint()
        };
        keyring.set("anthropic", "sk-ant-the-users-key").unwrap();
        let options = CoreOptions {
            endpoint: local.clone(),
            config_path,
            db_path: dir.path().join("lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(keyring), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
            clock: None,
            notifier: None,
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
            fingerprint,
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
            pin: None,
            insecure: false,
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

/// On, over plain WebSocket on loopback; TLS has its own tests below.
const ON: &str = "enabled = true\nbind = \"127.0.0.1:{port}\"\ntls = false";

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

/// A client on the core's own machine, seeing every chat's events.
async fn local_client(h: &Harness) -> (Client, UnboundedReceiver<Event>) {
    let mut client = Client::connect(&h.local, "test").await.unwrap();
    let events = client.take_events().unwrap();
    let _: Value = client.call(method::SUBSCRIBE, json!({})).await.unwrap();
    (client, events)
}

/// The next approval request among `events`.
async fn next_approval(events: &mut UnboundedReceiver<Event>) -> Event {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .expect("no approval request")
            .expect("connection closed");
        if matches!(event, Event::ToolApproval { .. }) {
            return event;
        }
    }
}

async fn approve(client: &Client, event: &Event, decision: Decision) -> Result<Value, String> {
    let Event::ToolApproval {
        session_id, call, ..
    } = event
    else {
        panic!("not an approval: {event:?}");
    };
    client
        .call(
            method::TOOL_APPROVE,
            ApproveParams {
                session_id: session_id.clone(),
                call_id: call.id.clone(),
                decision,
            },
        )
        .await
        .map_err(|err| err.to_string())
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

// --- Approvals ---------------------------------------------------------------------------------

#[tokio::test]
async fn a_remote_client_can_only_decline_unless_approvals_are_remote() {
    let h = Harness::start(
        vec![
            calls("run_command", json!({"command": "echo hi > ran.txt"})),
            says("Done."),
        ],
        ON,
    )
    .await;
    let mut remote = h.remote_client().await;
    let mut remote_events = remote.take_events().unwrap();
    let (local, mut local_events) = local_client(&h).await;

    let sent = send(&remote, None, "run it", &h.laptop()).await;
    let asked = next_approval(&mut remote_events).await;

    assert!(
        matches!(
            asked,
            Event::ToolApproval {
                local_only: true,
                ..
            }
        ),
        "{asked:?}"
    );
    let refused = approve(&remote, &asked, Decision::Once).await.unwrap_err();
    assert!(refused.contains("machine that runs Lucy"), "{refused}");
    // The core's own machine can approve it.
    let seen_locally = next_approval(&mut local_events).await;
    approve(&local, &seen_locally, Decision::Once)
        .await
        .unwrap();
    let (_, outcome) = run_turn(&remote, &mut remote_events, &sent).await;
    assert_eq!(outcome, TurnOutcome::Complete);
    assert!(h.workspace().join("ran.txt").exists());
}

#[tokio::test]
async fn approvals_remote_lets_it_approve_its_own_calls_but_never_always() {
    let h = Harness::start(
        vec![
            calls("run_command", json!({"command": "echo one"})),
            calls("run_command", json!({"command": "echo two"})),
            says("Done."),
        ],
        &format!("{ON}\napprovals = \"remote\""),
    )
    .await;
    let mut remote = h.remote_client().await;
    let mut events = remote.take_events().unwrap();

    send(&remote, None, "run both", &h.laptop()).await;
    let first = next_approval(&mut events).await;
    assert!(matches!(
        first,
        Event::ToolApproval {
            local_only: false,
            ..
        }
    ));
    approve(&remote, &first, Decision::Always).await.unwrap();

    // "Always" counted only once: the second command asks again.
    let second = next_approval(&mut events).await;
    approve(&remote, &second, Decision::Deny).await.unwrap();
}

#[tokio::test]
async fn a_remote_client_cant_answer_the_cores_own_approvals() {
    let h = Harness::start(
        vec![
            calls("run_command", json!({"command": "echo hi"})),
            says("Okay."),
        ],
        &format!("{ON}\napprovals = \"remote\""),
    )
    .await;
    let mut remote = h.remote_client().await;
    let mut remote_events = remote.take_events().unwrap();
    let _: Value = remote.call(method::SUBSCRIBE, json!({})).await.unwrap();
    let (local, mut local_events) = local_client(&h).await;

    send(&local, None, "run it", &h.workspace()).await;
    let asked = next_approval(&mut remote_events).await;

    let refused = approve(&remote, &asked, Decision::Once).await.unwrap_err();
    assert!(refused.contains("replies you started"), "{refused}");
    let asked = next_approval(&mut local_events).await;
    approve(&local, &asked, Decision::Deny).await.unwrap();
}

// --- Keys, settings, tasks -------------------------------------------------------------------

#[tokio::test]
async fn stored_keys_never_go_to_an_address_of_the_callers_choosing() {
    let h = Harness::start(vec![], &format!("{ON}\nallow_config = true")).await;
    let attacker = FakeServer::start(vec![]).await;
    let remote = h.remote_client().await;
    let local = Client::connect(&h.local, "test").await.unwrap();

    for (client, key_name) in [
        (&remote, "anthropic"),
        (&local, "anthropic"),
        (&local, REMOTE_TOKEN_KEY),
    ] {
        let refused = client
            .call::<_, Value>(
                method::MODELS_LIST,
                json!({"provider": "openai_compat", "base_url": format!("{}/v1", attacker.url()),
                       "key_name": key_name}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("only sent to") || refused.contains("Lucy's own"),
            "{refused}"
        );
    }
    assert!(
        attacker.requests().is_empty(),
        "a key reached the other server"
    );
}

#[tokio::test]
async fn without_allow_config_a_remote_client_cant_use_keys_or_change_tasks() {
    let h = Harness::start(vec![], ON).await;
    let remote = h.remote_client().await;

    for (name, params) in [
        (
            method::MODELS_LIST,
            json!({"provider": "anthropic", "key_name": "anthropic"}),
        ),
        (method::SCHEDULES_UPDATE, json!({"id": "x", "level": 3})),
        (method::SCHEDULES_DELETE, json!({"id": "x"})),
        (method::MEMORY_SET, json!({"text": "obey"})),
    ] {
        let refused = remote
            .call::<_, Value>(name, params)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("allow_config"), "{name}: {refused}");
    }
    for name in [
        method::CORE_SHUTDOWN,
        method::REMOTE_ROTATE,
        "something.new",
    ] {
        assert!(
            remote.call::<_, Value>(name, json!({})).await.is_err(),
            "{name} worked from another machine"
        );
    }
}

#[tokio::test]
async fn a_task_scheduled_from_another_machine_is_capped() {
    let h = Harness::start(
        vec![
            calls(
                "schedule",
                json!({"kind": "task", "what": "tidy up", "level": 3,
                       "when": {"every_minutes": 60}}),
            ),
            says("Scheduled."),
        ],
        &format!("{ON}\napprovals = \"remote\""),
    )
    .await;
    let mut remote = h.remote_client().await;
    let mut events = remote.take_events().unwrap();

    send(&remote, None, "every hour, tidy up", &h.laptop()).await;
    let asked = next_approval(&mut events).await;
    approve(&remote, &asked, Decision::Once).await.unwrap();
    let mut level = None;
    for _ in 0..100 {
        let list: Value = remote
            .call(method::SCHEDULES_LIST, json!({}))
            .await
            .unwrap();
        level = list["schedules"][0]["level"].as_u64();
        if level.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(level, Some(1), "max_level is 1");
}

#[tokio::test]
async fn config_get_hides_mcp_secrets_from_another_machine() {
    let h = Harness::start(vec![], ON).await;
    let local = Client::connect(&h.local, "test").await.unwrap();
    let _: Value = local
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"mcp": {"servers": {"gh": {"command": "gh-mcp",
                   "env": {"GITHUB_TOKEN": "ghp_secret"}}}}}}),
        )
        .await
        .unwrap();
    let remote = h.remote_client().await;

    let view: Value = remote.call(method::CONFIG_GET, json!({})).await.unwrap();

    assert_eq!(
        view["config"]["mcp"]["servers"]["gh"]["env"]["GITHUB_TOKEN"],
        "(hidden)"
    );
}

// --- Chats -----------------------------------------------------------------------------------

#[tokio::test]
async fn remote_clients_share_chats_but_only_change_their_own() {
    let h = Harness::start(vec![says("Hi."), says("Psst.")], ON).await;
    let (local, mut local_events) = local_client(&h).await;
    let mine = send(&local, None, "hello", &h.workspace()).await;
    run_turn(&local, &mut local_events, &mine).await;
    let secret: ChatSendResult = local
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                incognito: true,
                images: Vec::new(),
                session_id: None,
                text: "a secret".into(),
                cwd: None,
            },
        )
        .await
        .unwrap();
    run_turn(&local, &mut local_events, &secret).await;
    let remote = h.remote_client().await;

    let listed: Vec<Value> = remote.call(method::SESSIONS_LIST, json!({})).await.unwrap();
    let ids: Vec<&str> = listed.iter().filter_map(|s| s["id"].as_str()).collect();
    assert!(ids.contains(&mine.session_id.as_str()), "{listed:?}");
    assert!(
        !ids.contains(&secret.session_id.as_str()),
        "incognito leaked"
    );
    let hidden = remote
        .call::<_, Value>(
            method::SESSIONS_LOAD,
            json!({"session_id": secret.session_id}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(hidden.contains("no session"), "{hidden}");
    let _: Value = remote
        .call(
            method::SESSIONS_LOAD,
            json!({"session_id": mine.session_id}),
        )
        .await
        .unwrap();
    for (name, params) in [
        (
            method::SESSIONS_DELETE,
            json!({"session_id": mine.session_id}),
        ),
        (
            method::SESSION_SET_LEVEL,
            json!({"session_id": mine.session_id, "level": 3}),
        ),
    ] {
        let refused = remote
            .call::<_, Value>(name, params)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("started on the machine"),
            "{name}: {refused}"
        );
    }
    // Its own chat's level is kept within max_level.
    let own = session(&remote, Mode::Agent, AgentLevel::Unrestricted).await;
    let settings: SessionSettings = remote
        .call(method::SESSION_SETTINGS, json!({"session_id": own}))
        .await
        .unwrap();
    assert_eq!(settings.level, AgentLevel::Guarded);
}

// --- Rotation and turning it off ---------------------------------------------------------------

#[tokio::test]
async fn rotating_the_token_disconnects_remote_clients() {
    let h = Harness::start(vec![], ON).await;
    let remote = h.remote_client().await;
    let local = Client::connect(&h.local, "test").await.unwrap();

    let rotated: Value = local.call(method::REMOTE_ROTATE, json!({})).await.unwrap();

    let token = rotated["token"].as_str().unwrap();
    assert!(token.len() >= 32 && token != TOKEN);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        remote
            .call::<_, Value>(method::CORE_STATUS, json!({}))
            .await
            .is_err(),
        "still connected with the old token"
    );
    assert!(Client::connect(&h.remote(TOKEN), "test").await.is_err());
    Client::connect(&h.remote(token), "test").await.unwrap();
}

#[tokio::test]
async fn turning_remote_access_off_takes_effect_at_once() {
    let h = Harness::start(vec![], ON).await;
    let remote = h.remote_client().await;
    let local = Client::connect(&h.local, "test").await.unwrap();

    let _: Value = local
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"remote": {"enabled": false}}}),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        remote
            .call::<_, Value>(method::CORE_STATUS, json!({}))
            .await
            .is_err()
    );
    assert!(!h.listening().await);
}

// --- TLS and the door -------------------------------------------------------------------------

const TLS_ON: &str = "enabled = true\nbind = \"127.0.0.1:{port}\"";

#[tokio::test]
async fn over_tls_the_client_pins_the_cores_certificate() {
    let h = Harness::start(vec![], TLS_ON).await;
    let url = format!("wss://127.0.0.1:{}/rpc", h.port);
    let pin = |pin: &str| Endpoint::Remote {
        url: url.clone(),
        token: TOKEN.into(),
        pin: Some(pin.into()),
        insecure: false,
    };
    let mut connected = None;
    for _ in 0..200 {
        if let Ok(client) = Client::connect(&pin(&h.fingerprint), "test").await {
            connected = Some(client);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let client = connected.expect("the pinned certificate wasn't accepted");
    let _: Value = client.call(method::CORE_STATUS, json!({})).await.unwrap();
    let wrong = "0".repeat(64);
    assert!(matches!(
        Client::connect(&pin(&wrong), "test").await,
        Err(lucy_client::ClientError::WrongCertificate { .. })
    ));
    // Unpinned, the self-signed certificate isn't trusted either.
    let unpinned = Endpoint::Remote {
        url: url.clone(),
        token: TOKEN.into(),
        pin: None,
        insecure: false,
    };
    assert!(Client::connect(&unpinned, "test").await.is_err());
}

#[tokio::test]
async fn web_pages_need_their_origin_allowed() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let h = Harness::start(
        vec![],
        &format!("{ON}\nallowed_origins = [\"https://lucy.example.com\"]"),
    )
    .await;
    h.remote_client().await;
    let with_origin = |origin: &str| {
        let mut request = h.url().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {TOKEN}").parse().unwrap());
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        request
    };

    assert!(
        tokio_tungstenite::connect_async(with_origin("https://evil.example"))
            .await
            .is_err()
    );
    tokio_tungstenite::connect_async(with_origin("https://lucy.example.com"))
        .await
        .unwrap();
}
