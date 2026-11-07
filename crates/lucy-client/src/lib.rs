//! Client side of Lucy's protocol, shared by every front end: over the local socket, or over
//! WebSocket to a core on another machine.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use interprocess::local_socket::tokio::{Stream, prelude::*};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name};
use lucy_protocol::rpc::{
    CoreStatus, Event, HelloParams, HelloResult, PROTOCOL_VERSION, Request, Response, RpcError,
    ShutdownParams, ShutdownResult, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// Where the core listens: a socket file on Unix, a named pipe on Windows, or a WebSocket URL
/// for a core on another machine.
#[derive(Clone, PartialEq, Eq)]
pub enum Endpoint {
    Path(PathBuf),
    Pipe(String),
    /// `ws://host:7077/rpc`, and the token that core accepts.
    Remote {
        url: String,
        token: String,
    },
}

impl Endpoint {
    /// `$LUCY_SOCKET` if set; else `$XDG_RUNTIME_DIR/lucy/core.sock` (or a per-user temp
    /// folder) on Unix, and a per-user named pipe on Windows.
    pub fn default_for_user() -> Self {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".into());
        if cfg!(windows) {
            return Endpoint::Pipe(
                std::env::var("LUCY_SOCKET").unwrap_or_else(|_| format!("lucy-core-{user}")),
            );
        }
        if let Some(path) = std::env::var_os("LUCY_SOCKET") {
            return Endpoint::Path(path.into());
        }
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|dir| PathBuf::from(dir).join("lucy"))
            .unwrap_or_else(|| std::env::temp_dir().join(format!("lucy-{user}")));
        Endpoint::Path(dir.join("core.sock"))
    }

    /// The local socket's name; an error for a remote core.
    pub fn name(&self) -> io::Result<Name<'_>> {
        match self {
            Endpoint::Path(path) => path.as_path().to_fs_name::<GenericFilePath>(),
            Endpoint::Pipe(pipe) => pipe.as_str().to_ns_name::<GenericNamespaced>(),
            Endpoint::Remote { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a remote core has no local socket",
            )),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Endpoint::Remote { .. })
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(path) => write!(f, "{}", path.display()),
            Endpoint::Pipe(pipe) => write!(f, r"\\.\pipe\{pipe}"),
            Endpoint::Remote { url, .. } => write!(f, "{url}"),
        }
    }
}

