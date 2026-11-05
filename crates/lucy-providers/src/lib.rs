//! The seam between Lucy and any language model.
//!
//! Every backend implements [`Provider`]. The core only ever talks to this trait, so swapping
//! models is a config change. Adapters planned for v1: `mock` (tests), `openai_compat`
//! (OpenAI, Ollama, Qwen, OpenRouter, ...), and `anthropic`.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use lucy_protocol::Message;

pub mod mock;

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
}

/// One increment of a streamed model response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    Done,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("request failed: {0}")]
    Request(String),
    #[error("provider returned an unexpected response: {0}")]
    Protocol(String),
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier for this provider instance, e.g. the profile name.
    fn name(&self) -> &str;

    /// Start a streamed chat completion.
    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError>;
}
