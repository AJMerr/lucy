//! Agent mode end to end: a real core, a scripted model that calls tools and reports usage,
//! and the `lucy agent` command.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::{
    ApproveParams, ChatSendParams, ChatSendResult, Decision, Event, Session, SessionParams,
    SessionSettings, SetLevelParams, SetModeParams, SetProfileParams, TurnOutcome, TurnUsage,
    method,
};
use lucy_protocol::{AgentLevel, Mode, Role};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

// --- A scripted model ----------------------------------------------------------------------

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}

fn usage_chunk(input: u64, output: u64) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [], "usage": {"prompt_tokens": input, "completion_tokens": output}})
    )
}

/// A step that calls tools: `(name, arguments)` each.
fn calls(tools: &[(&str, Value)], usage: (u64, u64)) -> FakeResponse {
    let mut body = String::new();
    for (i, (name, args)) in tools.iter().enumerate() {
        body += &chunk(
            json!({"tool_calls": [{"index": i, "id": format!("call_{i}_{name}"), "type": "function",
                   "function": {"name": name, "arguments": args.to_string()}}]}),
            None,
        );
    }
    body += &chunk(json!({}), Some("tool_calls"));
    body += &usage_chunk(usage.0, usage.1);
    body += "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

/// A step that answers in text.
fn says(text: &str, usage: (u64, u64)) -> FakeResponse {
    let body = chunk(json!({"content": text}), None)
        + &chunk(json!({}), Some("stop"))
        + &usage_chunk(usage.0, usage.1)
        + "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

// --- A core with the model ------------------------------------------------------------------

struct Harness {
    dir: tempfile::TempDir,
    endpoint: Endpoint,
    model: FakeServer,
    config_path: PathBuf,
}

impl Harness {
    /// `extra` is appended to the config (e.g. an `[agent]` section).
    async fn start(script: Vec<FakeResponse>, extra: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project")).unwrap();
        let model = FakeServer::start(script).await;
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"fake\"\n\n\
                 [profiles.fake]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n\
                 [profiles.other]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"other-model\"\n\n{extra}",
                model.url(),
                model.url(),
            ),
        )
        .unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-agent-test-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let harness = Self {
            dir,
            endpoint,
            model,
            config_path,
        };
        harness.start_core(Duration::from_secs(3600)).await;
        harness
    }

    async fn start_core(&self, idle_timeout: Duration) {
        let options = CoreOptions {
            endpoint: self.endpoint.clone(),
            config_path: self.config_path.clone(),
            db_path: self.dir.path().join("lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout: Some(idle_timeout),
            token_store: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Client::connect(&self.endpoint, "probe").await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("core never started");
    }

    fn project(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    async fn client(&self) -> Client {
        Client::connect(&self.endpoint, "test").await.unwrap()
    }

    /// The system prompt and message roles/contents of request `n` to the model.
    fn request(&self, n: usize) -> Value {
        self.model.requests()[n].body.clone()
    }
}

async fn agent_session(client: &Client, level: Option<AgentLevel>) -> String {
    let id: String = client
        .call(method::SESSIONS_CREATE, json!({}))
        .await
        .unwrap();
    let _: SessionSettings = client
        .call(
            method::SESSION_SET_MODE,
            SetModeParams {
                session_id: id.clone(),
                mode: Mode::Agent,
            },
        )
        .await
        .unwrap();
    if let Some(level) = level {
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
    }
    id
}

async fn send(client: &Client, session: &str, text: &str, cwd: &Path) -> ChatSendResult {
    client
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                session_id: Some(session.into()),
                text: text.into(),
                cwd: Some(cwd.display().to_string()),
            },
        )
        .await
        .unwrap()
}

/// Events of one turn until it finishes, answering approvals with `answer`.
async fn run_turn(
    client: &Client,
    events: &mut UnboundedReceiver<Event>,
    sent: &ChatSendResult,
    answer: Decision,
) -> (Vec<Event>, TurnOutcome, Option<String>, Option<TurnUsage>) {
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
        if let Event::TurnFinished {
            turn_id,
            outcome,
            error,
            usage,
            ..
        } = &event
            && *turn_id == sent.turn_id
        {
            return (seen, *outcome, error.clone(), usage.clone());
        }
        seen.push(event);
    }
}

fn approvals(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToolApproval { call, .. } => Some(call.name.clone()),
            _ => None,
        })
        .collect()
}

// --- Tests -----------------------------------------------------------------------------------

