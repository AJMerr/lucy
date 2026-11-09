//! Anthropic Messages API adapter (raw HTTP; there is no official Rust SDK).

use async_trait::async_trait;
use lucy_protocol::{Message, Role, ToolCall};
use serde_json::{Value, json};

use crate::http::{check_status, error_message, send_error};
use crate::sse::{self, EventParser, SseEvent};
use crate::{
    ChatRequest, EventStream, Provider, ProviderError, StreamEvent, Usage, parse_arguments,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
/// Streaming avoids HTTP timeouts, so give the model room rather than truncating replies.
const MAX_TOKENS: u32 = 64_000;

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// Profile name, reported by [`Provider::name`].
    pub name: String,
    /// API root without the version path; defaults to `https://api.anthropic.com`.
    pub base_url: Option<String>,
    pub model: String,
    pub api_key: String,
}

pub struct Anthropic {
    config: AnthropicConfig,
    client: reqwest::Client,
}

impl Anthropic {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError> {
        let base = self.config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
        let url = format!("{}/v1/messages", base.trim_end_matches('/'));

        // System prompts go in the top-level `system` field, not in `messages`.
        let (system, turns): (Vec<_>, Vec<_>) = request
            .messages
            .iter()
            .partition(|m| m.role == Role::System);
        let system: Vec<&str> = system.iter().map(|m| m.content.as_str()).collect();

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "messages": messages(&turns),
        });
        if !system.is_empty() {
            body["system"] = json!(system.join("\n\n"));
        }
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.input_schema,
                    })
                })
                .collect();
        }

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|err| send_error(&url, err))?;
        Ok(sse::event_stream(
            check_status(response).await?,
            MessageParser::default(),
        ))
    }
}

fn messages(turns: &[&Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in turns {
        match m.role {
            Role::Tool => {
                let mut result = json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id,
                    "content": m.content,
                });
                if m.is_error {
                    result["is_error"] = json!(true);
                }
                // Results of one turn's calls share a single user message.
                match out.last_mut() {
                    Some(last) if last["role"] == "user" && last["content"].is_array() => {
                        last["content"].as_array_mut().unwrap().push(result);
                    }
                    _ => out.push(json!({"role": "user", "content": [result]})),
                }
            }
            Role::Assistant if !m.tool_calls.is_empty() => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({"type": "text", "text": m.content}));
                }
                for call in &m.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input(&call.arguments),
                    }));
                }
                out.push(json!({"role": "assistant", "content": blocks}));
            }
            Role::Assistant => out.push(json!({"role": "assistant", "content": m.content})),
            _ if !m.images.is_empty() => {
                // Anthropic recommends pictures before the text about them.
                let mut blocks: Vec<Value> = m
                    .images
                    .iter()
                    .map(|image| {
                        json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": image.media_type,
                                "data": image.data,
                            },
                        })
                    })
                    .collect();
                blocks.push(json!({"type": "text", "text": m.content}));
                out.push(json!({"role": "user", "content": blocks}));
            }
            _ => out.push(json!({"role": "user", "content": m.content})),
        }
    }
    out
}

/// `input` must be an object; garbled arguments (kept as a string) are wrapped in one.
fn input(arguments: &Value) -> Value {
    if arguments.is_object() {
        arguments.clone()
    } else {
        json!({"_unparsed": arguments})
    }
}

/// A tool_use block whose input JSON is still arriving.
struct PartialCall {
    id: String,
    name: String,
    input: String,
}

#[derive(Default)]
struct MessageParser {
    call: Option<PartialCall>,
    usage: Option<Usage>,
}

impl EventParser for MessageParser {
    fn on_event(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|err| ProviderError::Protocol(format!("bad event: {err}")))?;
        let delta = &value["delta"];
        let events = match value["type"].as_str() {
            // Thinking and signature deltas are skipped; text streams, tool calls arrive whole.
            Some("content_block_delta") if delta["type"] == "text_delta" => {
                let text = delta["text"].as_str().unwrap_or_default();
                vec![StreamEvent::TextDelta(text.to_owned())]
            }
            Some("content_block_start") if value["content_block"]["type"] == "tool_use" => {
                let block = &value["content_block"];
                self.call = Some(PartialCall {
                    id: block["id"].as_str().unwrap_or_default().to_owned(),
                    name: block["name"].as_str().unwrap_or_default().to_owned(),
                    input: String::new(),
                });
                vec![]
            }
            Some("content_block_delta") if delta["type"] == "input_json_delta" => {
                if let Some(call) = &mut self.call {
                    call.input
                        .push_str(delta["partial_json"].as_str().unwrap_or_default());
                }
                vec![]
            }
            Some("content_block_stop") => match self.call.take() {
                Some(call) => vec![StreamEvent::ToolCall(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: parse_arguments(&call.input),
                })],
                None => vec![],
            },
            Some("message_delta") if delta["stop_reason"] == "refusal" => {
                return Err(ProviderError::Refused(
                    "stopped by Anthropic's safety classifiers".into(),
                ));
            }
            // Output tokens so far (cumulative), sent with the stop reason.
            Some("message_delta") => {
                if let (Some(usage), Some(output)) =
                    (&mut self.usage, value["usage"]["output_tokens"].as_u64())
                {
                    usage.output_tokens = output;
                }
                vec![]
            }
            Some("message_start") => {
                let usage = &value["message"]["usage"];
                let count = |field: &str| usage[field].as_u64().unwrap_or(0);
                self.usage = Some(Usage {
                    input_tokens: count("input_tokens")
                        + count("cache_creation_input_tokens")
                        + count("cache_read_input_tokens"),
                    output_tokens: count("output_tokens"),
                });
                vec![]
            }
            Some("message_stop") => {
                let mut events: Vec<StreamEvent> = self
                    .usage
                    .take()
                    .map(StreamEvent::Usage)
                    .into_iter()
                    .collect();
                events.push(StreamEvent::Done);
                events
            }
            Some("error") => {
                let message = error_message(&value).unwrap_or_else(|| event.data.clone());
                return Err(ProviderError::Unavailable(message));
            }
            _ => vec![],
        };
        Ok(events)
    }

    fn on_eof(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        Err(ProviderError::Protocol(
            "stream ended before message_stop".into(),
        ))
    }
}
