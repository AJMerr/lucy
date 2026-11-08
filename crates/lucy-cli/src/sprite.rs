//! `lucy sprite` and `lucy app`: start Lucy's desktop app (`apps/desktop`, the `lucy-desktop`
//! program: her sprite, main window and tray icon), or send the running one a command. A
//! second launch hands its command to the one already running.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::core_process;

/// The desktop program's name.
const PROGRAM: &str = "lucy-desktop";

/// Start the app (if needed) and give it `command`: `sprite` shows her, `app` opens the main
/// window, and `ask`, `chat`, `toggle` and `quit` are `lucy sprite`'s.
pub fn run(command: &str) -> Result<()> {
    let exe = find().with_context(|| {
        format!(
            "Lucy's desktop app ({PROGRAM}) isn't installed; see apps/desktop/README.md for how \
             to build it"
        )
    })?;
    match command {
        // Quitting never starts it, so wait for it to finish.
        "quit" => {
            let status = Command::new(&exe)
                .arg("quit")
                .status()
                .with_context(|| format!("couldn't run {}", exe.display()))?;
            if !status.success() {
                bail!("`{PROGRAM} quit` failed ({status})");
            }
        }
        // Anything else may start it (if it isn't running), so it mustn't wait for it.
        _ => {
            let log = log_path().context("no data directory; set LUCY_DATA_DIR")?;
            let mut app = Command::new(&exe);
            app.arg(command);
            core_process::spawn_detached(app, &log)
                .with_context(|| format!("couldn't start {}", exe.display()))?;
            if command == "sprite" {
                println!("Lucy's sprite is on her way. `lucy sprite quit` sends her off.");
            }
        }
    }
    Ok(())
}

/// Where the desktop app writes what it has to say: `desktop.log`, next to `core.log`.
pub fn log_path() -> Option<PathBuf> {
    core_process::log_path()
        .and_then(|core_log| core_log.parent().map(|dir| dir.join("desktop.log")))
}

/// `lucy-desktop` next to this `lucy`, else on `PATH`.
pub(crate) fn find() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        format!("{PROGRAM}.exe")
    } else {
        PROGRAM.to_owned()
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&name));
    beside
        .into_iter()
        .chain(on_path)
        .find(|path| path.is_file())
}
