//! Running the core (`lucy core run`) and starting it in the background on demand.

use std::fs::OpenOptions;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{OsKeyring, Secrets};
use lucy_core::server::{CoreError, CoreOptions, serve};
use lucy_core::{config, persona, sessions};

/// Default idle timeout for an auto-started core (ADR 0001).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// `lucy core run`: serve until idle. Exits quietly if another core already owns the socket.
pub fn run() -> Result<()> {
    let idle_timeout = std::env::var("LUCY_CORE_IDLE_SECS")
        .ok()
        .and_then(|secs| secs.parse().ok())
        .map_or(IDLE_TIMEOUT, Duration::from_secs);
    let options = CoreOptions {
        endpoint: Endpoint::default_for_user(),
        config_path: config::default_path().context("no config directory; set LUCY_CONFIG")?,
        db_path: sessions::default_path().context("no data directory; set LUCY_DATA_DIR")?,
        personas_dir: persona::default_user_dir(),
        secrets: Secrets::new(Box::new(OsKeyring), Box::new(|var| std::env::var(var).ok())),
        idle_timeout,
    };
    let endpoint = options.endpoint.to_string();
    log(&format!(
        "lucy core {} (pid {}) starting at {endpoint}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
    let outcome = tokio::runtime::Runtime::new()?.block_on(serve(options));
    match outcome {
        Ok(()) => {
            log("exiting: idle");
            Ok(())
        }
        Err(CoreError::AlreadyRunning(_)) => {
            log("exiting: another core is already running");
            Ok(())
        }
        Err(err) => {
            log(&format!("exiting: {err}"));
            Err(err.into())
        }
    }
}

/// One line in the core's log (its stderr, which `spawn` points at `core.log`).
fn log(message: &str) {
    eprintln!(
        "[{}] {message}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
}

/// Connect to the core, starting it in the background if it isn't running.
pub async fn connect() -> Result<Client> {
    Client::connect_or_spawn(
        &Endpoint::default_for_user(),
        concat!("lucy-cli ", env!("CARGO_PKG_VERSION")),
        spawn,
    )
    .await
    .with_context(|| match log_path() {
        Some(log) => format!(
            "could not reach the Lucy core; see its log at {}",
            log.display()
        ),
        None => "could not reach the Lucy core".into(),
    })
}

fn log_path() -> Option<std::path::PathBuf> {
    sessions::default_path().and_then(|db| db.parent().map(|dir| dir.join("core.log")))
}

/// Start `lucy core run` detached from this terminal, logging to `<data dir>/core.log`.
fn spawn() -> std::io::Result<()> {
    let log =
        log_path().ok_or_else(|| std::io::Error::other("no data directory; set LUCY_DATA_DIR"))?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = OpenOptions::new().create(true).append(true).open(log)?;

    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["core", "run"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so Ctrl+C in this terminal doesn't take the core down too.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        stop_inheriting_std_handles();
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn()?;
    Ok(())
}

/// Windows hands every inheritable handle to child processes. Our stdin/stdout/stderr are
/// inheritable (they came from our parent), so without this the long-lived core would hold
/// them open: `lucy ask | other` would then wait for the core to exit instead of for us.
#[cfg(windows)]
fn stop_inheriting_std_handles() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};

    let handles = [
        std::io::stdin().as_raw_handle(),
        std::io::stdout().as_raw_handle(),
        std::io::stderr().as_raw_handle(),
    ];
    for handle in handles {
        if !handle.is_null() {
            // SAFETY: a handle we own for our whole lifetime; only its inherit flag changes.
            unsafe { SetHandleInformation(handle as _, HANDLE_FLAG_INHERIT, 0) };
        }
    }
}
