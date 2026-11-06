//! A scripted provider for offline tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;

use crate::{ChatRequest, EventStream, Provider, ProviderError, StreamEvent};

/// Replies with pre-scripted responses in order, streaming each one word by word.
pub struct MockProvider {
    replies: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    pub fn new<I, S>(replies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            replies: Mutex::new(replies.into_iter().map(Into::into).collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Every request this provider has received, for assertions.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let reply =
            self.replies.lock().unwrap().pop_front().ok_or_else(|| {
                ProviderError::Request("mock provider has no replies left".into())
            })?;

        let mut events: Vec<_> = reply
            .split_inclusive(' ')
            .map(|word| Ok(StreamEvent::TextDelta(word.to_owned())))
            .collect();
        events.push(Ok(StreamEvent::Done));
        Ok(Box::pin(stream::iter(events)))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use lucy_protocol::{Message, Role};

    use super::*;

    fn request(text: &str) -> ChatRequest {
        ChatRequest {
            messages: vec![Message::new(Role::User, text)],
            tools: vec![],
        }
    }

    #[tokio::test]
    async fn streams_scripted_reply_then_done() {
        let provider = MockProvider::new(["hey choom"]);
        let events: Vec<_> = provider
            .stream(request("hi"))
            .await
            .unwrap()
            .map(Result::unwrap)
            .collect()
            .await;

        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta("hey ".into()),
                StreamEvent::TextDelta("choom".into()),
                StreamEvent::Done,
            ]
        );
        assert_eq!(provider.requests(), vec![request("hi")]);
    }

    #[tokio::test]
    async fn errors_when_out_of_replies() {
        let provider = MockProvider::new(Vec::<String>::new());
        assert!(provider.stream(request("hi")).await.is_err());
    }
}
