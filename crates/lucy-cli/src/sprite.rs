//! `lucy sprite`: start Lucy's desktop sprite (`apps/sprite`), or send the running one a
//! command. The sprite is its own program, `lucy-sprite`; a second launch hands its command to
//! the one already running.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::core_process;

/// `lucy sprite [command]`.
pub fn run(command: Option<&str>) -> Result<()> {
    let exe = find().context(
        "the sprite (lucy-sprite) isn't installed; see apps/sprite/README.md for how to build it",
    )?;
    match command {
        None => {
            let log = core_process::log_path()
                .and_then(|core_log| core_log.parent().map(|dir| dir.join("sprite.log")))
                .context("no data directory; set LUCY_DATA_DIR")?;
            core_process::spawn_detached(Command::new(&exe), &log)
                .with_context(|| format!("couldn't start {}", exe.display()))?;
            println!("Lucy's sprite is on her way. `lucy sprite quit` sends her off.");
        }
        Some(command) => {
            let status = Command::new(&exe)
                .arg(command)
                .status()
                .with_context(|| format!("couldn't run {}", exe.display()))?;
            if !status.success() {
                bail!("`lucy-sprite {command}` failed ({status})");
            }
        }
    }
    Ok(())
}

/// `lucy-sprite` next to this `lucy`, else on `PATH`.
fn find() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "lucy-sprite.exe"
    } else {
        "lucy-sprite"
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name));
    beside
        .into_iter()
        .chain(on_path)
        .find(|path| path.is_file())
}
