//! Remote access (`[remote]`, ADR 0003): the same JSON-RPC as the local socket, one message
//! per WebSocket text frame at `/rpc`, for clients on other machines. Every connection must
//! present the token kept under [`REMOTE_TOKEN_KEY`], and may then call only what
//! [`authorize`] allows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{AUTHORIZATION, ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use lucy_protocol::rpc::{Request, RpcError, method};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{ConnState, Core, REMOTE_TOKEN_KEY, serve_client};
use crate::secrets::KeySource;

/// The shortest token accepted: a weaker one (set by hand in `LUCY_KEY_REMOTE_TOKEN`) keeps
/// remote access closed.
const MIN_TOKEN_CHARS: usize = 32;
/// Connections served at once; more wait at the door (closed straight away).
const MAX_CONNECTIONS: usize = 32;
/// How long a new connection may take to finish TLS and send its request.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest message a remote client may send: pictures travel inline, base64.
const MAX_MESSAGE: usize = 32 * 1024 * 1024;

/// Remote access while the core runs: its listener, the token it checks and the clients
/// connected through it.
#[derive(Default)]
pub(super) struct Remote {
    listener: Mutex<Option<JoinHandle<()>>>,
    /// Read from the keyring when listening starts and on rotation, not on every connection.
    token: Mutex<Option<String>>,
    /// Open remote connections, by connection number, so they can be closed.
    connections: Mutex<HashMap<u64, CancellationToken>>,
}

/// Start listening if `[remote]` is enabled. Problems are logged, and the core carries on
/// without remote access.
pub(super) async fn start(core: &Arc<Core>) {
    let Ok(config) = core.config() else {
        return;
    };
    let settings = config.remote;
    if !settings.enabled {
        return;
    }
    let bind = settings.bind;
    let local = bind.ip().is_loopback();
    if !local && !settings.allow_network {
        eprintln!(
            "remote access not started: {bind} reaches beyond this machine; add \
             `allow_network = true` under [remote] to allow it"
        );
        return;
    }
    let token = {
        let core = Arc::clone(core);
        tokio::task::spawn_blocking(move || core.options.secrets.resolve(REMOTE_TOKEN_KEY))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .map(|key| key.value)
    };
    match &token {
        None => eprintln!(
            "remote access has no token yet; run `lucy remote token` on this machine, then \
             restart the core"
        ),
        Some(token) if token.chars().count() < MIN_TOKEN_CHARS => eprintln!(
            "remote access not started: its token is shorter than {MIN_TOKEN_CHARS} \
             characters; `lucy remote token --rotate` makes a strong one"
        ),
        Some(_) => {}
    }
    *core.remote.token.lock().unwrap() = token;
    let tls = match settings.tls {
        true => {
            let core = Arc::clone(core);
            let identity = tokio::task::spawn_blocking(move || {
                crate::remote_tls::load_or_create(&core.options.secrets)
            })
            .await
            .map_err(|err| err.to_string())
            .and_then(|made| made);
            match identity.and_then(|identity| acceptor(&identity).map(|a| (a, identity))) {
                Ok((acceptor, identity)) => {
                    eprintln!(
                        "remote access at wss://{bind}/rpc, certificate fingerprint {}",
                        identity.fingerprint()
                    );
                    Some(acceptor)
                }
                Err(err) => {
                    eprintln!("remote access not started: {err}");
                    return;
                }
            }
        }
        false => {
            if local {
                eprintln!("remote access at ws://{bind}/rpc (no TLS: `tls = false`)");
            } else {
                eprintln!(
                    "remote access is listening on {bind} without TLS (`tls = false`); only do \
                     that behind something that adds it"
                );
            }
            None
        }
    };
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("remote access not started: couldn't listen at {bind}: {err}");
            return;
        }
    };
    let app = Router::new()
        .route("/rpc", get(upgrade))
        .with_state(Arc::clone(core));
    let handle = tokio::spawn(accept(listener, tls, app));
    *core.remote.listener.lock().unwrap() = Some(handle);
}

/// A TLS acceptor for the core's certificate.
fn acceptor(identity: &crate::remote_tls::Identity) -> Result<tokio_rustls::TlsAcceptor, String> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| {
            builder.with_no_client_auth().with_single_cert(
                vec![CertificateDer::from(identity.cert.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key.clone())),
            )
        })
        .map_err(|err| format!("the remote access certificate didn't load: {err}"))?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Accept connections: at most [`MAX_CONNECTIONS`] at once, each with [`HANDSHAKE_TIMEOUT`]
/// to finish TLS and its request, so idle or half-open connections can't pile up.
async fn accept(
    listener: tokio::net::TcpListener,
    tls: Option<tokio_rustls::TlsAcceptor>,
    app: Router,
) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            continue;
        };
        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
            continue;
        };
        let tls = tls.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let _slot = slot;
            match tls {
                Some(tls) => {
                    if let Ok(Ok(stream)) =
                        tokio::time::timeout(HANDSHAKE_TIMEOUT, tls.accept(tcp)).await
                    {
                        serve_connection(stream, app).await;
                    }
                }
                None => serve_connection(tcp, app).await,
            }
        });
    }
}

