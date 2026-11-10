//! External MCP servers, started as real processes. The server under test is Lucy's own
//! `lucy mcp serve`, which offers the built-in tools over stdio like any third-party server.

use std::sync::Arc;
use std::time::Duration;

use lucy_core::config::{McpServer, ToolSettings};
use lucy_core::mcp::McpHost;
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};
use lucy_core::tools::{ToolBox, ToolOutput};
use serde_json::json;

type TestSecrets = Secrets<MemoryStore, fn(&str) -> Option<String>>;

fn no_env(_: &str) -> Option<String> {
    None
}

fn secrets(entries: &[(&str, &str)]) -> TestSecrets {
    let store = MemoryStore::default();
    for (name, value) in entries {
        store.set(name, value).unwrap();
    }
    Secrets::new(store, no_env as fn(&str) -> Option<String>)
}

/// `lucy mcp serve`, working in `dir`.
fn lucy_server(dir: &std::path::Path) -> McpServer {
    let mut server = McpServer::program(
        env!("CARGO_BIN_EXE_lucy"),
        vec!["mcp".into(), "serve".into()],
    );
    server.cwd = Some(dir.to_owned());
    server
}

#[tokio::test]
async fn a_started_server_offers_its_tools_with_their_labels() {
    let dir = tempfile::tempdir().unwrap();
    let host = McpHost::new();

    let server = host
        .server("files", &lucy_server(dir.path()), &secrets(&[]))
        .await
        .unwrap();

    let read = server
        .tools()
        .iter()
        .find(|t| t.name == "read_file")
        .expect("read_file is offered");
    let hints = read.annotations.as_ref().unwrap();
    assert_eq!(hints.read_only_hint, Some(true));
    host.shutdown().await;
}

#[tokio::test]
async fn its_tools_reach_the_model_prefixed_with_the_server_name() {
    let dir = tempfile::tempdir().unwrap();
    let host = McpHost::new();
    let server = host
        .server("files", &lucy_server(dir.path()), &secrets(&[]))
        .await
        .unwrap();

    let settings = ToolSettings {
        builtin: false,
        ..ToolSettings::default()
    };
    let toolbox = ToolBox::open(&settings, dir.path().to_owned(), vec![server])
        .await
        .unwrap();

    let mut names: Vec<String> = toolbox.specs().into_iter().map(|s| s.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "files__edit_file",
            "files__fetch_url",
            "files__list_dir",
            "files__read_file",
            "files__run_command",
            "files__search_files",
            "files__write_file",
        ]
    );
    let ToolOutput {
        text: output,
        is_error,
        ..
    } = toolbox
        .call(&lucy_protocol::ToolCall {
            id: "call_1".into(),
            name: "files__list_dir".into(),
            arguments: json!({}),
        })
        .await;
    assert!(!is_error, "{output}");
    toolbox.close().await;
    host.shutdown().await;
}

#[tokio::test]
async fn calls_run_in_the_servers_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "buy batteries\n").unwrap();
    let host = McpHost::new();
    let server = host
        .server("files", &lucy_server(dir.path()), &secrets(&[]))
        .await
        .unwrap();

    let ToolOutput {
        text: output,
        is_error,
        ..
    } = server
        .call("read_file", json!({"path": "notes.txt"}))
        .await
        .unwrap();

    assert!(!is_error, "{output}");
    assert_eq!(output, "buy batteries\n");
    host.shutdown().await;
}

#[tokio::test]
async fn keyring_entries_and_plain_values_fill_its_environment() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = lucy_server(dir.path());
    config
        .env_keys
        .insert("LUCY_TEST_TOKEN".into(), "demo".into());
    config.env.insert("LUCY_TEST_PLAIN".into(), "plain".into());
    let host = McpHost::new();
    let server = host
        .server("files", &config, &secrets(&[("demo", "s3cret")]))
        .await
        .unwrap();

    let command = if cfg!(windows) {
        "echo %LUCY_TEST_TOKEN%-%LUCY_TEST_PLAIN%"
    } else {
        "echo $LUCY_TEST_TOKEN-$LUCY_TEST_PLAIN"
    };
    let ToolOutput {
        text: output,
        is_error,
        ..
    } = server
        .call("run_command", json!({"command": command}))
        .await
        .unwrap();

    assert!(!is_error, "{output}");
    assert_eq!(output.trim(), "s3cret-plain");
    host.shutdown().await;
}

#[tokio::test]
async fn a_missing_keyring_entry_says_how_to_add_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = lucy_server(dir.path());
    config
        .env_keys
        .insert("GITHUB_TOKEN".into(), "github".into());
    let host = McpHost::new();

    let err = host
        .server("gh", &config, &secrets(&[]))
        .await
        .err()
        .expect("should fail without the key");

    assert!(err.contains("lucy auth set github"), "{err}");
    assert!(err.contains("GITHUB_TOKEN"), "{err}");
}

#[tokio::test]
async fn a_command_that_does_not_exist_is_reported_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = lucy_server(dir.path());
    config.command = Some("lucy-no-such-mcp-server".into());
    let host = McpHost::new();

    let err = host
        .server("ghost", &config, &secrets(&[]))
        .await
        .err()
        .expect("should fail to start");

    assert!(err.contains("lucy-no-such-mcp-server"), "{err}");
}

#[tokio::test]
async fn a_running_server_is_reused() {
    let dir = tempfile::tempdir().unwrap();
    let host = McpHost::new();
    let config = lucy_server(dir.path());

    let first = host.server("files", &config, &secrets(&[])).await.unwrap();
    let second = host.server("files", &config, &secrets(&[])).await.unwrap();

    assert!(Arc::ptr_eq(&first, &second), "a second process was started");
    host.shutdown().await;
}

#[tokio::test]
async fn a_changed_config_restarts_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let host = McpHost::new();
    let mut config = lucy_server(dir.path());

    let first = host.server("files", &config, &secrets(&[])).await.unwrap();
    config
        .env
        .insert("LUCY_TEST_PLAIN".into(), "changed".into());
    let second = host.server("files", &config, &secrets(&[])).await.unwrap();

    assert!(!Arc::ptr_eq(&first, &second), "kept the old process");
    host.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_crashed_server_is_restarted_on_next_use() {
    let dir = tempfile::tempdir().unwrap();
    let host = McpHost::new();
    let config = lucy_server(dir.path());
    let first = host.server("files", &config, &secrets(&[])).await.unwrap();

    // The command's shell is a child of the server, so this kills the server itself.
    let _ = first
        .call("run_command", json!({"command": "kill -9 $PPID"}))
        .await;
    for _ in 0..50 {
        if !first.is_alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!first.is_alive(), "the server should have died");

    let second = host.server("files", &config, &secrets(&[])).await.unwrap();
    assert!(second.is_alive());
    let ToolOutput {
        text: output,
        is_error,
        ..
    } = second.call("list_dir", json!({})).await.unwrap();
    assert!(!is_error, "{output}");
    host.shutdown().await;
}
