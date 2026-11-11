//! What a provider offers: its models (for adding one without knowing its ID), each with its
//! context size and whether it can see, where the provider says.

use lucy_providers::models::{Kind, ModelInfo, describe_model, list_models};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::json;

#[tokio::test]
async fn ollama_lists_its_models_and_knows_each_ones_context_and_sight() {
    let tags = FakeResponse::json(
        200,
        json!({"models": [{"name": "qwen3:14b"}, {"name": "qwen2.5vl:7b"}]}).to_string(),
    );
    let server = FakeServer::start([tags]).await;

    let models = list_models(Kind::OpenAiCompat, &format!("{}/v1", server.url()), None)
        .await
        .unwrap();

    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["qwen2.5vl:7b", "qwen3:14b"], "sorted");
    assert_eq!(server.requests()[0].path, "/api/tags");
}

#[tokio::test]
async fn ollama_describes_one_model() {
    let show = FakeResponse::json(
        200,
        json!({
            "model_info": {"general.architecture": "qwen2vl", "qwen2vl.context_length": 128000},
            "capabilities": ["completion", "vision", "tools"],
        })
        .to_string(),
    );
    let server = FakeServer::start([show]).await;

    let info = describe_model(
        Kind::OpenAiCompat,
        &format!("{}/v1", server.url()),
        None,
        "qwen2.5vl:7b",
    )
    .await
    .unwrap();

    assert_eq!(
        info,
        ModelInfo {
            id: "qwen2.5vl:7b".into(),
            name: None,
            context_tokens: Some(128_000),
            vision: Some(true),
        }
    );
    let sent = &server.requests()[0];
    assert_eq!(sent.path, "/api/show");
    assert_eq!(sent.body, json!({"model": "qwen2.5vl:7b"}));
}

#[tokio::test]
async fn openrouter_says_each_models_context_and_whether_it_sees() {
    let not_ollama = FakeResponse::json(404, "{}");
    let models = FakeResponse::json(
        200,
        json!({"data": [
            {"id": "qwen/qwen3-235b-a22b", "name": "Qwen3 235B", "context_length": 131072,
             "architecture": {"input_modalities": ["text"]}},
            {"id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "context_length": 1048576,
             "architecture": {"input_modalities": ["text", "image"]}},
        ]})
        .to_string(),
    )
    .chunk_size(4096);
    let server = FakeServer::start([not_ollama, models]).await;

    let models = list_models(
        Kind::OpenAiCompat,
        &format!("{}/api/v1", server.url()),
        Some("sk-or"),
    )
    .await
    .unwrap();

    assert_eq!(
        models[0],
        ModelInfo {
            id: "google/gemini-2.5-flash".into(),
            name: Some("Gemini 2.5 Flash".into()),
            context_tokens: Some(1_048_576),
            vision: Some(true),
        }
    );
    assert_eq!(models[1].vision, Some(false));
    let requests = server.requests();
    assert_eq!(requests[1].path, "/api/v1/models");
    assert_eq!(requests[1].headers["authorization"], "Bearer sk-or");
}

#[tokio::test]
async fn anthropic_lists_its_models() {
    let models = FakeResponse::json(
        200,
        json!({"data": [
            {"id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5", "type": "model"},
            {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "type": "model"},
        ], "has_more": false})
        .to_string(),
    );
    let server = FakeServer::start([models]).await;

    let models = list_models(Kind::Anthropic, server.url(), Some("sk-ant"))
        .await
        .unwrap();

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "claude-opus-5-5");
    assert_eq!(models[0].name.as_deref(), Some("Claude Opus 5.5"));
    assert_eq!(models[0].vision, Some(true), "Claude models can all see");
    let sent = &server.requests()[0];
    assert!(sent.path.starts_with("/v1/models"), "{}", sent.path);
    assert_eq!(sent.headers["x-api-key"], "sk-ant");
}

#[tokio::test]
async fn a_refused_key_says_so() {
    let refused = FakeResponse::json(
        401,
        json!({"error": {"message": "invalid key"}}).to_string(),
    );
    let server = FakeServer::start([refused]).await;

    let err = list_models(Kind::Anthropic, server.url(), Some("bad"))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("invalid key"), "{err}");
}
