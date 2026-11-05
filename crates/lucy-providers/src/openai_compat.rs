//! OpenAI-compatible Chat Completions adapter: OpenAI, Ollama, Qwen (DashScope), OpenRouter,
//! LM Studio, llama.cpp, vLLM, ...

use async_trait::async_trait;
use lucy_protocol::Role;
use serde::Deserialize;
use serde_json::json;

use crate::http::{check_status, error_message, send_error};
use crate::sse::{self, EventParser, SseEvent};
use crate::{ChatRequest, EventStream, Provider, ProviderError, StreamEvent};

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
        let messages: Vec<_> = request
            .messages
            .iter()
            .map(|m| json!({"role": role(m.role), "content": m.content}))
            .collect();
        let body = json!({
            "model": self.config.model,
            "stream": true,
            "messages": messages,
        });

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

fn role(role: Role) -> &'static str {
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
}

#[derive(Default)]
struct ChunkParser {
    finished: bool,
}

impl EventParser for ChunkParser {
    fn on_event(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if event.data == "[DONE]" {
            return Ok(vec![StreamEvent::Done]);
        }
        let value: serde_json::Value = serde_json::from_str(&event.data)
            .map_err(|err| ProviderError::Protocol(format!("bad chunk: {err}")))?;
        if let Some(message) = error_message(&value) {
            return Err(ProviderError::Unavailable(message));
        }
        let chunk: Chunk = serde_json::from_value(value)
            .map_err(|err| ProviderError::Protocol(format!("bad chunk: {err}")))?;
        let mut events = Vec::new();
        for choice in chunk.choices {
            if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                events.push(StreamEvent::TextDelta(text));
            }
            self.finished |= choice.finish_reason.is_some();
        }
        Ok(events)
    }

    fn on_eof(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        // Some servers omit `[DONE]`; a finish_reason is enough to know the reply is complete.
        if self.finished {
            Ok(vec![StreamEvent::Done])
        } else {
            Err(ProviderError::Protocol(
                "stream ended before the response finished".into(),
            ))
        }
    }
}
