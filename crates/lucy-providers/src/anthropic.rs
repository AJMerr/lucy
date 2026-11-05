//! Anthropic Messages API adapter (raw HTTP; there is no official Rust SDK).

use async_trait::async_trait;
use lucy_protocol::Role;
use serde_json::{Value, json};

use crate::http::{check_status, error_message, send_error};
use crate::sse::{self, EventParser, SseEvent};
use crate::{ChatRequest, EventStream, Provider, ProviderError, StreamEvent};

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
        let messages: Vec<Value> = turns
            .iter()
            .map(|m| {
                let role = if m.role == Role::Assistant {
                    "assistant"
                } else {
                    "user"
                };
                json!({"role": role, "content": m.content})
            })
            .collect();

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system.join("\n\n"));
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
            MessageParser,
        ))
    }
}

struct MessageParser;

impl EventParser for MessageParser {
    fn on_event(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|err| ProviderError::Protocol(format!("bad event: {err}")))?;
        let events = match value["type"].as_str() {
            // Thinking, signature and tool-input deltas are skipped; only text is streamed.
            Some("content_block_delta") if value["delta"]["type"] == "text_delta" => {
                let text = value["delta"]["text"].as_str().unwrap_or_default();
                vec![StreamEvent::TextDelta(text.to_owned())]
            }
            Some("message_delta") if value["delta"]["stop_reason"] == "refusal" => {
                return Err(ProviderError::Refused(
                    "stopped by Anthropic's safety classifiers".into(),
                ));
            }
            Some("message_stop") => vec![StreamEvent::Done],
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
