//! Lucy's desktop sprite (ADR 0005): a small transparent window, always on top, that draws
//! the configured sprite pack. `lucy sprite` starts it; `lucy sprite quit` stops it.

// No console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pack;
mod placement;

use tauri::{AppHandle, Manager};

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

    tauri::Builder::default()
        // A second `lucy-sprite` hands its command to the running one and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            command(app, args.get(1).map(String::as_str));
        }))
        .invoke_handler(tauri::generate_handler![pack::pack])
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("couldn't start the sprite");
}

/// A command from `lucy sprite <command>`, forwarded from a second launch.
fn command(app: &AppHandle, command: Option<&str>) {
    match command {
        Some("quit") => app.exit(0),
        // `lucy sprite` again: she's already here.
        None => {}
        Some(other) => eprintln!("lucy-sprite: unknown command `{other}`"),
    }
}
