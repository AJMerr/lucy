//! What "always" approvals and `auto` rules can't skip: private paths, and file changes outside
//! the folder Lucy may write in.

use std::collections::HashSet;
use std::path::Path;

use lucy_core::config::{ToolRule, ToolSettings};
use lucy_core::tools::{ToolBox, Verdict};
use lucy_protocol::ToolCall;
use serde_json::{Value, json};

/// A chat working in `<tmp>/app`, with `<tmp>/app/secrets/**` private.
async fn toolbox(dir: &Path, rules: &[(&str, ToolRule)]) -> ToolBox {
    let app = dir.join("app");
    std::fs::create_dir_all(app.join("secrets")).unwrap();
    std::fs::write(app.join("secrets/token"), "s3cret").unwrap();
    std::fs::write(app.join("notes.txt"), "hi").unwrap();
    let settings = ToolSettings {
        private_paths: vec![format!("{}/secrets/**", app.display())],
        rules: rules.iter().map(|(n, r)| ((*n).to_owned(), *r)).collect(),
        ..ToolSettings::default()
    };
    ToolBox::open(&settings, app, vec![]).await.unwrap()
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        name: name.into(),
        arguments,
    }
}

fn asks(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Ask { .. })
}

#[tokio::test]
async fn always_doesnt_open_private_paths_or_files_outside_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let toolbox = toolbox(dir, &[]).await;
    let always: HashSet<String> = ["read_file", "write_file"].map(String::from).into();

    let secret = toolbox.verdict(
        &call("read_file", json!({"path": "secrets/token"})),
        &always,
    );
    let notes = toolbox.verdict(&call("read_file", json!({"path": "notes.txt"})), &always);
    let outside = toolbox.verdict(
        &call(
            "write_file",
            json!({"path": "new/../../outside.txt", "content": "x"}),
        ),
        &always,
    );
    let inside = toolbox.verdict(
        &call("write_file", json!({"path": "inside.txt", "content": "x"})),
        &always,
    );

    assert!(asks(&secret), "{secret:?}");
    assert_eq!(notes, Verdict::Run);
    assert!(asks(&outside), "{outside:?}");
    assert_eq!(inside, Verdict::Run);
    toolbox.close().await;
}

#[tokio::test]
async fn an_auto_rule_doesnt_open_private_paths_either() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let toolbox = toolbox(dir, &[("read_file", ToolRule::Auto)]).await;

    let secret = toolbox.verdict(
        &call("read_file", json!({"path": "secrets/token"})),
        &HashSet::new(),
    );
    let outside_home = toolbox.verdict(
        &call("read_file", json!({"path": "/etc/hostname"})),
        &HashSet::new(),
    );

    assert!(asks(&secret), "{secret:?}");
    // Everything else is still up to the rule.
    assert_eq!(outside_home, Verdict::Run);
    toolbox.close().await;
}

#[tokio::test]
async fn fetching_from_this_machine_always_asks() {
    let dir = tempfile::tempdir().unwrap();
    let toolbox = toolbox(dir.path(), &[("fetch_url", ToolRule::Auto)]).await;
    let always: HashSet<String> = ["fetch_url"].map(String::from).into();

    let local = toolbox.verdict(
        &call(
            "fetch_url",
            json!({"url": "http://169.254.169.254/latest/meta-data/"}),
        ),
        &always,
    );
    let public = toolbox.verdict(
        &call("fetch_url", json!({"url": "https://example.com/"})),
        &always,
    );

    assert!(asks(&local), "{local:?}");
    assert_eq!(public, Verdict::Run);
    toolbox.close().await;
}