/// HTTP/1 on one connection, with a deadline for its request headers, then the WebSocket.
async fn serve_connection<I>(io: I, app: Router)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use hyper_util::rt::{TokioIo, TokioTimer};
    let service = hyper_util::service::TowerToHyperService::new(app);
    let _ = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HANDSHAKE_TIMEOUT)
        .serve_connection(TokioIo::new(io), service)
        .with_upgrades()
        .await;
}

/// Stop listening and close every remote connection.
pub(super) fn stop(core: &Core) {
    if let Some(listener) = core.remote.listener.lock().unwrap().take() {
        listener.abort();
    }
    disconnect_all(core);
}

/// `[remote]` changed: stop, then start again with the new settings. Boxed, since serving a
/// remote client can lead back here.
pub(super) fn restart(core: Arc<Core>) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        stop(&core);
        start(&core).await;
    })
}

fn disconnect_all(core: &Core) {
    for (_, closed) in core.remote.connections.lock().unwrap().drain() {
        closed.cancel();
    }
}

/// `remote.rotate`: a new token in the keyring, and every remote client disconnected (they
/// need the new token to come back).
pub(super) async fn rotate(core: &Arc<Core>) -> Result<String, RpcError> {
    let current = {
        let core = Arc::clone(core);
        tokio::task::spawn_blocking(move || core.options.secrets.resolve(REMOTE_TOKEN_KEY))
            .await
            .map_err(|err| RpcError::app(err.to_string()))?
            .map_err(|err| RpcError::app(err.to_string()))?
    };
    if let Some(key) = current
        && let KeySource::Env(var) = key.source
    {
        return Err(RpcError::app(format!(
            "the token comes from {var}, so a new one in the keyring wouldn't be used; change \
             {var} instead (or unset it), then restart the core"
        )));
    }
    let token = new_token().map_err(RpcError::app)?;
    {
        let core = Arc::clone(core);
        let value = token.clone();
        tokio::task::spawn_blocking(move || core.options.secrets.set(REMOTE_TOKEN_KEY, &value))
            .await
            .map_err(|err| RpcError::app(err.to_string()))?
            .map_err(|err| RpcError::app(err.to_string()))?;
    }
    *core.remote.token.lock().unwrap() = Some(token.clone());
    disconnect_all(core);
    Ok(token)
}

/// 32 random bytes from the operating system, as hex.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| format!("couldn't get random bytes for a token: {err}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Check the token, then hand the connection to the usual request handler. Browsers can't
/// set headers on a WebSocket, so besides `Authorization: Bearer <token>` the token may come
/// as a `lucy-token.<token>` subprotocol (offered alongside `lucy`, which is accepted).
async fn upgrade(
    State(core): State<Arc<Core>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // Turned off since the core started: refuse, even before the listener stops.
    let Ok(config) = core.config() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "remote access is off").into_response();
    };
    if !config.remote.enabled {
        return (StatusCode::SERVICE_UNAVAILABLE, "remote access is off").into_response();
    }
    // A web page can open a WebSocket to any address: only ones you named may.
    if let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok())
        && !config
            .remote
            .allowed_origins
            .iter()
            .any(|allowed| allowed.trim_end_matches('/').eq_ignore_ascii_case(origin))
    {
        return (
            StatusCode::FORBIDDEN,
            "this web page isn't allowed to connect; add its origin to [remote] allowed_origins",
        )
            .into_response();
    }
    let expected = core.remote.token.lock().unwrap().clone();
    let Some(expected) = expected.filter(|t| t.chars().count() >= MIN_TOKEN_CHARS) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "remote access has no usable token; run `lucy remote token` on the core's machine",
        )
            .into_response();
    };
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let via_protocol = headers
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .find_map(|offer| offer.trim().strip_prefix("lucy-token."))
        });
    if !bearer
        .or(via_protocol)
        .is_some_and(|token| same(token, &expected))
    {
        return (StatusCode::UNAUTHORIZED, "wrong or missing token").into_response();
    }
    let ws = ws.max_message_size(MAX_MESSAGE).max_frame_size(MAX_MESSAGE);
    let ws = match via_protocol {
        Some(_) => ws.protocols(["lucy"]),
        None => ws,
    };
    ws.on_upgrade(move |socket| client(core, socket))
}