#[tokio::test]
async fn guarded_agent_changes_files_in_its_folder_without_asking() {
    let h = Harness::start(
        vec![
            calls(
                &[(
                    "write_file",
                    json!({"path": "plan.md", "content": "# Plan\n"}),
                )],
                (100, 10),
            ),
            says("Wrote plan.md.", (120, 5)),
        ],
        "",
    )
    .await;
    let mut client = h.client().await;
    let mut events = client.take_events().unwrap();
    let session = agent_session(&client, None).await;

    let sent = send(&client, &session, "write a plan", &h.project()).await;
    let (seen, outcome, error, usage) = run_turn(&client, &mut events, &sent, Decision::Deny).await;

    assert_eq!(outcome, TurnOutcome::Complete, "{error:?}");
    assert!(approvals(&seen).is_empty(), "asked: {:?}", approvals(&seen));
    assert_eq!(
        std::fs::read_to_string(h.project().join("plan.md")).unwrap(),
        "# Plan\n"
    );
    let usage = usage.unwrap();
    assert_eq!(
        (usage.steps, usage.input_tokens, usage.output_tokens),
        (2, 220, 15)
    );
    let system = h.request(0)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(system.contains("Agent mode (level 1"), "{system}");
}

#[tokio::test]
async fn guarded_agent_still_asks_before_running_commands() {
    let h = Harness::start(
        vec![
            calls(&[("run_command", json!({"command": "echo hi"}))], (10, 1)),
            says("Okay, I won't.", (10, 1)),
        ],
        "",
    )
    .await;
    let mut client = h.client().await;
    let mut events = client.take_events().unwrap();
    let session = agent_session(&client, None).await;

    let sent = send(&client, &session, "say hi", &h.project()).await;
    let (seen, outcome, _, _) = run_turn(&client, &mut events, &sent, Decision::Deny).await;

    assert_eq!(approvals(&seen), ["run_command"]);
    assert_eq!(outcome, TurnOutcome::Complete);
}

#[tokio::test]
async fn the_step_cap_ends_with_a_summary_and_no_more_tools() {
    let h = Harness::start(
        vec![
            calls(&[("list_dir", json!({}))], (10, 1)),
            calls(&[("list_dir", json!({}))], (10, 1)),
            // The wrap-up step tries one more call; it's refused.
            calls(&[("list_dir", json!({}))], (10, 1)),
        ],
        "[agent]\nmax_steps = 2\n",
    )
    .await;
    let mut client = h.client().await;
    let mut events = client.take_events().unwrap();
    let session = agent_session(&client, None).await;

    let sent = send(&client, &session, "keep listing", &h.project()).await;
    let (seen, outcome, error, usage) = run_turn(&client, &mut events, &sent, Decision::Deny).await;

    assert_eq!(outcome, TurnOutcome::Capped);
    assert!(
        error.as_deref().unwrap_or("").contains("step cap"),
        "{error:?}"
    );
    assert_eq!(usage.unwrap().steps, 3, "two steps plus the wrap-up");
    // The wrap-up request says why and asks for a summary, with tools still declared.
    let wrap_up = h.request(2);
    let last = wrap_up["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["role"], "user");
    assert!(
        last["content"].as_str().unwrap().contains("step cap"),
        "{last}"
    );
    assert!(wrap_up["tools"].is_array());
    // The extra call in the wrap-up didn't run.
    let ran = seen
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::ToolFinished {
                    is_error: false,
                    ..
                }
            )
        })
        .count();
    assert_eq!(ran, 2);
    // The history stays valid for the next message: every call has a result.
    let saved: Session = client
        .call(
            method::SESSIONS_LOAD,
            SessionParams {
                session_id: session,
            },
        )
        .await
        .unwrap();
    let call_count: usize = saved.messages.iter().map(|m| m.tool_calls.len()).sum();
    let results = saved
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .count();
    assert_eq!(call_count, results);
}

#[tokio::test]
async fn the_cost_cap_uses_the_reported_usage_and_profile_prices() {
    let h = Harness::start(
        vec![
            // 20 input tokens at $1000 per million = $0.02, over the $0.01 cap.
            calls(&[("list_dir", json!({}))], (20, 0)),
            says("Listed the folder; stopping for budget.", (0, 0)),
        ],
        "[agent]\nmax_cost_usd = 0.01\n",
    )
    .await;
    let config = std::fs::read_to_string(&h.config_path).unwrap().replace(
        "model = \"fake-model\"\n",
        "model = \"fake-model\"\nprice_in = 1000.0\nprice_out = 1000.0\n",
    );
    std::fs::write(&h.config_path, config).unwrap();
    let mut client = h.client().await;
    let mut events = client.take_events().unwrap();
    let session = agent_session(&client, None).await;

    let sent = send(&client, &session, "look around", &h.project()).await;
    let (_, outcome, error, usage) = run_turn(&client, &mut events, &sent, Decision::Deny).await;

    assert_eq!(outcome, TurnOutcome::Capped);
    assert!(
        error.as_deref().unwrap_or("").contains("cost cap"),
        "{error:?}"
    );
    let cost = usage.unwrap().cost_usd.unwrap();
    assert!((cost - 0.02).abs() < 1e-9, "{cost}");
}

