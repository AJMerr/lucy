//! The real `lucy` binary talking to a core on "another machine" over WebSocket: no local
//! config, no local core, the token from `LUCY_KEY_REMOTE_CLIENT`.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};
use lucy_core::server::{CoreOptions, REMOTE_TOKEN_KEY, serve};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};

const TOKEN: &str = "e2e-token-0123456789abcdef";

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}

fn calls(name: &str, args: Value) -> FakeResponse {
    let body = chunk(
        json!({"tool_calls": [{"index": 0, "id": format!("call_{name}"), "type": "function",
               "function": {"name": name, "arguments": args.to_string()}}]}),
        None,
    ) + &chunk(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

fn says(text: &str) -> FakeResponse {
    let body = chunk(json!({"content": text}), None)
        + &chunk(json!({}), Some("stop"))
        + "data: [DONE]\n\n";
    FakeResponse::sse(body)
}

/// The "desktop": a core with remote access on, and an empty "laptop" next to it.
struct Desktop {
    dir: tempfile::TempDir,
    model: FakeServer,
    port: u16,
}

impl Desktop {
    async fn start(script: Vec<FakeResponse>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("desktop/workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("marker.txt"), "on the desktop").unwrap();
        std::fs::create_dir_all(dir.path().join("laptop/project")).unwrap();
        std::fs::write(dir.path().join("laptop/project/decoy.txt"), "on the laptop").unwrap();
        let model = FakeServer::start(script).await;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config_path = dir.path().join("desktop/config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"fake\"\n\n\
                 [profiles.fake]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n\
                 [agent]\nworkspace = {:?}\n\n\
                 [remote]\nenabled = true\nbind = \"127.0.0.1:{port}\"\n",
                model.url(),
                workspace.display().to_string(),
            ),
        )
        .unwrap();
        let local = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-desktop-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("desktop/core.sock"))
        };
        let keyring = MemoryStore::default();
        keyring.set(REMOTE_TOKEN_KEY, TOKEN).unwrap();
        let options = CoreOptions {
            endpoint: local,
            config_path,
            db_path: dir.path().join("desktop/lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(keyring), Box::new(|_: &str| None)),
            idle_timeout: Some(Duration::from_secs(3600)),
            token_store: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        let desktop = Self { dir, model, port };
        let endpoint = Endpoint::Remote {
            url: desktop.url(),
            token: TOKEN.into(),
        };
        for _ in 0..200 {
            if Client::connect(&endpoint, "probe").await.is_ok() {
                return desktop;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("remote listener never came up");
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/rpc", self.port)
    }

    fn laptop(&self) -> PathBuf {
        self.dir.path().join("laptop")
    }

    /// Run `lucy` as the laptop would: its own empty config and data folders, no core of its
    /// own, working in `laptop/project`.
    async fn lucy(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let laptop = self.laptop();
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lucy"));
        command
            .args(args)
            .current_dir(laptop.join("project"))
            .env("LUCY_CONFIG", laptop.join("config/config.toml"))
            .env("LUCY_DATA_DIR", laptop.join("data"))
            .env("LUCY_SOCKET", local_socket(&laptop))
            .env_remove("LUCY_REMOTE")
            .env_remove("LUCY_KEY_REMOTE_CLIENT")
            // Keep away from the real keyring (Secret Service) on Linux.
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .stdin(std::process::Stdio::null());
        for (name, value) in env {
            command.env(name, value);
        }
        tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap()
    }
}

/// Where a local core would listen on the laptop (nothing should).
fn local_socket(laptop: &Path) -> String {
    if cfg!(windows) {
        format!(
            "lucy-laptop-{}",
            laptop
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
        )
    } else {
        laptop.join("core.sock").display().to_string()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn ask_answers_from_the_remote_core() {
    let desktop = Desktop::start(vec![says("Hello from the desktop")]).await;

    let out = desktop
        .lucy(
            &["--remote", &desktop.url(), "ask", "hi"],
            &[("LUCY_KEY_REMOTE_CLIENT", TOKEN)],
        )
        .await;

    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "Hello from the desktop");
    assert!(
        !desktop.laptop().join("core.sock").exists(),
        "started a local core"
    );
}

#[tokio::test]
async fn agent_tasks_run_in_the_remote_cores_folder() {
    let desktop = Desktop::start(vec![
        calls("list_dir", json!({"path": "."})),
        says("There's a marker file."),
    ])
    .await;

    let out = desktop
        .lucy(
            &["agent", "what's in the folder?"],
            &[
                ("LUCY_REMOTE", &desktop.url()),
                ("LUCY_KEY_REMOTE_CLIENT", TOKEN),
            ],
        )
        .await;

    assert!(out.status.success(), "{}", text(&out.stderr));
    let requests = desktop.model.requests();
    let tool_result = requests[1].body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .map(|m| m["content"].to_string())
        .expect("the listing went back to the model");
    assert!(tool_result.contains("marker.txt"), "{tool_result}");
    assert!(!tool_result.contains("decoy.txt"), "{tool_result}");
}

#[tokio::test]
async fn a_wrong_token_says_how_to_fix_it() {
    let desktop = Desktop::start(vec![]).await;

    let out = desktop
        .lucy(
            &["--remote", &desktop.url(), "ask", "hi"],
            &[("LUCY_KEY_REMOTE_CLIENT", "nope")],
        )
        .await;

    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("refused this machine's token"), "{err}");
    assert!(err.contains("lucy remote token"), "{err}");
}

#[tokio::test]
async fn without_a_token_it_says_to_connect() {
    let desktop = Desktop::start(vec![]).await;

    let out = desktop
        .lucy(&["--remote", &desktop.url(), "ask", "hi"], &[])
        .await;

    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("lucy remote connect"), "{err}");
}