/// Compare without stopping at the first difference, so timing doesn't leak the token.
fn same(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// One remote client, for as long as its WebSocket stays open (or until it's disconnected).
async fn client(core: Arc<Core>, socket: WebSocket) {
    let id = core
        .next_connection
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let closed = CancellationToken::new();
    core.remote
        .connections
        .lock()
        .unwrap()
        .insert(id, closed.clone());
    core.activity.add(&core.activity.connections, 1);
    let (mut sink, mut source) = socket.split();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if sink.send(Message::Text(line.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let reader = {
        let closed = closed.clone();
        tokio::spawn(async move {
            loop {
                let message = tokio::select! {
                    () = closed.cancelled() => break,
                    message = source.next() => message,
                };
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if in_tx.send(text.to_string()).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        })
    };
    serve_client(Arc::clone(&core), in_rx, out_tx, id, true).await;
    reader.abort();
    let _ = writer.await;
    core.remote.connections.lock().unwrap().remove(&id);
    core.activity.add(&core.activity.connections, -1);
}

// --- What a remote client may do ---------------------------------------------------------------

/// What a remote client may call. Anything not named here is refused, so a new method stays
/// closed to other machines until it's decided otherwise. Chats are shared (it may read and
/// continue them) except other clients' incognito chats, which it never sees; it may only
/// change chats started from another machine.
pub(super) fn authorize(
    core: &Core,
    state: &Mutex<ConnState>,
    request: &Request,
) -> Result<(), RpcError> {
    use method::*;
    let params = &request.params;
    let chat = params
        .get("session_id")
        .or_else(|| params.get("prefix"))
        .and_then(Value::as_str);
    let key = match chat {
        Some(prefix) => Some(visible_chat(core, state, prefix)?),
        None => None,
    };
    let needs_config = || {
        if core.config().is_ok_and(|config| config.remote.allow_config) {
            Ok(())
        } else {
            Err(RpcError::app(
                "from another machine you can't change settings, keys, the persona, memory or \
                 tasks, or use stored keys to list models; set `[remote] allow_config = true` \
                 on the machine that runs Lucy to allow it",
            ))
        }
    };
    let own_chat = || match &key {
        Some(key) if started_remotely(core, key) => Ok(()),
        Some(_) => Err(RpcError::app(
            "this chat was started on the machine that runs Lucy; from another machine you can \
             read and continue it, but not change, clear or delete it",
        )),
        None => Err(RpcError::app("name the chat (`session_id`)")),
    };
    match request.method.as_str() {
        HELLO | CORE_STATUS | SUBSCRIBE | CHAT_SEND | SESSIONS_CREATE | SESSIONS_LIST
        | SESSIONS_RESOLVE | SESSIONS_LOAD | SESSION_SETTINGS | PROFILES_LIST | TOOLS_LIST
        | SETUP_PRESETS | PERSONA_GET | CONFIG_GET | KEYS_STATUS | SCHEDULES_LIST
        | IMAGES_GENERATE | IMAGES_GET | MEMORY_GET | TOOL_APPROVE => Ok(()),
        // Stopping a reply it asked for, even in a shared chat.
        CHAT_CANCEL => match &key {
            Some(key) if state.lock().unwrap().own_sessions.contains(key) => Ok(()),
            _ => own_chat(),
        },
        CHAT_CLEAR | CHAT_COMPRESS | SESSIONS_RENAME | SESSIONS_DELETE | SESSION_SET_PLAIN
        | SESSION_SET_MODE | SESSION_SET_LEVEL | SESSION_UNDO => own_chat(),
        SESSION_SET_PROFILE => {
            own_chat()?;
            if params.get("make_default").and_then(Value::as_bool) == Some(true) {
                needs_config()?;
            }
            Ok(())
        }
        CONFIG_UPDATE | KEYS_SET | KEYS_REMOVE | PERSONA_SET | PERSONA_RESET | MEMORY_SET
        | SCHEDULES_UPDATE | SCHEDULES_DELETE | MODELS_LIST | MODELS_TRY => needs_config(),
        CORE_SHUTDOWN => Err(RpcError::app(
            "a remote client can't stop the core (it couldn't start it again); stop it on its \
             own machine",
        )),
        MCP_LOGIN => Err(RpcError::app(
            "signing in opens a browser on the machine that runs Lucy, so do it there",
        )),
        REMOTE_ROTATE => Err(RpcError::app(
            "replace the token on the machine that runs Lucy: `lucy remote token --rotate`",
        )),
        other => Err(RpcError::app(format!(
            "`{other}` isn't available from another machine"
        ))),
    }
}

/// The chat `prefix` names, unless it's someone else's incognito chat (which a remote client
/// doesn't learn exists).
fn visible_chat(core: &Core, state: &Mutex<ConnState>, prefix: &str) -> Result<String, RpcError> {
    let key = core.resolve_key(prefix)?;
    let hidden = core.incognito.lock().unwrap().contains(&key)
        && !state.lock().unwrap().incognito.contains(&key);
    if hidden {
        return Err(RpcError::app(format!("no session matches `{prefix}`")));
    }
    Ok(key)
}

/// Whether a chat was started from another machine.
fn started_remotely(core: &Core, key: &str) -> bool {
    key.parse()
        .ok()
        .and_then(|id| core.store.lock().unwrap().is_remote(&id).ok())
        .unwrap_or(false)
}

/// `config.get` for a remote client without `allow_config`: MCP servers' plain environment
/// variables and headers hidden, since people do put tokens in them.
pub(super) fn redact(config: &mut Value) {
    let Some(servers) = config
        .pointer_mut("/mcp/servers")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for server in servers.values_mut() {
        for field in ["env", "headers"] {
            if let Some(values) = server.get_mut(field).and_then(Value::as_object_mut) {
                for value in values.values_mut() {
                    *value = Value::String("(hidden)".into());
                }
            }
        }
    }
}
