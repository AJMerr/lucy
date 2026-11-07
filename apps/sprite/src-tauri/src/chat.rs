//! Her conversation: one ongoing chat of her own (titled "Sprite"), with whichever core `lucy`
//! uses (this machine's, started if needed, or a remote one after `lucy remote connect`). Its
//! events go to both pages (her and the chat window) as `core-event`; they send messages and
//! approvals back.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lucy_client::{Client, ClientError};
use lucy_protocol::rpc::{
    ApproveParams, ChatSendParams, ChatSendResult, Decision, Event, RenameParams, ResolveParams,
    Session, SessionParams, StoredMessage, method,
};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, MutexGuard, mpsc};

/// The title of her chat in `/sessions`.
const TITLE: &str = "Sprite";

/// The connection to the core, opened on first use and again after it drops.
#[derive(Default)]
pub struct Chat {
    conn: Mutex<Option<Conn>>,
    /// Her reply in progress, for a chat window opened partway through it.
    live: std::sync::Mutex<Option<Live>>,
}

/// A reply in progress (`Live` in src/transcript.ts).
#[derive(Debug, Clone, Serialize)]
pub struct Live {
    turn_id: u64,
    /// What she's said since her last tool call.
    text: String,
    /// The approval request she's waiting on.
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<Event>,
}

impl Live {
    /// Follow the reply through one of her chat's events.
    fn follow(live: &mut Option<Live>, event: &Event) {
        match event {
            Event::TurnStarted { turn_id, .. } => {
                *live = Some(Live {
                    turn_id: *turn_id,
                    text: String::new(),
                    approval: None,
                });
            }
            Event::TurnFinished { .. } => *live = None,
            _ => {
                let Some(current) = live.as_mut() else {
                    return;
                };
                match event {
                    Event::Delta { text, .. } => current.text += text,
                    Event::ToolApproval { .. } => current.approval = Some(event.clone()),
                    Event::ToolStarted { .. } => {
                        current.text.clear();
                        current.approval = None;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// What the chat window opens on.
#[derive(Serialize)]
pub struct History {
    messages: Vec<StoredMessage>,
    live: Option<Live>,
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
                Err(err) => eprintln!("lucy-sprite: {err}"),
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
                "lucy-sprite: couldn't remember her chat in {}: {err}",
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
            Live::follow(&mut app.state::<Chat>().live.lock().unwrap(), &event);
            let _ = app.emit("core-event", &event);
        }
    }
    *app.state::<Chat>().live.lock().unwrap() = None;
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

/// Her connection, opened if it isn't.
async fn connected<'a>(
    app: &AppHandle,
    chat: &'a Chat,
) -> Result<MutexGuard<'a, Option<Conn>>, String> {
    let mut conn = chat.conn.lock().await;
    if conn.is_none() {
        *conn = Some(open(app).await?);
    }
    Ok(conn)
}

/// Her chat so far, and the reply in progress, for the chat window.
#[tauri::command]
pub async fn history(app: AppHandle, chat: tauri::State<'_, Chat>) -> Result<History, String> {
    let conn = connected(&app, &chat).await?;
    let conn = conn.as_ref().expect("just connected");
    let session: Session = conn
        .client
        .call(
            method::SESSIONS_LOAD,
            SessionParams {
                session_id: conn.session.clone(),
            },
        )
        .await
        .map_err(|err| err.to_string())?;
    let live = chat.live.lock().unwrap().clone();
    Ok(History {
        messages: session.messages,
        live,
    })
}

/// Stop her reply in progress.
#[tauri::command]
pub async fn cancel(chat: tauri::State<'_, Chat>) -> Result<(), String> {
    let conn = chat.conn.lock().await;
    let conn = conn.as_ref().ok_or("lost the connection to Lucy's core")?;
    conn.client
        .call::<_, Value>(
            method::CHAT_CANCEL,
            SessionParams {
                session_id: conn.session.clone(),
            },
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}
