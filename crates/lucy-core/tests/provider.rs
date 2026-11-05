use futures::StreamExt;
use lucy_core::config::{Profile, ProviderKind};
use lucy_core::protocol::{Message, Role};
use lucy_core::provider::build_provider;
use lucy_core::providers::testing::{FakeResponse, FakeServer};
use lucy_core::providers::{ChatRequest, Provider};
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};

const OPENAI_SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
const ANTHROPIC_SSE: &str = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

fn secrets(keys: &[(&str, &str)]) -> Secrets<MemoryStore, impl Fn(&str) -> Option<String>> {
    let store = MemoryStore::default();
    for (name, key) in keys {
        store.set(name, key).unwrap();
    }
    Secrets::new(store, |_: &str| None)
}

fn profile(provider: ProviderKind, base_url: &str, key_name: Option<&str>) -> Profile {
    Profile {
        provider,
        base_url: Some(base_url.into()),
        model: "some-model".into(),
        key_name: key_name.map(Into::into),
    }
}

async fn send(provider: &dyn Provider) {
    let request = ChatRequest {
        messages: vec![Message::new(Role::User, "hi")],
    };
    let mut stream = provider.stream(request).await.unwrap();
    while stream.next().await.is_some() {}
}

#[tokio::test]
async fn openai_compat_profile_uses_chat_completions_with_its_key() {
    let server = FakeServer::start([FakeResponse::sse(OPENAI_SSE)]).await;
    let profile = profile(
        ProviderKind::OpenAiCompat,
        &format!("{}/v1", server.url()),
        Some("openrouter"),
    );

    let provider =
        build_provider("router", &profile, &secrets(&[("openrouter", "sk-or")])).unwrap();
    send(provider.as_ref()).await;

    assert_eq!(provider.name(), "router");
    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.headers["authorization"], "Bearer sk-or");
    assert_eq!(request.body["model"], "some-model");
}

#[tokio::test]
async fn profile_without_key_name_sends_no_key() {
    let server = FakeServer::start([FakeResponse::sse(OPENAI_SSE)]).await;
    let profile = profile(ProviderKind::OpenAiCompat, server.url(), None);

    let provider = build_provider("local", &profile, &secrets(&[])).unwrap();
    send(provider.as_ref()).await;

    assert!(!server.requests()[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn anthropic_profile_uses_messages_api_with_its_key() {
    let server = FakeServer::start([FakeResponse::sse(ANTHROPIC_SSE)]).await;
    let profile = profile(ProviderKind::Anthropic, server.url(), Some("anthropic"));

    let provider =
        build_provider("claude", &profile, &secrets(&[("anthropic", "sk-ant")])).unwrap();
    send(provider.as_ref()).await;

    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.headers["x-api-key"], "sk-ant");
}

#[test]
fn missing_key_says_how_to_add_it() {
    let profile = profile(ProviderKind::Anthropic, "http://unused", Some("anthropic"));

    let err = build_provider("claude", &profile, &secrets(&[]))
        .err()
        .unwrap()
        .to_string();

    assert!(err.contains("profile `claude`"), "{err}");
    assert!(err.contains("lucy auth set anthropic"), "{err}");
    assert!(err.contains("LUCY_KEY_ANTHROPIC"), "{err}");
}
