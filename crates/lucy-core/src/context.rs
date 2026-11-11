//! Keeping chats within the model's context window. What the model gets of a chat starts after
//! the last `/clear`, and a summary stands in for the messages it covers. Near the context size,
//! older turns are summarised (recent ones stay word for word); `/compress` summarises now.
//! The chat itself always keeps every message.

use futures::StreamExt;
use lucy_protocol::{Message, Role};
use lucy_providers::{ChatRequest, Provider, StreamEvent};

use crate::sessions::StoredMessage;

/// Summarise once the prompt passes this share of the context window…
pub const SUMMARISE_AT: f32 = 0.75;
/// Prompts up to this many tokens fit any model worth chatting with (4k context and up).
pub const FITS_ANY: usize = 3_000;

/// …keeping about this share of it, the most recent turns, word for word.
pub const KEEP: f32 = 0.25;

const SUMMARISE: &str = "Summarise the conversation below for yourself, so you can carry on \
                         from it without the full text: who and what it's about, what was \
                         asked, decided and done, facts, names and numbers that matter, and \
                         anything left to do. If it starts with an earlier summary, fold that \
                         in. Plain text, under 300 words, no preamble.";

/// What of a chat the model gets.
pub struct Visible {
    /// The summary standing in for earlier messages.
    pub summary: Option<String>,
    /// The chat's index of the first message in `messages`.
    pub first: usize,
    pub messages: Vec<StoredMessage>,
}

/// The chat after its last `/clear`, with its latest summary in place of what that covers.
pub fn visible(history: Vec<StoredMessage>) -> Visible {
    let cleared = history
        .iter()
        .rposition(|m| m.marker.as_deref() == Some("cleared"))
        .map_or(0, |at| at + 1);
    let summary = history
        .iter()
        .enumerate()
        .skip(cleared)
        .rev()
        .find(|(_, m)| m.marker.as_deref() == Some("summary"))
        .map(|(_, m)| (m.message.content.clone(), m.covers.unwrap_or(0) as usize));
    let first = match &summary {
        Some((_, covers)) => (*covers).max(cleared),
        None => cleared,
    };
    let messages = history
        .into_iter()
        .skip(first)
        .filter(|m| m.marker.is_none())
        .collect();
    Visible {
        summary: summary.map(|(text, _)| text),
        first,
        messages,
    }
}

/// The summary, as the model gets it.
pub fn summary_message(summary: &str) -> Message {
    Message::new(
        Role::System,
        format!("Summary of the conversation before this point:\n\n{summary}"),
    )
}

/// Roughly how many tokens `messages` are (about four characters each).
pub fn estimate(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| {
            let calls: usize = m
                .tool_calls
                .iter()
                .map(|c| c.arguments.to_string().len())
                .sum();
            (m.content.len() + calls) / 4 + 4
        })
        .sum()
}

/// Where to cut `history` so what follows (kept word for word) fits in `keep` tokens: at a
/// message of yours, keeping at least your latest. 0 means there's nothing to summarise.
pub fn cut(history: &[Message], keep: usize) -> usize {
    let mut kept = 0;
    let mut cut = history.len();
    for (at, message) in history.iter().enumerate().rev() {
        kept += estimate(std::slice::from_ref(message));
        if message.role == Role::User {
            if kept > keep && cut < history.len() {
                break;
            }
            cut = at;
        }
    }
    cut
}

/// A summary of `messages` (and `earlier`, the summary before them), written by `provider`.
pub async fn summarise(
    provider: &dyn Provider,
    earlier: Option<&str>,
    messages: &[Message],
) -> Result<String, String> {
    let mut transcript = String::new();
    if let Some(earlier) = earlier {
        transcript += &format!("Earlier summary: {earlier}\n\n");
    }
    for message in messages {
        let (who, text) = match message.role {
            Role::User => ("User", message.content.clone()),
            Role::Assistant if message.tool_calls.is_empty() => ("You", message.content.clone()),
            Role::Assistant => (
                "You",
                format!(
                    "{} [used: {}]",
                    message.content,
                    message
                        .tool_calls
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            // Tool output can be long; its gist is enough.
            Role::Tool => ("Tool result", message.content.chars().take(500).collect()),
            Role::System => continue,
        };
        transcript += &format!("{who}: {}\n\n", text.trim());
    }
    let request = ChatRequest {
        messages: vec![
            Message::new(Role::System, SUMMARISE),
            Message::new(Role::User, transcript),
        ],
        tools: Vec::new(),
    };
    let mut stream = provider
        .stream(request)
        .await
        .map_err(|err| err.to_string())?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let StreamEvent::TextDelta(delta) = event.map_err(|err| err.to_string())? {
            text.push_str(&delta);
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("the summary came back empty".into());
    }
    Ok(text.to_owned())
}
