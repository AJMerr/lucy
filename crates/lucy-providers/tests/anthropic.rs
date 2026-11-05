use futures::StreamExt;
use lucy_protocol::{Message, Role};
use lucy_providers::anthropic::{Anthropic, AnthropicConfig};
use lucy_providers::testing::{FakeResponse, FakeServer};
use lucy_providers::{ChatRequest, Provider, ProviderError, StreamEvent};
use serde_json::json;

const TEXT: &str = include_str!("fixtures/anthropic/text.sse");

fn provider(server: &FakeServer) -> Anthropic {
    Anthropic::new(AnthropicConfig {
        name: "claude".into(),
        base_url: Some(server.url().into()),
        model: "claude-sonnet-5-5".into(),
        api_key: "sk-ant-test".into(),
    })
}

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![
            Message::new(Role::System, "You are Lucy."),
            Message::new(Role::System, "Plain mode is off."),
            Message::new(Role::User, "hi"),
            Message::new(Role::Assistant, "Hey."),
            Message::new(Role::User, "what's up?"),
        ],
    }
}

async fn collect(provider: &impl Provider) -> (Vec<StreamEvent>, Option<ProviderError>) {
    let mut stream = match provider.stream(request()).await {
        Ok(stream) => stream,
        Err(err) => return (vec![], Some(err)),
    };
    let mut events = vec![];
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => events.push(event),
            Err(err) => return (events, Some(err)),
        }
    }
    (events, None)
}

#[tokio::test]
async fn streams_text_and_skips_thinking() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;

    let (events, err) = collect(&provider(&server)).await;

    assert!(err.is_none(), "{err:?}");
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("Wake up,".into()),
            StreamEvent::TextDelta(" choom. 🌙".into()),
            StreamEvent::Done,
        ]
    );
}

#[tokio::test]
async fn sends_a_streaming_messages_request() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;

    collect(&provider(&server)).await;

    let [request] = server.requests().try_into().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.headers["x-api-key"], "sk-ant-test");
    assert_eq!(request.headers["anthropic-version"], "2023-06-01");
    assert_eq!(
        request.body,
        json!({
            "model": "claude-sonnet-5-5",
            "max_tokens": 64000,
            "stream": true,
            "system": "You are Lucy.\n\nPlain mode is off.",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "Hey."},
                {"role": "user", "content": "what's up?"},
            ],
        })
    );
}

/// The fixture's events up to and including the first text delta.
fn first_text_delta_only() -> String {
    let blocks: Vec<&str> = TEXT.split("\n\n").collect();
    let end = blocks
        .iter()
        .position(|b| b.contains("text_delta"))
        .unwrap();
    blocks[..=end].iter().map(|b| format!("{b}\n\n")).collect()
}

#[tokio::test]
async fn bad_key_is_an_auth_error_with_the_servers_message() {
    let server = FakeServer::start([FakeResponse::json(
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"},"request_id":"req_01"}"#,
    )])
    .await;

    let (_, err) = collect(&provider(&server)).await;

    assert!(
        matches!(&err, Some(ProviderError::Auth(m)) if m.contains("invalid x-api-key")),
        "{err:?}"
    );
}

#[tokio::test]
async fn overloaded_is_unavailable() {
    let server = FakeServer::start([FakeResponse::json(
        529,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    )])
    .await;

    let (_, err) = collect(&provider(&server)).await;

    assert!(
        matches!(&err, Some(ProviderError::Unavailable(m)) if m.contains("Overloaded")),
        "{err:?}"
    );
}

#[tokio::test]
async fn error_event_mid_stream_is_surfaced_after_the_text_so_far() {
    let body = first_text_delta_only()
        + "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
    let server = FakeServer::start([FakeResponse::sse(body)]).await;

    let (events, err) = collect(&provider(&server)).await;

    assert_eq!(events, vec![StreamEvent::TextDelta("Wake up,".into())]);
    assert!(
        matches!(&err, Some(ProviderError::Unavailable(m)) if m.contains("Overloaded")),
        "{err:?}"
    );
}

#[tokio::test]
async fn refusal_stop_reason_is_a_refused_error() {
    let body = first_text_delta_only()
        + "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}\n\n"
        + "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let server = FakeServer::start([FakeResponse::sse(body)]).await;

    let (events, err) = collect(&provider(&server)).await;

    assert_eq!(events, vec![StreamEvent::TextDelta("Wake up,".into())]);
    assert!(matches!(err, Some(ProviderError::Refused(_))), "{err:?}");
}

#[tokio::test]
async fn stream_cut_off_before_message_stop_is_a_protocol_error() {
    let server = FakeServer::start([FakeResponse::sse(first_text_delta_only())]).await;

    let (events, err) = collect(&provider(&server)).await;

    assert_eq!(events, vec![StreamEvent::TextDelta("Wake up,".into())]);
    assert!(matches!(err, Some(ProviderError::Protocol(_))), "{err:?}");
}
