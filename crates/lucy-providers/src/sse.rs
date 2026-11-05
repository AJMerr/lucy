//! Server-Sent Events decoding, shared by the HTTP adapters.

use std::collections::VecDeque;

use futures::{Stream, StreamExt, stream};

use crate::{EventStream, ProviderError, StreamEvent};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Splits a byte stream into SSE events, however the bytes happen to be chunked.
#[derive(Default)]
struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((end, separator)) = find_blank_line(&self.buf) {
            let block: Vec<u8> = self.buf.drain(..end + separator).collect();
            events.extend(parse_block(&block[..end]));
        }
        events
    }
}

/// Position and length of the first blank line (`\n\n` or `\r\n\r\n`).
fn find_blank_line(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn parse_block(block: &[u8]) -> Option<SseEvent> {
    let text = String::from_utf8_lossy(block);
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.to_owned()),
            "data" => data.push(value),
            _ => {}
        }
    }
    (!data.is_empty()).then(|| SseEvent {
        event,
        data: data.join("\n"),
    })
}

/// Turns one provider's SSE events into [`StreamEvent`]s.
pub(crate) trait EventParser: Send + 'static {
    /// Handle one SSE event. Emitting [`StreamEvent::Done`] ends the stream.
    fn on_event(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError>;

    /// The body ended without the parser emitting `Done`.
    fn on_eof(&mut self) -> Result<Vec<StreamEvent>, ProviderError>;
}

struct State<B, P> {
    body: B,
    decoder: SseDecoder,
    parser: P,
    pending: VecDeque<Result<StreamEvent, ProviderError>>,
    finished: bool,
}

impl<B, P> State<B, P> {
    fn emit(&mut self, result: Result<Vec<StreamEvent>, ProviderError>) {
        match result {
            Ok(events) => {
                for event in events {
                    let done = event == StreamEvent::Done;
                    self.pending.push_back(Ok(event));
                    if done {
                        self.finished = true;
                        return;
                    }
                }
            }
            Err(err) => {
                self.pending.push_back(Err(err));
                self.finished = true;
            }
        }
    }
}

pub(crate) fn event_stream(response: reqwest::Response, parser: impl EventParser) -> EventStream {
    stream_from_bytes(response.bytes_stream(), parser)
}

fn stream_from_bytes<B, T, P>(body: B, parser: P) -> EventStream
where
    B: Stream<Item = Result<T, reqwest::Error>> + Unpin + Send + 'static,
    T: AsRef<[u8]>,
    P: EventParser,
{
    let state = State {
        body,
        decoder: SseDecoder::default(),
        parser,
        pending: VecDeque::new(),
        finished: false,
    };
    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }
            match state.body.next().await {
                Some(Ok(bytes)) => {
                    for event in state.decoder.push(bytes.as_ref()) {
                        let result = state.parser.on_event(event);
                        state.emit(result);
                        if state.finished {
                            break;
                        }
                    }
                }
                Some(Err(err)) => state.emit(Err(ProviderError::Unavailable(format!(
                    "connection lost mid-response: {err}"
                )))),
                None => {
                    let result = state.parser.on_eof();
                    state.emit(result);
                    state.finished = true;
                }
            }
        }
    }))
}
