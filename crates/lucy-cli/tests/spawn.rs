//! Runs the real `lucy` binary: it must start the core on demand and reuse it afterwards.

use std::path::PathBuf;
use std::process::Output;
use std::time::Duration;

use lucy_client::{Client, Endpoint};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::json;

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

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(model: Option<&FakeServer>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if let Some(model) = model {
            let config = dir.path().join("config/config.toml");
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(
                &config,
                format!(
                    "[defaults]\nprofile = \"fake\"\n\n[profiles.fake]\n\
                     provider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n",
                    model.url()
                ),
            )
            .unwrap();
        }
        Self { dir }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("run/core.sock")
    }

    /// A socket path on Unix; on Windows a pipe name unique to this test.
    fn socket_setting(&self) -> String {
        if cfg!(windows) {
            format!(
                "lucy-test-{}",
                self.dir.path().file_name().unwrap().to_string_lossy()
            )
        } else {
            self.socket().to_string_lossy().into_owned()
        }
    }

    fn endpoint(&self) -> Endpoint {
        if cfg!(windows) {
            Endpoint::Pipe(self.socket_setting())
        } else {
            Endpoint::Path(self.socket())
        }
    }

    /// Run `lucy` with every path pointed into the temp folder.
    async fn lucy(&self, args: &[&str]) -> Output {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lucy"));
        command
            .args(args)
            .env("LUCY_CONFIG", self.dir.path().join("config/config.toml"))
            .env("LUCY_DATA_DIR", self.dir.path().join("data"))
            .env("LUCY_SOCKET", self.socket_setting())
            .env("LUCY_CORE_IDLE_SECS", "2");
        tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap()
    }

    async fn core_pid(&self) -> u32 {
        Client::connect(&self.endpoint(), "test")
            .await
            .unwrap()
            .core_pid()
    }

    /// Wait without connecting: every connection counts as activity and would keep the
    /// core awake. On Unix the socket file disappears on exit; on Windows, probe rarely.
    async fn wait_for_core_to_exit(&self) {
        for _ in 0..5 {
            if cfg!(unix) {
                for _ in 0..30 {
                    if !self.socket().exists() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } else {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if Client::connect(&self.endpoint(), "test").await.is_err() {
                    return;
                }
            }
        }
        panic!("core never exited after going idle");
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "lucy failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_starts_the_core_and_later_calls_reuse_it() {
    let model = FakeServer::start([reply(&["Wake up,", " choom."]), reply(&["Still here."])]).await;
    let env = Env::new(Some(&model));

    assert_eq!(stdout(&env.lucy(&["ask", "hi"]).await), "Wake up, choom.\n");
    let first_core = env.core_pid().await;
    assert_ne!(first_core, std::process::id());

    assert_eq!(
        stdout(&env.lucy(&["ask", "still there?"]).await),
        "Still here.\n"
    );
    assert_eq!(
        env.core_pid().await,
        first_core,
        "a second core was started"
    );

    env.wait_for_core_to_exit().await;
    assert!(!env.socket().exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_stale_socket_file_does_not_block_startup() {
    let model = FakeServer::start([reply(&["Back online."])]).await;
    let env = Env::new(Some(&model));
    std::fs::create_dir_all(env.socket().parent().unwrap()).unwrap();
    std::fs::write(env.socket(), "left over from a crash").unwrap();

    assert_eq!(stdout(&env.lucy(&["ask", "hi"]).await), "Back online.\n");
    env.wait_for_core_to_exit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_without_a_config_explains_and_starts_nothing() {
    let env = Env::new(None);

    let output = env.lucy(&["ask", "hi"]).await;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("lucy config init"), "{stderr}");
    assert!(
        Client::connect(&env.endpoint(), "test").await.is_err(),
        "a core was started anyway"
    );
}