/// Like `Display`: never shows a remote token.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Endpoint::Pipe(pipe) => f.debug_tuple("Pipe").field(pipe).finish(),
            Endpoint::Remote { url, .. } => f.debug_struct("Remote").field("url", url).finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not connect to the Lucy core at {endpoint}: {source}")]
    Connect {
        endpoint: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "the Lucy core at {url} refused this machine's token; run `lucy remote token` on that \
         machine, then `lucy remote connect` here again"
    )]
    Refused { url: String },
    #[error(
        "the running Lucy core speaks protocol v{core} but this client needs v{client}; \
         stop it with `lucy core stop` and try again"
    )]
    VersionMismatch { core: u32, client: u32 },
    #[error("lost connection to the Lucy core")]
    Disconnected,
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("unexpected reply from the Lucy core: {0}")]
    Protocol(String),
    #[error("i/o error talking to the Lucy core: {0}")]
    Io(#[from] io::Error),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

pub struct Client {
    /// Messages for the writer task, which sends them in order.
    outgoing: mpsc::UnboundedSender<String>,
    /// The reader and writer; stopped when the client is dropped.
    tasks: Vec<JoinHandle<()>>,
    pending: Pending,
    /// Set once the reader has stopped, so later calls fail instead of waiting forever.
    closed: Arc<AtomicBool>,
    next_id: AtomicU64,
    events: Option<mpsc::UnboundedReceiver<Event>>,
    hello: HelloResult,
}

impl Client {
    /// Connect and complete the `hello` handshake.
    pub async fn connect(endpoint: &Endpoint, client_name: &str) -> Result<Self, ClientError> {
        let mut client = Self::open(endpoint).await?;
        client.hello = client
            .call(
                method::HELLO,
                HelloParams {
                    protocol_version: PROTOCOL_VERSION,
                    client: client_name.to_owned(),
                },
            )
            .await
            .map_err(|err| match err {
                ClientError::Rpc(rpc) if rpc.message.contains("protocol") => {
                    ClientError::Protocol(rpc.message)
                }
                other => other,
            })?;
        if client.hello.protocol_version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                core: client.hello.protocol_version,
                client: PROTOCOL_VERSION,
            });
        }
        Ok(client)
    }

    /// The running core's version, PID and busy chats. Skips `hello`, so it works whatever
    /// protocol the core speaks (from v4 on).
    pub async fn status(endpoint: &Endpoint) -> Result<CoreStatus, ClientError> {
        Self::open(endpoint)
            .await?
            .call(method::CORE_STATUS, serde_json::json!({}))
            .await
    }

    /// Ask the core to stop. Unless `force`d, a core with replies in progress keeps running
    /// and says which chats are busy. Skips `hello`, like [`Client::status`].
    pub async fn shutdown(endpoint: &Endpoint, force: bool) -> Result<ShutdownResult, ClientError> {
        Self::open(endpoint)
            .await?
            .call(method::CORE_SHUTDOWN, ShutdownParams { force })
            .await
    }

    /// Wait until nothing answers at `endpoint`; false if something still does after `timeout`.
    pub async fn wait_until_gone(endpoint: &Endpoint, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let answered = match endpoint.name() {
                Ok(name) => Stream::connect(name).await.is_ok(),
                Err(_) => false,
            };
            if !answered {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Connect without the handshake.
    async fn open(endpoint: &Endpoint) -> Result<Self, ClientError> {
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (outgoing, mut to_send) = mpsc::unbounded_channel::<String>();
        let incoming = Incoming {
            pending: Arc::clone(&pending),
            closed: Arc::clone(&closed),
            events: events_tx,
        };

        let tasks = match endpoint {
            Endpoint::Remote { url, token } => {
                let (mut sink, mut source) = connect_websocket(url, token).await?.split();
                let writer = tokio::spawn(async move {
                    while let Some(line) = to_send.recv().await {
                        if sink.send(Message::text(line)).await.is_err() {
                            break;
                        }
                    }
                    let _ = sink.close().await;
                });
                let reader = tokio::spawn(async move {
                    while let Some(Ok(message)) = source.next().await {
                        match message {
                            Message::Text(text) => incoming.dispatch(&text).await,
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    incoming.close().await;
                });
                vec![reader, writer]
            }
            local => {
                let connect_err = |source| ClientError::Connect {
                    endpoint: local.to_string(),
                    source,
                };
                let stream = Stream::connect(local.name().map_err(connect_err)?)
                    .await
                    .map_err(connect_err)?;
                let stream = Arc::new(stream);
                let writer = {
                    let stream = Arc::clone(&stream);
                    tokio::spawn(async move {
                        while let Some(line) = to_send.recv().await {
                            if (&*stream).write_all(line.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    })
                };
                let reader = tokio::spawn(async move {
                    let mut lines = BufReader::new(&*stream).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        incoming.dispatch(&line).await;
                    }
                    incoming.close().await;
                });
                vec![reader, writer]
            }
        };

        Ok(Self {
            outgoing,
            tasks,
            pending,
            closed,
            next_id: AtomicU64::new(1),
            events: Some(events_rx),
            hello: HelloResult {
                protocol_version: 0,
                core_version: String::new(),
                pid: 0,
                folder: None,
            },
        })
    }

    /// Connect, starting the core with `spawn` if nothing is listening yet (never for a remote
    /// core, which this machine can't start).
    pub async fn connect_or_spawn(
        endpoint: &Endpoint,
        client_name: &str,
        spawn: impl FnOnce() -> io::Result<()>,
    ) -> Result<Self, ClientError> {
        if endpoint.is_remote() {
            return Self::connect(endpoint, client_name).await;
        }
        match Self::connect(endpoint, client_name).await {
            // A name the OS can never accept (e.g. a Unix socket path over ~108 bytes) won't
            // start working because a core was spawned, so only spawn for other failures.
            Err(ClientError::Connect { ref source, .. })
                if source.kind() != io::ErrorKind::InvalidInput => {}
            other => return other,
        }
        spawn()?;
        let mut last = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            match Self::connect(endpoint, client_name).await {
                Err(err @ ClientError::Connect { .. }) => last = Some(err),
                other => return other,
            }
        }
        Err(last.expect("at least one attempt"))
    }

    pub fn core_version(&self) -> &str {
        &self.hello.core_version
    }

    pub fn core_pid(&self) -> u32 {
        self.hello.pid
    }

    /// For a remote core: the folder on its machine where this client's chats use tools.
    pub fn folder(&self) -> Option<&str> {
        self.hello.folder.as_deref()
    }

    /// Live events from the core. Can be taken once.
    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<Event>> {
        self.events.take()
    }

    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params =
            serde_json::to_value(params).map_err(|err| ClientError::Protocol(err.to_string()))?;
        let mut line = serde_json::to_string(&Request::new(id, method, params))
            .map_err(|err| ClientError::Protocol(err.to_string()))?;
        line.push('\n');

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        // The reader marks the connection closed before clearing `pending`, so a call that
        // missed the clearing sees the flag instead.
        if self.closed.load(Ordering::SeqCst) || self.outgoing.send(line).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(ClientError::Disconnected);
        }

        let response = rx.await.map_err(|_| ClientError::Disconnected)?;
        if let Some(error) = response.error {
            return Err(error.into());
        }
        serde_json::from_value(response.result.unwrap_or(Value::Null))
            .map_err(|err| ClientError::Protocol(err.to_string()))
    }
}

/// Where the reader delivers what arrives.
struct Incoming {
    pending: Pending,
    closed: Arc<AtomicBool>,
    events: mpsc::UnboundedSender<Event>,
}

impl Incoming {
    /// Route one message: a reply to its waiting call, an event to the events channel.
    async fn dispatch(&self, text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if value.get("id").is_some() {
            if let Ok(response) = serde_json::from_value::<Response>(value)
                && let Some(tx) = self.pending.lock().await.remove(&response.id)
            {
                let _ = tx.send(response);
            }
        } else if value["method"] == method::EVENT
            && let Ok(event) = serde_json::from_value::<Event>(value["params"].clone())
        {
            let _ = self.events.send(event);
        }
    }

    /// The connection is gone: wake every waiting call with `Disconnected`.
    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.pending.lock().await.clear();
    }
}

type WebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open a WebSocket to a remote core, sending `token` as a bearer token.
async fn connect_websocket(url: &str, token: &str) -> Result<WebSocket, ClientError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::{Error, http};

    let connect_err = |source| ClientError::Connect {
        endpoint: url.to_owned(),
        source,
    };
    let mut request = url
        .into_client_request()
        .map_err(|err| connect_err(io::Error::new(io::ErrorKind::InvalidInput, err)))?;
    let bearer = format!("Bearer {token}")
        .parse()
        .map_err(|_| ClientError::Refused { url: url.into() })?;
    request
        .headers_mut()
        .insert(http::header::AUTHORIZATION, bearer);
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(Error::Http(response)) if response.status() == http::StatusCode::UNAUTHORIZED => {
            Err(ClientError::Refused { url: url.into() })
        }
        Err(Error::Io(err)) => Err(connect_err(err)),
        Err(err) => Err(connect_err(io::Error::other(err))),
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // The tasks hold the connection; stop them so it closes.
        for task in &self.tasks {
            task.abort();
        }
    }
}
