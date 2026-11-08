//! Her conversation: one ongoing chat of her own (titled "Sprite"), with whichever core `lucy`
//! uses (this machine's, started if needed, or a remote one after `lucy remote connect`). Its
//! events go to her page as `core-event`; it sends messages and approvals back. Her whole chat
//! is in the main window (app_window.rs), which has its own connection (bridge.rs).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lucy_client::{Client, ClientError};
use lucy_protocol::rpc::{
    ApproveParams, ChatSendParams, ChatSendResult, Decision, Event, RenameParams, ResolveParams,
    method,
};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, mpsc};

/// The title of her chat in `/sessions`.
const TITLE: &str = "Sprite";

/// The connection to the core, opened on first use and again after it drops.
#[derive(Default)]
pub struct Chat {
    conn: Mutex<Option<Conn>>,
}

struct Conn {
    /// Tells this connection apart from the ones before and after it.
    id: u64,
    client: Client,
    session: String,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Connect now, so the core is up (and her chat found) before the first message.
pub fn connect_in_background(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let chat = app.state::<Chat>();
        let mut conn = chat.conn.lock().await;
        if conn.is_none() {
            match open(&app).await {
                Ok(opened) => *conn = Some(opened),
                Err(err) => eprintln!("lucy-desktop: {err}"),
            }
        }
    });
}

async fn open(app: &AppHandle) -> Result<Conn, String> {
    let mut client = lucy_cli::core_process::connect()
        .await
        .map_err(|err| format!("{err:#}"))?;
    let session = her_session(&client).await.map_err(|err| err.to_string())?;
    // Every chat's events, filtered to hers below, so a reply in her chat shows up even when
    // it was sent from elsewhere (e.g. `lucy -r` in a terminal).
    let _: Value = client
        .call(method::SUBSCRIBE, json!({}))
        .await
        .map_err(|err| err.to_string())?;
    let events = client
        .take_events()
        .expect("a new client's events haven't been taken");
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    tauri::async_runtime::spawn(relay(app.clone(), events, session.clone(), id));
    Ok(Conn {
        id,
        client,
        session,
    })
}

/// Her chat: the one she used last time, or a new one if it's gone (or never existed).
async fn her_session(client: &Client) -> Result<String, ClientError> {
    let file = session_file();
    if let Some(saved) = file
        .as_ref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| !id.is_empty())
        && let Ok(id) = client
            .call::<_, String>(method::SESSIONS_RESOLVE, ResolveParams { prefix: saved })
            .await
    {
        return Ok(id);
    }
    let id: String = client.call(method::SESSIONS_CREATE, json!({})).await?;
    let _: Value = client
        .call(
            method::SESSIONS_RENAME,
            RenameParams {
                session_id: id.clone(),
                title: TITLE.into(),
            },
        )
        .await?;
    if let Some(file) = file {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(err) = std::fs::write(&file, &id) {
            eprintln!(
                "lucy-desktop: couldn't remember her chat in {}: {err}",
                file.display()
            );
        }
    }
    Ok(id)
}

/// `<data dir>/sprite-session`, next to the chats database.
fn session_file() -> Option<PathBuf> {
    lucy_core::sessions::default_path()
        .and_then(|db| db.parent().map(|dir| dir.join("sprite-session")))
}

/// Her chat's ID, as last saved (for opening it in the main window).
pub fn her_saved_session() -> Option<String> {
    let text = std::fs::read_to_string(session_file()?).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Pass her chat's events to the page; when the connection drops, say so and forget it, so
/// the next message reconnects.
async fn relay(
    app: AppHandle,
    mut events: mpsc::UnboundedReceiver<Event>,
    session: String,
    id: u64,
) {
    while let Some(event) = events.recv().await {
        if event.session_id() == session {
            let _ = app.emit("core-event", &event);
        }
    }
    let _ = app.emit("core-disconnected", ());
    let chat = app.state::<Chat>();
    let mut conn = chat.conn.lock().await;
    if conn.as_ref().is_some_and(|conn| conn.id == id) {
        *conn = None;
    }
}

/// Send a message in her chat. The reply arrives as `core-event`s.
#[tauri::command]
pub async fn send(
    app: AppHandle,
    chat: tauri::State<'_, Chat>,
    text: String,
) -> Result<(), String> {
    let mut conn = chat.conn.lock().await;
    for attempt in 0..2 {
        if conn.is_none() {
            *conn = Some(open(&app).await?);
        }
        let current = conn.as_ref().expect("just opened");
        let sent = current
            .client
            .call::<_, ChatSendResult>(
                method::CHAT_SEND,
                ChatSendParams {
                    session_id: Some(current.session.clone()),
                    text: text.clone(),
                    // Her tools work in the core's own default folder.
                    cwd: None,
                },
            )
            .await;
        match sent {
            Ok(_) => return Ok(()),
            // The core went away (e.g. it was restarted): reconnect once and try again.
            Err(ClientError::Disconnected) if attempt == 0 => *conn = None,
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("lost the connection to Lucy's core".into())
}

/// Answer an approval request: `once`, `always` or `deny`.
#[tauri::command]
pub async fn approve(
    chat: tauri::State<'_, Chat>,
    call_id: String,
    decision: String,
) -> Result<(), String> {
    let decision = match decision.as_str() {
        "once" => Decision::Once,
        "always" => Decision::Always,
        _ => Decision::Deny,
    };
    let conn = chat.conn.lock().await;
    let conn = conn.as_ref().ok_or("lost the connection to Lucy's core")?;
    conn.client
        .call::<_, Value>(
            method::TOOL_APPROVE,
            ApproveParams {
                session_id: conn.session.clone(),
                call_id,
                decision,
            },
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}
