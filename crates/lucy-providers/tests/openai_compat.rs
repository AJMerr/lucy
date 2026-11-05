use futures::StreamExt;
use lucy_protocol::{Message, Role};
use lucy_providers::openai_compat::{OpenAiCompat, OpenAiCompatConfig};
use lucy_providers::testing::{FakeResponse, FakeServer};
use lucy_providers::{ChatRequest, Provider, ProviderError, StreamEvent};
use serde_json::json;

const TEXT: &str = include_str!("fixtures/openai_compat/text.sse");

fn provider(server: &FakeServer, api_key: Option<&str>) -> OpenAiCompat {
    OpenAiCompat::new(OpenAiCompatConfig {
        name: "local-qwen".into(),
        base_url: format!("{}/v1", server.url()),
        model: "qwen3:14b".into(),
        api_key: api_key.map(Into::into),
    })
}

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![
            Message::new(Role::System, "You are Lucy."),
            Message::new(Role::User, "hi"),
        ],
    }
}

/// Collect a stream into its events, stopping at the first error.
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
async fn streams_text_and_skips_reasoning() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

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
async fn sends_a_streaming_chat_completions_request() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;

    collect(&provider(&server, Some("sk-test"))).await;

    let [request] = server.requests().try_into().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.headers["authorization"], "Bearer sk-test");
    assert_eq!(
        request.body,
        json!({
            "model": "qwen3:14b",
            "stream": true,
            "messages": [
                {"role": "system", "content": "You are Lucy."},
                {"role": "user", "content": "hi"},
            ],
        })
    );
}

#[tokio::test]
async fn sends_no_auth_header_without_a_key() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;

    collect(&provider(&server, None)).await;

    assert!(!server.requests()[0].headers.contains_key("authorization"));
}

async fn error_for(response: FakeResponse) -> ProviderError {
    let server = FakeServer::start([response]).await;
    let (_, err) = collect(&provider(&server, Some("sk-test"))).await;
    err.expect("expected an error")
}

#[tokio::test]
async fn bad_key_is_an_auth_error_with_the_servers_message() {
    let err = error_for(FakeResponse::json(
        401,
        r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
    ))
    .await;
    assert!(
        matches!(&err, ProviderError::Auth(m) if m.contains("Incorrect API key provided")),
        "{err:?}"
    );
}

#[tokio::test]
async fn rate_limit_and_server_errors_are_unavailable() {
    for status in [429, 500, 503] {
        let err = error_for(FakeResponse::json(
            status,
            r#"{"error":{"message":"slow down"}}"#,
        ))
        .await;
        assert!(
            matches!(&err, ProviderError::Unavailable(m) if m.contains("slow down")),
            "{status}: {err:?}"
        );
    }
}

#[tokio::test]
async fn unknown_model_is_a_rejected_request_with_ollamas_message() {
    // Ollama puts the message directly under `error`.
    let err = error_for(FakeResponse::json(
        404,
        r#"{"error":"model 'qwen9' not found"}"#,
    ))
    .await;
    assert!(
        matches!(&err, ProviderError::Request(m) if m.contains("model 'qwen9' not found")),
        "{err:?}"
    );
}

#[tokio::test]
async fn stream_cut_off_before_finishing_is_a_protocol_error() {
    let truncated: String = TEXT
        .split("\n\n")
        .take(2)
        .map(|b| format!("{b}\n\n"))
        .collect();
    let server = FakeServer::start([FakeResponse::sse(truncated)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert_eq!(events, vec![StreamEvent::TextDelta("Wake up,".into())]);
    assert!(matches!(err, Some(ProviderError::Protocol(_))), "{err:?}");
}

#[tokio::test]
async fn error_sent_inside_the_stream_is_surfaced() {
    let body = "data: {\"error\":{\"message\":\"model runner crashed\"}}\n\n";
    let err = error_for(FakeResponse::sse(body)).await;
    assert!(
        matches!(&err, ProviderError::Unavailable(m) if m.contains("model runner crashed")),
        "{err:?}"
    );
}

#[tokio::test]
async fn unreachable_server_is_unavailable_and_names_the_url() {
    // Bind and drop a listener to get a port nothing is listening on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let provider = OpenAiCompat::new(OpenAiCompatConfig {
        name: "local-qwen".into(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "qwen3:14b".into(),
        api_key: None,
    });

    let (_, err) = collect(&provider).await;

    let expected_url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    assert!(
        matches!(&err, Some(ProviderError::Unavailable(m)) if m.contains(&expected_url) && m.contains("could not connect")),
        "{err:?}"
    );
}

/// A real stream recorded from Ollama + qwen3:14b with the `capture` example
/// (prompt: "Say hello in exactly five words."). ~800 events, mostly reasoning.
#[tokio::test]
async fn real_ollama_qwen3_capture_yields_only_the_answer() {
    let capture = include_str!("fixtures/openai_compat/captured_ollama_qwen3.sse");
    let server = FakeServer::start([FakeResponse::sse(capture).chunk_size(4096)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert!(err.is_none(), "{err:?}");
    assert_eq!(events.last(), Some(&StreamEvent::Done));
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta(text) => Some(text.as_str()),
            StreamEvent::Done => None,
        })
        .collect();
    // Expected value extracted independently with jq from the capture's `content` fields.
    assert_eq!(text, "Hello, how are you doing?");
}
