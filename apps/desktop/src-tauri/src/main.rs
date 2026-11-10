//! Lucy's desktop app (`lucy-desktop`): her sprite (ADR 0005), a small transparent window,
//! always on top, that draws the configured sprite pack and talks with Lucy's core; her main
//! window (chats and settings); and a tray icon. `lucy app` and `lucy sprite` start it, and
//! `lucy sprite toggle|ask|chat|quit` drive it.
//!
//! Started with no command (from your app menu), it opens the main window, and the sprite if
//! `[sprite] enabled`; with `background` (at login), just the sprite if she's enabled.

// No console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_window;
mod bridge;
mod chat;
mod lucy_command;
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
    // Commands that are about her show her even when she's off at start.
    let show_sprite =
        loaded.enabled || matches!(first_command.as_deref(), Some("sprite" | "ask" | "toggle"));

    tauri::Builder::default()
        // A second `lucy-desktop` hands its command to the running one and exits.
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
        .manage(bridge::Bridge::default())
        .manage(app_window::AppWindow::default())
        .invoke_handler(tauri::generate_handler![
            pack::pack,
            chat::send,
            chat::approve,
            chat::picture,
            app_window::open_chat,
            app_window::take_pending_chat,
            bridge::rpc_call,
            lucy_command::run_lucy,
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
            if !show_sprite {
                placement::hide(app.handle());
            }
            register_hotkey(app.handle(), &hotkey);
            chat::connect_in_background(app.handle().clone());
            if tray_supported() {
                if let Err(err) = tray(app.handle()) {
                    eprintln!("lucy-desktop: no tray icon: {err}");
                }
            } else {
                eprintln!(
                    "lucy-desktop: no tray icon (it needs libayatana-appindicator); `lucy app` \
                     opens Lucy's window"
                );
            }
            let first = match first_command.as_deref() {
                // From your app menu: open the window.
                None => Some("app"),
                // At login, or `lucy sprite`: she's already up (if she should be).
                Some("background" | "sprite" | "toggle") => None,
                Some(other) => Some(other),
            };
            if let Some(first) = first {
                // Act once the pages are listening.
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
        Some("chat") => app_window::open(app, chat::her_saved_session()),
        Some("app") => app_window::open(app, None),
        Some("ask") => {
            placement::show(app);
            let _ = app.emit("sprite-command", "ask");
        }
        // `lucy sprite`: make sure she's visible.
        Some("sprite") => placement::show(app),
        // Opened again from your app menu: show the window.
        None => app_window::open(app, None),
        Some("background") => {}
        Some(other) => eprintln!("lucy-desktop: unknown command `{other}`"),
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
        eprintln!("lucy-desktop: couldn't use `{hotkey}` as her hotkey: {err}");
    }
}

/// The tray icon: open Lucy's window, show or hide the sprite, or quit. Clicking it opens the
/// window too. (On GNOME it needs the AppIndicator extension; `lucy app` always works.)
fn tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open Lucy", true, None::<&str>)?,
            &MenuItem::with_id(app, "sprite", "Show or hide the sprite", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit Lucy", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("lucy")
        .tooltip("Lucy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => app_window::open(app, None),
            "sprite" => placement::toggle(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                app_window::open(tray.app_handle(), None);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Whether a tray icon can work here. On Linux, Tauri's tray loads the AppIndicator library
/// at runtime, and stops the whole app if it can't, so look first.
fn tray_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        [
            "libayatana-appindicator3.so.1",
            "libayatana-appindicator3.so",
            "libappindicator3.so.1",
            "libappindicator3.so",
        ]
        .iter()
        .any(|name| {
            let name = std::ffi::CString::new(*name).expect("no NUL in a library name");
            // SAFETY: a valid C string; the handle is closed straight away.
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };
            if handle.is_null() {
                return false;
            }
            // SAFETY: `handle` came from dlopen above.
            unsafe { libc::dlclose(handle) };
            true
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
