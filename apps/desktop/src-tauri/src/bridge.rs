//! The main window's line to the core (`src/lib/rpc/tauri.ts` on the page side): `rpc_call`
//! forwards a call to the core `lucy` would use (this machine's, started if needed, or the
//! remote one after `lucy remote connect`), and every event the core sends goes up as
//! `rpc-event`. `rpc-state` says when the connection is ready or lost; the next call
//! reconnects.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lucy_client::{Client, ClientError};
use lucy_protocol::rpc::{Event, method};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, mpsc};

#[derive(Default)]
pub struct Bridge {
    conn: Mutex<Option<Conn>>,
}

struct Conn {
    /// Tells this connection apart from the ones before and after it.
    id: u64,
    client: Arc<Client>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Call a core method for the page; answers with its result or the core's error message.
#[tauri::command]
pub async fn rpc_call(
    app: AppHandle,
    bridge: tauri::State<'_, Bridge>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    for attempt in 0..2 {
        let client = {
            let mut conn = bridge.conn.lock().await;
            if conn.is_none() {
                *conn = Some(open(&app).await?);
            }
            Arc::clone(&conn.as_ref().expect("just opened").client)
        };
        // Outside the lock: a slow call (MCP sign-in) mustn't hold up the others.
        match client.call::<_, Value>(&method, params.clone()).await {
            Ok(value) => return Ok(value),
            // The core went away (e.g. it was restarted): reconnect once and try again.
            Err(ClientError::Disconnected) if attempt == 0 => forget(&app, &bridge).await,
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("lost the connection to Lucy's core".into())
}

async fn open(app: &AppHandle) -> Result<Conn, String> {
    let mut client = lucy_cli::core_process::connect()
        .await
        .map_err(|err| format!("{err:#}"))?;
    let _: Value = client
        .call(method::SUBSCRIBE, json!({}))
        .await
        .map_err(|err| err.to_string())?;
    let events = client
        .take_events()
        .expect("a new client's events haven't been taken");
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    tauri::async_runtime::spawn(relay(app.clone(), events, id));
    let _ = app.emit("rpc-state", "ready");
    Ok(Conn {
        id,
        client: Arc::new(client),
    })
}

/// Pass the core's events to the page until the connection drops, then say so.
async fn relay(app: AppHandle, mut events: mpsc::UnboundedReceiver<Event>, id: u64) {
    while let Some(event) = events.recv().await {
        let _ = app.emit("rpc-event", &event);
    }
    let bridge = app.state::<Bridge>();
    let mut conn = bridge.conn.lock().await;
    if conn.as_ref().is_some_and(|conn| conn.id == id) {
        *conn = None;
        let _ = app.emit("rpc-state", "lost");
    }
}

async fn forget(app: &AppHandle, bridge: &Bridge) {
    *bridge.conn.lock().await = None;
    let _ = app.emit("rpc-state", "lost");
}
