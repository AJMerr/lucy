//! Lucy's main window (`app.html`): her chats and, later, settings. `lucy app`, the tray icon
//! and "more…" on the sprite's bubble open it; closing it only hides it, so the sprite and the
//! tray stay. While it's showing, the sprite keeps replies out of her bubble (`chat-window`).

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const LABEL: &str = "app";

/// A chat to open once a newly made window is listening.
#[derive(Default)]
pub struct AppWindow {
    pending_chat: Mutex<Option<String>>,
}

/// Show the main window (making it if needed), on chat `chat` if given.
pub fn open(app: &AppHandle, chat: Option<String>) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit("chat-window", true);
        if let Some(chat) = chat {
            let _ = app.emit_to(LABEL, "app-open-chat", chat);
        }
        return;
    }
    // The page asks for this when it starts (`take_pending_chat`).
    *app.state::<AppWindow>().pending_chat.lock().unwrap() = chat;
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("app.html".into()))
        .title("Lucy")
        .inner_size(1000.0, 700.0)
        .min_inner_size(360.0, 480.0)
        // Dropped pictures go to the page (as files it can read), not to Tauri.
        .disable_drag_drop_handler()
        .build();
    match built {
        Ok(window) => {
            let _ = app.emit("chat-window", true);
            let handle = app.clone();
            let hide = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    // Keep it (and its chats) for next time; the tray and the sprite stay.
                    api.prevent_close();
                    let _ = hide.hide();
                    let _ = handle.emit("chat-window", false);
                }
            });
        }
        Err(err) => eprintln!("lucy-desktop: couldn't open Lucy's window: {err}"),
    }
}

/// The page's way to open it on her chat ("more…").
#[tauri::command]
pub async fn open_chat(app: AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || open(&handle, crate::chat::her_saved_session()));
}

/// The chat a newly made window should open, if any (once).
#[tauri::command]
pub fn take_pending_chat(state: tauri::State<'_, AppWindow>) -> Option<String> {
    state.pending_chat.lock().unwrap().take()
}
