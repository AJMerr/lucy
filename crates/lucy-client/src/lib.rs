//! Client side of Lucy's local-socket protocol, shared by every front end.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

/// Where the core listens: a socket file on Unix, a named pipe on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Path(PathBuf),
    Pipe(String),
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

    pub fn name(&self) -> io::Result<Name<'_>> {
        match self {
            Endpoint::Path(path) => path.as_path().to_fs_name::<GenericFilePath>(),
            Endpoint::Pipe(pipe) => pipe.as_str().to_ns_name::<GenericNamespaced>(),
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(path) => write!(f, "{}", path.display()),
            Endpoint::Pipe(pipe) => write!(f, r"\\.\pipe\{pipe}"),
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
    stream: Arc<Stream>,
    /// Serialises writes so concurrent calls never interleave their lines.
    write_lock: Mutex<()>,
    reader: JoinHandle<()>,
    pending: Pending,
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
        let connect_err = |source| ClientError::Connect {
            endpoint: endpoint.to_string(),
            source,
        };
        let stream = Stream::connect(endpoint.name().map_err(connect_err)?)
            .await
            .map_err(connect_err)?;
        let stream = Arc::new(stream);

        let pending: Pending = Arc::default();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(read_loop(
            Arc::clone(&stream),
            Arc::clone(&pending),
            events_tx,
        ));

        Ok(Self {
            stream,
            write_lock: Mutex::new(()),
            reader,
            pending,
            next_id: AtomicU64::new(1),
            events: Some(events_rx),
            hello: HelloResult {
                protocol_version: 0,
                core_version: String::new(),
                pid: 0,
            },
        })
    }

    /// Connect, starting the core with `spawn` if nothing is listening yet.
    pub async fn connect_or_spawn(
        endpoint: &Endpoint,
        client_name: &str,
        spawn: impl FnOnce() -> io::Result<()>,
    ) -> Result<Self, ClientError> {
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
        {
            let _guard = self.write_lock.lock().await;
            (&*self.stream).write_all(line.as_bytes()).await?;
        }

        let response = rx.await.map_err(|_| ClientError::Disconnected)?;
        if let Some(error) = response.error {
            return Err(error.into());
        }
        serde_json::from_value(response.result.unwrap_or(Value::Null))
            .map_err(|err| ClientError::Protocol(err.to_string()))
    }
}

async fn read_loop(stream: Arc<Stream>, pending: Pending, events: mpsc::UnboundedSender<Event>) {
    let mut lines = BufReader::new(&*stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").is_some() {
            if let Ok(response) = serde_json::from_value::<Response>(value)
                && let Some(tx) = pending.lock().await.remove(&response.id)
            {
                let _ = tx.send(response);
            }
        } else if value["method"] == method::EVENT
            && let Ok(event) = serde_json::from_value::<Event>(value["params"].clone())
        {
            let _ = events.send(event);
        }
    }
    // Dropping the senders wakes every waiting call with `Disconnected`.
    pending.lock().await.clear();
}

impl Drop for Client {
    fn drop(&mut self) {
        // The reader holds a reference to the stream; stop it so the connection closes.
        self.reader.abort();
    }
}
