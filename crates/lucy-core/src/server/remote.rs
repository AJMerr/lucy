//! Remote access (`[remote]`, ADR 0003): the same JSON-RPC as the local socket, one message
//! per WebSocket text frame at `ws://<bind>/rpc`, for clients on other machines. Every
//! connection must present the token kept under [`REMOTE_TOKEN_KEY`].

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{Core, REMOTE_TOKEN_KEY, serve_client};

/// Start listening if `[remote]` is enabled. Problems are logged, and the core carries on
/// without remote access.
pub(super) async fn start(core: Arc<Core>) -> Option<JoinHandle<()>> {
    let settings = core.config().ok()?.remote;
    if !settings.enabled {
        return None;
    }
    let bind = settings.bind;
    let local = bind.ip().is_loopback();
    if !local && !settings.allow_network {
        eprintln!(
            "remote access not started: {bind} reaches beyond this machine; add \
             `allow_network = true` under [remote] to allow it"
        );
        return None;
    }
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("remote access not started: couldn't listen at {bind}: {err}");
            return None;
        }
    };
    if !local {
        eprintln!(
            "remote access is listening on {bind} without TLS; use it only on a network you \
             trust (e.g. Tailscale), or put something in front that adds TLS"
        );
    }
    eprintln!("remote access at ws://{bind}/rpc");
    let app = Router::new().route("/rpc", get(upgrade)).with_state(core);
    Some(tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    }))
}

/// Check the token, then hand the connection to the usual request handler. Browsers can't
/// set headers on a WebSocket, so besides `Authorization: Bearer <token>` the token may come
/// as a `lucy-token.<token>` subprotocol (offered alongside `lucy`, which is accepted).
async fn upgrade(
    State(core): State<Arc<Core>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let expected = match core.options.secrets.resolve(REMOTE_TOKEN_KEY) {
        Ok(Some(key)) => key.value,
        _ => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "remote access has no token yet; run `lucy remote token` on the core's machine",
            )
                .into_response();
        }
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

/// One remote client, for as long as its WebSocket stays open.
async fn client(core: Arc<Core>, socket: WebSocket) {
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
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            match message {
                Message::Text(text) => {
                    if in_tx.send(text.to_string()).is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    serve_client(Arc::clone(&core), in_rx, out_tx, true).await;
    reader.abort();
    let _ = writer.await;
    core.activity.add(&core.activity.connections, -1);
}
