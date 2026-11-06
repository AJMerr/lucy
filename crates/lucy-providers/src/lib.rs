//! The seam between Lucy and any language model.
//!
//! Every backend implements [`Provider`]. The core only ever talks to this trait, so swapping
//! models is a config change. Adapters planned for v1: `mock` (tests), `openai_compat`
//! (OpenAI, Ollama, Qwen, OpenRouter, ...), and `anthropic`.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use lucy_protocol::{Message, ToolCall};
use serde_json::Value;

pub mod anthropic;
mod http;
pub mod mock;
pub mod openai_compat;
mod sse;
#[cfg(feature = "testing")]
pub mod testing;

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    /// Tools the model may call; empty means a plain chat.
    pub tools: Vec<ToolSpec>,
}

/// A tool offered to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments (an `object` schema).
    pub input_schema: Value,
}

/// One increment of a streamed model response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    /// A complete tool call, emitted once all of its argument fragments have arrived. Arguments
    /// the model garbled are passed on as a JSON string so the caller can report it back.
    ToolCall(ToolCall),
    Done,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Auth(String),
    /// Rejected; retrying unchanged won't help (bad model name, invalid input, ...).
    #[error("request rejected: {0}")]
    Request(String),
    /// Temporary: rate limited, overloaded, server error, or network failure. Worth retrying.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("provider returned an unexpected response: {0}")]
    Protocol(String),
    /// The model declined to answer (e.g. a safety refusal).
    #[error("the model declined to answer: {0}")]
    Refused(String),
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier for this provider instance, e.g. the profile name.
    fn name(&self) -> &str;

    /// Start a streamed chat completion.
    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError>;
}

/// Parse a tool call's streamed argument JSON. No text means no arguments; text that isn't
/// valid JSON is kept as a string so the caller can tell the model what went wrong.
pub(crate) fn parse_arguments(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}
