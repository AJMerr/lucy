//! `lucy config add-model`: pick a provider, choose from the models it actually offers, try the
//! one you picked, and save it as a profile.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use lucy_cli::add_model;
use lucy_cli::wizard::Prompter;
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_protocol::rpc::method;
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};

/// Answers in order: a choice by (part of) its text, or typed text.
struct Scripted(VecDeque<String>);

impl Prompter for Scripted {
    fn select(&mut self, prompt: &str, items: &[&str], _default: usize) -> Result<usize> {
        let answer = self
            .0
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected choice: {prompt}"));
        Ok(items
            .iter()
            .position(|item| item.contains(answer.as_str()))
            .unwrap_or_else(|| panic!("no {answer:?} in {items:?}")))
    }

    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String> {
        let answer = self
            .0
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected question: {prompt}"));
        Ok(match (answer.as_str(), default) {
            ("", Some(default)) => default.to_owned(),
            _ => answer,
        })
    }
}

async fn core() -> (tempfile::TempDir, Client) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[defaults]\nprofile = \"fake\"\n\n[profiles.fake]\nprovider = \"openai_compat\"\n\
         base_url = \"http://127.0.0.1:9/v1\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let endpoint = if cfg!(windows) {
        let unique = dir.path().file_name().unwrap().to_string_lossy();
        Endpoint::Pipe(format!("lucy-add-model-{unique}"))
    } else {
        Endpoint::Path(dir.path().join("core.sock"))
    };
    let options = CoreOptions {
        endpoint: endpoint.clone(),
        config_path,
        db_path: dir.path().join("lucy.sqlite"),
        personas_dir: None,
        secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
        idle_timeout: Some(Duration::from_secs(3600)),
        token_store: None,
        clock: None,
        notifier: None,
    };
    tokio::spawn(async move { serve(options).await.unwrap() });
    for _ in 0..200 {
        if let Ok(client) = Client::connect(&endpoint, "test").await {
            return (dir, client);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the core didn't start");
}

fn says_ok() -> FakeResponse {
    FakeResponse::sse(
        "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":null}]}\n\n\
         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
    )
}

#[tokio::test]
async fn adding_an_ollama_model_from_its_list() {
    let ollama = FakeServer::start([
        FakeResponse::json(
            200,
            json!({"models": [{"name": "qwen3:14b"}, {"name": "qwen2.5vl:7b"}]}).to_string(),
        ),
        says_ok(),
        FakeResponse::json(
            200,
            json!({"model_info": {"qwen2vl.context_length": 128000}, "capabilities": ["vision"]})
                .to_string(),
        ),
    ])
    .await;
    let (_dir, client) = core().await;
    let mut answers = Scripted(
        ["Ollama", &format!("{}/v1", ollama.url()), "qwen2.5vl", ""]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let mut out = Vec::new();

    let name = add_model::run(&client, &mut answers, &mut out)
        .await
        .unwrap();

    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("OK"), "shows her test reply: {out}");
    assert_eq!(name, "qwen2-5vl-7b");
    let config: Value = client.call(method::CONFIG_GET, json!({})).await.unwrap();
    let profile = &config["config"]["profiles"][&name];
    assert_eq!(profile["model"], "qwen2.5vl:7b");
    assert_eq!(profile["base_url"], format!("{}/v1", ollama.url()));
    assert_eq!(profile["vision"], true, "the provider says it can see");
    assert_eq!(profile["context_tokens"], 128000);
}

#[tokio::test]
async fn a_model_that_doesnt_answer_isnt_saved_unless_you_say_so() {
    let ollama = FakeServer::start([
        FakeResponse::json(200, json!({"models": [{"name": "qwen3:14b"}]}).to_string()),
        FakeResponse::json(404, json!({"error": "model not found"}).to_string()),
    ])
    .await;
    let (_dir, client) = core().await;
    let mut answers = Scripted(
        [
            "Ollama",
            &format!("{}/v1", ollama.url()),
            "qwen3",
            "",
            "Cancel",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    );
    let mut out = Vec::new();

    let err = add_model::run(&client, &mut answers, &mut out)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("not saved"), "{err}");
    let config: Value = client.call(method::CONFIG_GET, json!({})).await.unwrap();
    assert!(config["config"]["profiles"].get("qwen3-14b").is_none());
}
