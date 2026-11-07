//! Running the core (`lucy core run`), starting it on demand or through the login service,
//! and stopping it.

use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialoguer::Confirm;
use lucy_client::{Client, ClientError, Endpoint};
use lucy_core::secrets::{OsKeyring, Secrets};
use lucy_core::server::{CoreError, CoreOptions, serve};
use lucy_core::{config, persona, sessions};
use lucy_protocol::rpc::{BusyChat, CoreStatus, PROTOCOL_VERSION};

use crate::service::{ServiceManager, SystemRunner};

/// Default idle timeout for an auto-started core (ADR 0001).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long to wait for a core to stop, or to come up.
const WAIT: Duration = Duration::from_secs(5);
pub const CLIENT_NAME: &str = concat!("lucy-cli ", env!("CARGO_PKG_VERSION"));

/// `lucy core run`: serve until idle, or with `service` until stopped. Exits quietly if
/// another core already owns the socket.
pub fn run(service: bool) -> Result<()> {
    if service {
        #[cfg(windows)]
        detach_and_log()?;
    }
    let idle_timeout = if service {
        None
    } else {
        Some(
            std::env::var("LUCY_CORE_IDLE_SECS")
                .ok()
                .and_then(|secs| secs.parse().ok())
                .map_or(IDLE_TIMEOUT, Duration::from_secs),
        )
    };
    let options = CoreOptions {
        endpoint: Endpoint::default_for_user(),
        config_path: config::default_path().context("no config directory; set LUCY_CONFIG")?,
        db_path: sessions::default_path().context("no data directory; set LUCY_DATA_DIR")?,
        personas_dir: persona::default_user_dir(),
        secrets: Secrets::new(Box::new(OsKeyring), Box::new(|var| std::env::var(var).ok())),
        idle_timeout,
        token_store: Some(std::sync::Arc::new(OsKeyring)),
    };
    let endpoint = options.endpoint.to_string();
    log(&format!(
        "lucy core {} (pid {}) starting at {endpoint}{}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        if service { " as a service" } else { "" }
    ));
    let outcome = tokio::runtime::Runtime::new()?.block_on(serve(options));
    match outcome {
        Ok(()) => {
            log(if service {
                "exiting: stopped"
            } else {
                "exiting: idle or stopped"
            });
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

/// Connect to the core chats use: a remote one if this machine is connected to one (see
/// [`crate::remote::target`]), else this machine's own.
pub async fn connect() -> Result<Client> {
    match crate::remote::target()? {
        Some(remote) => connect_remote(&remote).await,
        None => connect_local().await,
    }
}

/// Connect to a core on another machine (`lucy remote connect`, `--remote`, `LUCY_REMOTE`).
async fn connect_remote(endpoint: &Endpoint) -> Result<Client> {
    Client::connect(endpoint, CLIENT_NAME)
        .await
        .map_err(|err| match err {
            ClientError::Connect { .. } => anyhow::Error::new(err).context(format!(
                "couldn't reach the Lucy core at {endpoint}; is it running there, with remote \
                 access on (`[remote] enabled = true`)?"
            )),
            err => err.into(),
        })
}

/// Connect to this machine's core, starting it if it isn't running. A core from an older
/// Lucy is replaced when it's idle.
pub async fn connect_local() -> Result<Client> {
    let endpoint = Endpoint::default_for_user();
    let connected = match Client::connect_or_spawn(&endpoint, CLIENT_NAME, spawn).await {
        Err(err @ (ClientError::VersionMismatch { .. } | ClientError::Protocol(_))) => {
            replace_old_core(&endpoint, err).await?;
            Client::connect_or_spawn(&endpoint, CLIENT_NAME, spawn).await
        }
        other => other,
    };
    connected.with_context(|| match log_path() {
        Some(log) => format!(
            "could not reach the Lucy core; see its log at {}",
            log.display()
        ),
        None => "could not reach the Lucy core".into(),
    })
}

/// Stop a core that speaks an older protocol, if nothing is in progress, so a new one starts.
async fn replace_old_core(endpoint: &Endpoint, mismatch: ClientError) -> Result<()> {
    let Ok(status) = Client::status(endpoint).await else {
        // Cores before protocol v4 can't be asked to stop.
        bail!(
            "the running Lucy core is from an older version; stop it with \
             `pkill -f \"lucy core run\"` (on Windows, end the lucy.exe running `core run`) \
             and try again"
        );
    };
    if status.protocol_version > PROTOCOL_VERSION {
        bail!(
            "the running Lucy core (v{}) is newer than this lucy (v{}); update lucy, or stop \
             the core with `lucy core stop`",
            status.core_version,
            env!("CARGO_PKG_VERSION")
        );
    }
    if !status.busy.is_empty() {
        bail!(
            "the running Lucy core is from an older version and is replying in {}; let it \
             finish, then try again (or stop it now with `lucy core stop --force`)",
            busy_list(&status.busy)
        );
    }
    let stopped = Client::shutdown(endpoint, false).await?;
    if !stopped.stopping || !Client::wait_until_gone(endpoint, WAIT).await {
        return Err(mismatch.into());
    }
    eprintln!("Restarted Lucy's core (the running one was from an older build).");
    Ok(())
}

/// The running core's status, or `None` if nothing answers.
pub async fn status(endpoint: &Endpoint) -> Result<Option<CoreStatus>> {
    match Client::status(endpoint).await {
        Ok(status) => Ok(Some(status)),
        Err(ClientError::Connect { .. }) => Ok(None),
        // Cores before protocol v4 want `hello` first.
        Err(_) => bail!(
            "the running Lucy core is from an older version; stop it with \
             `pkill -f \"lucy core run\"` (on Windows, end the lucy.exe running `core run`)"
        ),
    }
}

/// Stop the core at `endpoint` and wait until it's gone. Replies in progress are cancelled
/// with `force`, or after asking on a terminal. False if no core was running.
pub async fn stop(endpoint: &Endpoint, force: bool) -> Result<bool> {
    if status(endpoint).await?.is_none() {
        return Ok(false);
    }
    let mut result = Client::shutdown(endpoint, force).await?;
    if !result.stopping {
        let busy = busy_list(&result.busy);
        let confirmed = std::io::stdin().is_terminal()
            && Confirm::new()
                .with_prompt(format!(
                    "Lucy is replying in {busy}. Stop her anyway? (she keeps what she's said)"
                ))
                .default(false)
                .interact()?;
        if !confirmed {
            bail!("Lucy is replying in {busy}; wait for her, or pass --force to stop anyway");
        }
        result = Client::shutdown(endpoint, true).await?;
    }
    if !result.stopping || !Client::wait_until_gone(endpoint, WAIT).await {
        bail!(
            "the Lucy core didn't stop{}",
            log_path().map_or(String::new(), |log| format!("; see {}", log.display()))
        );
    }
    Ok(true)
}

/// Busy chats by title, for messages.
fn busy_list(busy: &[BusyChat]) -> String {
    busy.iter()
        .map(|chat| match &chat.title {
            Some(title) => format!("\"{title}\""),
            None => format!("chat {}", &chat.session_id[..chat.session_id.len().min(8)]),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Start the core: through the login service when one is installed for this socket, else on
/// demand.
pub fn start(endpoint: &Endpoint) -> std::io::Result<()> {
    if let Some(manager) = service_for(endpoint) {
        match manager.start() {
            Ok(true) => return Ok(()),
            Ok(false) => return spawn_core(true),
            Err(err) => eprintln!(
                "Couldn't start Lucy's service ({err:#}); starting the core on demand instead."
            ),
        }
    }
    spawn_core(false)
}

/// Wait until the core at `endpoint` answers, and return its status.
pub async fn wait_until_up(endpoint: &Endpoint) -> Result<CoreStatus> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Ok(status) = Client::status(endpoint).await {
            return Ok(status);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the Lucy core didn't start{}",
                log_path().map_or(String::new(), |log| format!("; see {}", log.display()))
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The login service that runs the core at `endpoint`, if one is installed. A service pins
/// the socket it was installed for, so cores at other sockets (tests, `LUCY_SOCKET`) start
/// on demand; on Windows it uses the default pipe.
pub fn service_for(endpoint: &Endpoint) -> Option<ServiceManager<SystemRunner>> {
    let manager = ServiceManager::for_this_os()?;
    let installed = manager.installed().ok()??;
    let ours = match (&installed.socket, endpoint) {
        (Some(socket), Endpoint::Path(path)) => socket == path,
        (None, Endpoint::Pipe(_)) => std::env::var_os("LUCY_SOCKET").is_none(),
        _ => false,
    };
    ours.then_some(manager)
}

pub fn log_path() -> Option<PathBuf> {
    sessions::default_path().and_then(|db| db.parent().map(|dir| dir.join("core.log")))
}

fn spawn() -> std::io::Result<()> {
    start(&Endpoint::default_for_user())
}

/// Start `lucy core run` detached from this terminal, logging to `<data dir>/core.log`.
pub(crate) fn spawn_core(service: bool) -> std::io::Result<()> {
    let log =
        log_path().ok_or_else(|| std::io::Error::other("no data directory; set LUCY_DATA_DIR"))?;
    let mut command = Command::new(lucy_exe()?);
    command.args(["core", "run"]);
    if service {
        command.arg("--service");
    }
    spawn_detached(command, &log)
}

/// Start `command` detached from this terminal, appending its output to `log`.
pub(crate) fn spawn_detached(mut command: Command, log: &Path) -> std::io::Result<()> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = OpenOptions::new().create(true).append(true).open(log)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so Ctrl+C in this terminal doesn't take it down too.
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

/// Windows starts the `Run` value as a console program, so a window opens at login. Let go of
/// it, and log to `core.log` instead of the console.
#[cfg(windows)]
fn detach_and_log() -> Result<()> {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{
        FreeConsole, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    let log = log_path().context("no data directory; set LUCY_DATA_DIR")?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&log)?;
    let handle = file.into_raw_handle();
    // SAFETY: the handle stays open for the life of the process (into_raw_handle leaks it).
    unsafe {
        FreeConsole();
        SetStdHandle(STD_OUTPUT_HANDLE, handle as _);
        SetStdHandle(STD_ERROR_HANDLE, handle as _);
    }
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

/// The `lucy` program: this one, or (when another front end such as the sprite starts the
/// core) the `lucy` next to it, else the one on `PATH`.
fn lucy_exe() -> std::io::Result<PathBuf> {
    let me = std::env::current_exe()?;
    let name = if cfg!(windows) { "lucy.exe" } else { "lucy" };
    if me.file_name().is_some_and(|file| file == name) {
        return Ok(me);
    }
    let beside = me.parent().map(|dir| dir.join(name));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name));
    beside
        .into_iter()
        .chain(on_path)
        .find(|path| path.is_file())
        .ok_or_else(|| std::io::Error::other("can't find the `lucy` program to start the core"))
}
