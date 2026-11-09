//! OpenAI-compatible Chat Completions adapter: OpenAI, Ollama, Qwen (DashScope), OpenRouter,
//! LM Studio, llama.cpp, vLLM, ...

use async_trait::async_trait;
use lucy_protocol::{Message, Role, ToolCall};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{check_status, error_message, send_error};
use crate::sse::{self, EventParser, SseEvent};
use crate::{
    ChatRequest, EventStream, Provider, ProviderError, StreamEvent, Usage, parse_arguments,
};

#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    /// Profile name, reported by [`Provider::name`].
    pub name: String,
    /// API root including the version, e.g. `http://localhost:11434/v1`.
    pub base_url: String,
    pub model: String,
    /// Sent as a bearer token when present.
    pub api_key: Option<String>,
}

pub struct OpenAiCompat {
    config: OpenAiCompatConfig,
    client: reqwest::Client,
}

impl OpenAiCompat {
    pub fn new(config: OpenAiCompatConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let messages: Vec<Value> = request.messages.iter().flat_map(messages).collect();
        let mut body = json!({
            "model": self.config.model,
            "stream": true,
            // Ask for token counts in a final chunk; servers that don't support it ignore it.
            "stream_options": {"include_usage": true},
            "messages": messages,
        });
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema,
                        },
                    })
                })
                .collect();
        }

        let mut http = self.client.post(&url).json(&body);
        if let Some(key) = &self.config.api_key {
            http = http.bearer_auth(key);
        }
        let response = http.send().await.map_err(|err| send_error(&url, err))?;
        Ok(sse::event_stream(
            check_status(response).await?,
            ChunkParser::default(),
        ))
    }
}

/// A message on the wire: usually one, but this API's tool messages can't carry pictures, so a
/// tool result's pictures follow it in a user message.
fn messages(m: &Message) -> Vec<Value> {
    let mut out = vec![message(m)];
    if m.role == Role::Tool && !m.images.is_empty() {
        let mut parts =
            vec![json!({"type": "text", "text": "(The picture from that tool result.)"})];
        parts.extend(m.images.iter().map(image_part));
        out.push(json!({"role": "user", "content": parts}));
    }
    out
}

fn image_part(image: &lucy_protocol::Image) -> Value {
    let url = format!("data:{};base64,{}", image.media_type, image.data);
    json!({"type": "image_url", "image_url": {"url": url}})
}

fn message(m: &Message) -> Value {
    match m.role {
        Role::Tool => {
            // This API has no error flag, so a failure has to say so in its text.
            let content = if m.is_error {
                format!("Error: {}", m.content)
            } else {
                m.content.clone()
            };
            json!({"role": "tool", "tool_call_id": m.tool_call_id, "content": content})
        }
        Role::Assistant if !m.tool_calls.is_empty() => {
            let calls: Vec<Value> = m
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {"name": call.name, "arguments": arguments_text(&call.arguments)},
                    })
                })
                .collect();
            let content = (!m.content.is_empty()).then_some(m.content.as_str());
            json!({"role": "assistant", "content": content, "tool_calls": calls})
        }
        role if !m.images.is_empty() => {
            let mut parts = vec![json!({"type": "text", "text": m.content})];
            parts.extend(m.images.iter().map(image_part));
            json!({"role": role_name(role), "content": parts})
        }
        role => json!({"role": role_name(role), "content": m.content}),
    }
}

/// Arguments go back exactly as the model wrote them, garbled or not.
fn arguments_text(arguments: &Value) -> String {
    match arguments {
        Value::String(raw) => raw.clone(),
        other => other.to_string(),
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct Delta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    #[serde(default)]
    function: FunctionDelta,
}

#[derive(Default, Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

/// A tool call whose fragments are still arriving.
#[derive(Default)]
struct PartialCall {
    index: Option<usize>,
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct ChunkParser {
    finished: bool,
    calls: Vec<PartialCall>,
    usage: Option<Usage>,
}

impl ChunkParser {
    fn add_tool_delta(&mut self, delta: ToolCallDelta) {
        // Streaming servers number each call and send its ID and name once, then argument
        // fragments. Some local servers send whole calls with no index; a name starts a new one.
        let existing = match delta.index {
            Some(index) => self.calls.iter().position(|c| c.index == Some(index)),
            None if delta.function.name.is_some() => None,
            None => self.calls.len().checked_sub(1),
        };
        let at = existing.unwrap_or_else(|| {
            self.calls.push(PartialCall {
                index: delta.index,
                ..PartialCall::default()
            });
            self.calls.len() - 1
        });
        let call = &mut self.calls[at];
        if let Some(id) = delta.id.filter(|id| !id.is_empty()) {
            call.id = id;
        }
        if let Some(name) = delta.function.name {
            call.name.push_str(&name);
        }
        if let Some(arguments) = delta.function.arguments {
            call.arguments.push_str(&arguments);
        }
    }

    /// The finished calls, then `Done`.
    fn finish(&mut self) -> Vec<StreamEvent> {
        let mut events: Vec<StreamEvent> = self
            .calls
            .drain(..)
            .enumerate()
            .map(|(n, call)| {
                StreamEvent::ToolCall(ToolCall {
                    id: if call.id.is_empty() {
                        format!("call_{n}")
                    } else {
                        call.id
                    },
                    name: call.name,
                    arguments: parse_arguments(&call.arguments),
                })
            })
            .collect();
        events.extend(self.usage.take().map(StreamEvent::Usage));
        events.push(StreamEvent::Done);
        events
    }
}

impl EventParser for ChunkParser {
    fn on_event(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if event.data == "[DONE]" {
            return Ok(self.finish());
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|err| ProviderError::Protocol(format!("bad chunk: {err}")))?;
        if let Some(message) = error_message(&value) {
            return Err(ProviderError::Unavailable(message));
        }
        let chunk: Chunk = serde_json::from_value(value)
            .map_err(|err| ProviderError::Protocol(format!("bad chunk: {err}")))?;
        if let Some(usage) = chunk.usage {
            self.usage = Some(Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            });
        }
        let mut events = Vec::new();
        for choice in chunk.choices {
            if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                events.push(StreamEvent::TextDelta(text));
            }
            for delta in choice.delta.tool_calls {
                self.add_tool_delta(delta);
            }
            self.finished |= choice.finish_reason.is_some();
        }
        Ok(events)
    }

    fn on_eof(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        // Some servers omit `[DONE]`; a finish_reason is enough to know the reply is complete.
        if self.finished {
            Ok(self.finish())
        } else {
            Err(ProviderError::Protocol(
                "stream ended before the response finished".into(),
            ))
        }
    }
}
