//! Lucy's desktop sprite (ADR 0005): a small transparent window, always on top, that draws
//! the configured sprite pack and talks with Lucy's core. `lucy sprite` starts it, and
//! `lucy sprite toggle|ask|chat|quit` drive it.

// No console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chat;
mod chat_window;
mod pack;
mod placement;

use tauri::{AppHandle, Emitter, Manager};

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // WebKitGTK's DMA-BUF renderer draws nothing (or crashes) on some GPUs under Wayland,
        // NVIDIA's included; a sprite this small doesn't need it.
        // SAFETY: nothing else is running yet to read the environment concurrently.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }

    let first_command = std::env::args().nth(1);
    let loaded = pack::load_configured();
    let hotkey = loaded.hotkey.clone();

    tauri::Builder::default()
        // A second `lucy-sprite` hands its command to the running one and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            command(app, args.get(1).map(String::as_str));
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        command(app, Some("ask"));
                    }
                })
                .build(),
        )
        .manage(chat::Chat::default())
        .invoke_handler(tauri::generate_handler![
            pack::pack,
            chat::send,
            chat::approve,
            chat::history,
            chat::cancel,
            chat_window::open_chat,
            placement::set_layout,
        ])
        .setup(move |app| {
            if first_command.as_deref() == Some("quit") {
                // Nothing was running to quit.
                app.handle().exit(0);
                return Ok(());
            }
            let size = loaded.window_size();
            app.manage(loaded);
            let window = app
                .get_webview_window("sprite")
                .expect("tauri.conf.json defines the sprite window");
            placement::place(&window, size)?;
            register_hotkey(app.handle(), &hotkey);
            chat::connect_in_background(app.handle().clone());
            if let Some(first @ ("ask" | "chat")) = first_command.as_deref() {
                // `lucy sprite ask|chat` started her: act once the page is listening.
                let first = first.to_owned();
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let main = handle.clone();
                    let _ = handle.run_on_main_thread(move || command(&main, Some(&first)));
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("couldn't start the sprite");
}

/// A command from `lucy sprite <command>` (forwarded from a second launch) or the hotkey.
fn command(app: &AppHandle, command: Option<&str>) {
    match command {
        Some("quit") => app.exit(0),
        Some("toggle") => placement::toggle(app),
        Some("chat") => chat_window::open(app),
        Some("ask") => {
            placement::show(app);
            let _ = app.emit("sprite-command", "ask");
        }
        // `lucy sprite` again: make sure she's visible.
        None => placement::show(app),
        Some(other) => eprintln!("lucy-sprite: unknown command `{other}`"),
    }
}

/// Register `[sprite] hotkey` where apps can have global shortcuts. Wayland doesn't allow
/// it; bind `lucy sprite ask` in the compositor there.
fn register_hotkey(app: &AppHandle, hotkey: &str) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let wayland = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some();
    if hotkey.is_empty() || wayland {
        return;
    }
    if let Err(err) = app.global_shortcut().register(hotkey) {
        eprintln!("lucy-sprite: couldn't use `{hotkey}` as her hotkey: {err}");
    }
}