#[tokio::test]
async fn assist_mode_is_unchanged_and_asks_for_writes() {
    let h = Harness::start(
        vec![
            calls(
                &[("write_file", json!({"path": "x.md", "content": "x"}))],
                (1, 1),
            ),
            says("Fine.", (1, 1)),
        ],
        "",
    )
    .await;
    let mut client = h.client().await;
    let mut events = client.take_events().unwrap();
    let session: String = client
        .call(method::SESSIONS_CREATE, json!({}))
        .await
        .unwrap();

    let sent = send(&client, &session, "write x", &h.project()).await;
    let (seen, _, _, _) = run_turn(&client, &mut events, &sent, Decision::Deny).await;

    assert_eq!(approvals(&seen), ["write_file"]);
    assert!(!h.project().join("x.md").exists());
}

#[tokio::test]
async fn chat_settings_survive_a_core_restart() {
    let h = Harness::start(vec![], "").await;
    let session = {
        let client = h.client().await;
        let id = agent_session(&client, Some(AgentLevel::Workspace)).await;
        let _: SessionSettings = client
            .call(
                method::SESSION_SET_PROFILE,
                SetProfileParams {
                    session_id: id.clone(),
                    profile: "other".into(),
                },
            )
            .await
            .unwrap();
        id
    };
    // A second core on the same database (the first keeps its socket; use a new one).
    let restarted = Harness {
        dir: tempfile::tempdir().unwrap(),
        endpoint: if cfg!(windows) {
            Endpoint::Pipe(format!("lucy-agent-restart-{}", std::process::id()))
        } else {
            Endpoint::Path(h.dir.path().join("core2.sock"))
        },
        model: FakeServer::start([]).await,
        config_path: h.config_path.clone(),
    };
    let options = CoreOptions {
        endpoint: restarted.endpoint.clone(),
        config_path: h.config_path.clone(),
        db_path: h.dir.path().join("lucy.sqlite"),
        personas_dir: None,
        secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
        idle_timeout: Some(Duration::from_secs(3600)),
        token_store: None,
    };
    tokio::spawn(async move { serve(options).await.unwrap() });
    let client = loop {
        if let Ok(client) = Client::connect(&restarted.endpoint, "test").await {
            break client;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let settings: SessionSettings = client
        .call(
            method::SESSION_SETTINGS,
            SessionParams {
                session_id: session,
            },
        )
        .await
        .unwrap();

    assert_eq!(settings.mode, Mode::Agent);
    assert_eq!(settings.level, AgentLevel::Workspace);
    assert_eq!(settings.profile, "other");
}

// --- `lucy agent` ------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn lucy_agent_runs_in_agent_mode_and_exits_2_at_a_cap() {
    let model = FakeServer::start([
        calls(&[("list_dir", json!({}))], (10, 1)),
        says("Listed it; that's all I had room for.", (10, 1)),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[defaults]\nprofile = \"fake\"\n\n[profiles.fake]\nprovider = \"openai_compat\"\n\
             base_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n[agent]\nmax_steps = 1\n",
            model.url()
        ),
    )
    .unwrap();
    let socket = if cfg!(windows) {
        format!(
            "lucy-agent-cmd-{}",
            dir.path().file_name().unwrap().to_string_lossy()
        )
    } else {
        dir.path()
            .join("run/core.sock")
            .to_string_lossy()
            .into_owned()
    };
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lucy"));
    command
        .args(["agent", "look around"])
        .current_dir(dir.path())
        .env("LUCY_CONFIG", &config)
        .env("LUCY_DATA_DIR", dir.path().join("data"))
        .env("LUCY_SOCKET", &socket)
        .env("LUCY_CORE_IDLE_SECS", "2");
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("that's all I had room for"), "{stdout}");
    assert!(stderr.contains("step cap"), "{stderr}");
    let system = model.requests()[0].body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(system.contains("Agent mode"), "{system}");
}
