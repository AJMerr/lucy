//! A tiny HTTP server for adapter tests: records each request and replies with a scripted
//! response, written in small chunks so stream parsers see events split across reads.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    /// Header names are lowercased.
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct FakeResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    chunk_size: usize,
    stall: bool,
}

impl FakeResponse {
    /// Small enough that most events straddle two reads.
    const DEFAULT_CHUNK: usize = 7;

    /// A 200 `text/event-stream` response.
    pub fn sse(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            body: body.into(),
            chunk_size: Self::DEFAULT_CHUNK,
            stall: false,
        }
    }

    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into(),
            chunk_size: Self::DEFAULT_CHUNK,
            stall: false,
        }
    }

    /// Any other response, e.g. a web page for the fetch tool.
    pub fn body(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type,
            body: body.into(),
            chunk_size: Self::DEFAULT_CHUNK,
            stall: false,
        }
    }

    /// Write the body in pieces of `size` bytes (large captures are slow at the default).
    pub fn chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// After the body, keep the connection open until the client hangs up (a reply that
    /// never finishes), so tests can cancel mid-stream.
    pub fn stall(mut self) -> Self {
        self.stall = true;
        self
    }
}

pub struct FakeServer {
    url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeServer {
    /// Serve `responses` in order, one per incoming request.
    pub async fn start(responses: impl IntoIterator<Item = FakeResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut responses: VecDeque<_> = responses.into_iter().collect();

        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Some(response) = responses.pop_front() {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                handle(socket, response, &recorded).await;
            }
        });

        Self { url, requests }
    }

    /// Base URL, e.g. `http://127.0.0.1:41234`.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

async fn handle(
    mut socket: TcpStream,
    response: FakeResponse,
    recorded: &Mutex<Vec<RecordedRequest>>,
) {
    socket.set_nodelay(true).unwrap();
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        let n = socket.read(&mut buf).await.unwrap();
        raw.extend_from_slice(&buf[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap().split(' ');
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();

    let length: usize = headers
        .get("content-length")
        .map_or(0, |v| v.parse().unwrap());
    while raw.len() < header_end + length {
        let n = socket.read(&mut buf).await.unwrap();
        raw.extend_from_slice(&buf[..n]);
    }
    let body = serde_json::from_slice(&raw[header_end..header_end + length])
        .unwrap_or(serde_json::Value::Null);
    // Record before replying, so the request is visible once the client sees the response end.
    recorded.lock().unwrap().push(RecordedRequest {
        method,
        path,
        headers,
        body,
    });

    let head = format!(
        "HTTP/1.1 {} Fake\r\ncontent-type: {}\r\nconnection: close\r\n\r\n",
        response.status, response.content_type
    );
    socket.write_all(head.as_bytes()).await.unwrap();
    for chunk in response.body.chunks(response.chunk_size) {
        if socket.write_all(chunk).await.is_err() {
            break;
        }
        socket.flush().await.ok();
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    if response.stall {
        let mut buf = [0u8; 1];
        let _ = socket.read(&mut buf).await;
    }
    socket.shutdown().await.ok();
}
