//! Her chat window: an ordinary window with her whole chat (`chat.html`), for replies too long
//! for a bubble. "more…" in the bubble and `lucy sprite chat` open it. While it's open the
//! sprite page keeps her replies out of the bubble (`chat-window` events).

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const LABEL: &str = "chat";

/// Open the chat window, or bring it forward if it's open.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("chat.html".into()))
        .title("Lucy")
        .inner_size(460.0, 640.0)
        .min_inner_size(320.0, 360.0)
        .build();
    match built {
        Ok(window) => {
            let _ = app.emit("chat-window", true);
            let handle = app.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    let _ = handle.emit("chat-window", false);
                }
            });
        }
        Err(err) => eprintln!("lucy-sprite: couldn't open her chat window: {err}"),
    }
}

/// The page's way to open it ("more…").
#[tauri::command]
pub async fn open_chat(app: AppHandle) {
    // Windows are made on the main thread.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || open(&handle));
}
