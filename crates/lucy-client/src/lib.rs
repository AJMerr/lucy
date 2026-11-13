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
    /// `wss://host:7077/rpc`, the token that core accepts, and how to trust it.
    Remote {
        url: String,
        token: String,
        /// The SHA-256 fingerprint of the core's own certificate (`lucy remote token` shows
        /// it). `None` checks a `wss://` certificate the usual way (a proxy with a real one).
        pin: Option<String>,
        /// Allow plain `ws://` beyond this machine and Tailscale.
        insecure: bool,
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
    #[error(
        "the Lucy core at {url} has a different certificate from the one this machine paired \
         with; if it changed on purpose (a new install), run `lucy remote connect` again with \
         the fingerprint `lucy remote token` shows there"
    )]
    WrongCertificate { url: String },
    #[error(
        "not sending the token to {url} unencrypted: plain ws:// is only used on this machine or \
         over Tailscale; use wss://, or pass --insecure if the network is trusted"
    )]
    Plaintext { url: String },
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
            Endpoint::Remote {
                url,
                token,
                pin,
                insecure,
            } => {
                let (mut sink, mut source) =
                    connect_websocket(url, token, pin.as_deref(), *insecure)
                        .await?
                        .split();
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
                #[cfg(unix)]
                if let Endpoint::Path(path) = local {
                    trusted_socket(path).map_err(connect_err)?;
                }
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

/// Open a WebSocket to a remote core, sending `token` as a bearer token. Over `wss://` the
/// core's certificate must match `pin`; plain `ws://` only goes to this machine or a
/// Tailscale address (judged by the address actually connected to), unless `insecure`.
async fn connect_websocket(
    url: &str,
    token: &str,
    pin: Option<&str>,
    insecure: bool,
) -> Result<WebSocket, ClientError> {
    use tokio_tungstenite::Connector;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::{Error, http};

    let connect_err = |source| ClientError::Connect {
        endpoint: url.to_owned(),
        source,
    };
    let invalid = |err: String| connect_err(io::Error::new(io::ErrorKind::InvalidInput, err));
    let mut request = url
        .into_client_request()
        .map_err(|err| invalid(err.to_string()))?;
    let secure = match request.uri().scheme_str() {
        Some("wss") => true,
        Some("ws") => false,
        _ => return Err(invalid(format!("{url} isn't a ws:// or wss:// address"))),
    };
    let host = request
        .uri()
        .host()
        .ok_or_else(|| invalid(format!("{url} has no host")))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = request
        .uri()
        .port_u16()
        .unwrap_or(if secure { 443 } else { 80 });
    let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .map_err(connect_err)?;
    if !secure && !insecure && !tcp.peer_addr().is_ok_and(|peer| private_link(peer.ip())) {
        return Err(ClientError::Plaintext { url: url.into() });
    }
    let bearer = format!("Bearer {token}")
        .parse()
        .map_err(|_| ClientError::Refused { url: url.into() })?;
    request
        .headers_mut()
        .insert(http::header::AUTHORIZATION, bearer);
    let connector = match (secure, pin) {
        (true, Some(pin)) => Some(Connector::Rustls(Arc::new(pinned::config(pin)))),
        _ => None,
    };
    match tokio_tungstenite::client_async_tls_with_config(request, tcp, None, connector).await {
        Ok((socket, _)) => Ok(socket),
        Err(Error::Http(response)) if response.status() == http::StatusCode::UNAUTHORIZED => {
            Err(ClientError::Refused { url: url.into() })
        }
        Err(Error::Tls(err)) if err.to_string().contains(pinned::MISMATCH) => {
            Err(ClientError::WrongCertificate { url: url.into() })
        }
        Err(Error::Io(err)) if err.to_string().contains(pinned::MISMATCH) => {
            Err(ClientError::WrongCertificate { url: url.into() })
        }
        Err(Error::Io(err)) => Err(connect_err(err)),
        Err(err) => Err(connect_err(io::Error::other(err))),
    }
}

/// A local socket is only trusted in a folder that's this user's own and that nobody else can
/// write to, and only if this user made it: otherwise another account on a shared machine
/// (with the socket in /tmp) could stand in for the core and read everything sent to it.
#[cfg(unix)]
fn trusted_socket(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // SAFETY: getuid has no preconditions and can't fail.
    let me = unsafe { libc::getuid() };
    let untrusted = |what: String| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{what}, so a core listening there can't be trusted"),
        )
    };
    // Missing: connecting fails anyway, with the usual message.
    if let Some(dir) = path.parent()
        && let Ok(meta) = std::fs::symlink_metadata(dir)
    {
        if meta.file_type().is_symlink() || meta.uid() != me {
            return Err(untrusted(format!(
                "{} isn't a folder of yours",
                dir.display()
            )));
        }
        if meta.permissions().mode() & 0o022 != 0 {
            return Err(untrusted(format!("others can write to {}", dir.display())));
        }
    }
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.uid() != me
    {
        return Err(untrusted(format!(
            "{} belongs to someone else",
            path.display()
        )));
    }
    Ok(())
}

/// This machine, or a Tailscale address (which WireGuard already encrypts).
fn private_link(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback() || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| private_link(v4.into()))
                || ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0]
        }
    }
}

/// Trusting a core by its certificate's fingerprint rather than a certificate authority.
mod pinned {
    use std::sync::Arc;

    use rustls::DigitallySignedStruct;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use sha2::{Digest, Sha256};

    pub const MISMATCH: &str = "the core's certificate doesn't match the pinned fingerprint";

    /// The fingerprint as compared: hex digits only, lowercase.
    pub fn normalize(fingerprint: &str) -> String {
        fingerprint
            .chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }

    #[derive(Debug)]
    struct Pinned {
        fingerprint: String,
        provider: Arc<CryptoProvider>,
    }

    pub fn config(pin: &str) -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = Pinned {
            fingerprint: normalize(pin),
            provider: Arc::clone(&provider),
        };
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("the default protocol versions are supported")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth()
    }

    impl ServerCertVerifier for Pinned {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            let seen: String = Sha256::digest(end_entity.as_ref())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if !self.fingerprint.is_empty() && seen == self.fingerprint {
                Ok(ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General(MISMATCH.into()))
            }
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
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
