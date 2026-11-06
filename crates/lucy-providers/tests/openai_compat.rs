use futures::StreamExt;
use lucy_protocol::{Message, Role, ToolCall};
use lucy_providers::openai_compat::{OpenAiCompat, OpenAiCompatConfig};
use lucy_providers::testing::{FakeResponse, FakeServer};
use lucy_providers::{ChatRequest, Provider, ProviderError, StreamEvent, ToolSpec};
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
        tools: vec![],
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
            _ => None,
        })
        .collect();
    // Expected value extracted independently with jq from the capture's `content` fields.
    assert_eq!(text, "Hello, how are you doing?");
}

// --- Tools ---------------------------------------------------------------------------------

fn read_file_tool() -> ToolSpec {
    ToolSpec {
        name: "read_file".into(),
        description: "Read a text file.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        }),
    }
}

async fn collect_request(
    provider: &impl Provider,
    request: ChatRequest,
) -> (Vec<StreamEvent>, Option<ProviderError>) {
    let mut stream = match provider.stream(request).await {
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

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn sends_tools_and_tool_history_as_functions() {
    let server = FakeServer::start([FakeResponse::sse(TEXT)]).await;
    let request = ChatRequest {
        messages: vec![
            Message::new(Role::User, "what's in my notes?"),
            Message::assistant_with_tools(
                "Let me look.",
                vec![call("call_1", "read_file", json!({"path": "notes.txt"}))],
            ),
            Message::tool_result("call_1", "buy milk", false),
            Message::assistant_with_tools(
                "",
                vec![call("call_2", "read_file", json!({"path": "todo.txt"}))],
            ),
            Message::tool_result("call_2", "no such file", true),
        ],
        tools: vec![read_file_tool()],
    };

    collect_request(&provider(&server, None), request).await;

    let [request] = server.requests().try_into().unwrap();
    assert_eq!(
        request.body["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file.",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                },
            },
        }])
    );
    assert_eq!(
        request.body["messages"],
        json!([
            {"role": "user", "content": "what's in my notes?"},
            {"role": "assistant", "content": "Let me look.", "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\":\"notes.txt\"}"},
            }]},
            {"role": "tool", "tool_call_id": "call_1", "content": "buy milk"},
            {"role": "assistant", "content": null, "tool_calls": [{
                "id": "call_2",
                "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\":\"todo.txt\"}"},
            }]},
            // There's no error flag in this API, so failures say so in the text.
            {"role": "tool", "tool_call_id": "call_2", "content": "Error: no such file"},
        ])
    );
}

#[tokio::test]
async fn tool_calls_streamed_in_fragments_arrive_whole() {
    let fixture = include_str!("fixtures/openai_compat/tool_calls.sse");
    // Tiny chunks split events, JSON and multi-byte characters at arbitrary points.
    let server = FakeServer::start([FakeResponse::sse(fixture).chunk_size(5)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert!(err.is_none(), "{err:?}");
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("Let me look.".into()),
            StreamEvent::ToolCall(call("call_1", "read_file", json!({"path": "notes.txt"}))),
            StreamEvent::ToolCall(call("call_2", "list_dir", json!({}))),
            StreamEvent::Done,
        ]
    );
}

fn tool_call_chunk(tool_call: serde_json::Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [tool_call]}, "finish_reason": null}]})
    )
}

const FINISH_TOOL_CALLS: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";

#[tokio::test]
async fn garbled_arguments_are_passed_on_as_a_string() {
    let body = tool_call_chunk(json!({
        "index": 0, "id": "call_1", "type": "function",
        "function": {"name": "read_file", "arguments": "{\"path\": \"notes"},
    })) + FINISH_TOOL_CALLS;
    let server = FakeServer::start([FakeResponse::sse(body)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert!(err.is_none(), "{err:?}");
    assert_eq!(
        events,
        vec![
            StreamEvent::ToolCall(call("call_1", "read_file", json!("{\"path\": \"notes"))),
            StreamEvent::Done,
        ]
    );
}

#[tokio::test]
async fn calls_without_an_id_or_index_get_distinct_ids() {
    // Some local servers send each call whole, with neither an index nor an ID.
    let body = tool_call_chunk(json!({
        "type": "function", "function": {"name": "list_dir", "arguments": "{}"},
    })) + &tool_call_chunk(json!({
        "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a\"}"},
    })) + FINISH_TOOL_CALLS;
    let server = FakeServer::start([FakeResponse::sse(body)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert!(err.is_none(), "{err:?}");
    let calls: Vec<&ToolCall> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 2, "{events:?}");
    assert_eq!(calls[0].name, "list_dir");
    assert_eq!(calls[1].arguments, json!({"path": "a"}));
    assert_ne!(calls[0].id, calls[1].id);
    assert!(!calls[0].id.is_empty());
}

/// A real stream recorded from Ollama + qwen3:14b with `capture ... tools` (prompt: "What does
/// notes.txt say? Read it."). Reasoning, then one whole tool call, then two finish chunks.
#[tokio::test]
async fn real_ollama_qwen3_tool_call_capture_yields_just_the_call() {
    let capture = include_str!("fixtures/openai_compat/captured_ollama_qwen3_tool_call.sse");
    let server = FakeServer::start([FakeResponse::sse(capture).chunk_size(512)]).await;

    let (events, err) = collect(&provider(&server, None)).await;

    assert!(err.is_none(), "{err:?}");
    // Expected values read independently from the capture with jq.
    assert_eq!(
        events,
        vec![
            StreamEvent::ToolCall(call(
                "call_ghdmvvdu",
                "read_file",
                json!({"path": "notes.txt"})
            )),
            StreamEvent::Done,
        ]
    );
}
