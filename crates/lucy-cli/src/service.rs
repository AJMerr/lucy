//! `lucy service`: run the core at login under the OS's service manager: a systemd user unit
//! on Linux, a launchd agent on macOS, and a `Run` registry value on Windows. With
//! `[app] autostart`, Lucy's desktop app starts at login too (in the background: just the
//! sprite, if she's on, and the tray): an XDG autostart entry on Linux, a launchd agent on
//! macOS, and another `Run` value on Windows.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use lucy_client::Endpoint;

use crate::core_process;

/// The systemd unit's file name.
pub const UNIT_NAME: &str = "lucy-core.service";
/// The launchd agent's label (and plist name).
pub const LAUNCHD_LABEL: &str = "io.github.ajmerr.lucy.core";
/// Where Windows keeps programs to start at login, and the value Lucy adds there.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Lucy";
/// The desktop app's XDG autostart entry, launchd label and `Run` value.
pub const APP_DESKTOP_FILE: &str = "lucy-desktop.desktop";
pub const APP_LAUNCHD_LABEL: &str = "io.github.ajmerr.lucy.desktop";
pub const APP_RUN_VALUE: &str = "LucyDesktop";
/// What v0.4.0 used for the sprite before she became part of the app; cleared away.
const OLD_SPRITE_DESKTOP_FILE: &str = "lucy-sprite.desktop";
const OLD_SPRITE_LAUNCHD_LABEL: &str = "io.github.ajmerr.lucy.sprite";
const OLD_SPRITE_RUN_VALUE: &str = "LucySprite";
/// The argument login entries start the app with: no window, just the sprite and tray.
pub const BACKGROUND: &str = "background";

/// What the service runs, and with which environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    pub exe: PathBuf,
    /// Where the core's output goes (systemd and launchd; on Windows the core opens it).
    pub log: PathBuf,
    /// Variables the service runs with, from [`capture_env`] (unused on Windows, where the
    /// core inherits your user environment).
    pub env: Vec<(String, String)>,
}

impl ServiceSpec {
    /// The running `lucy`, logging to `log`, with this shell's environment for a core that
    /// listens at `socket`.
    pub fn current(log: PathBuf, socket: Option<&Path>) -> Result<Self> {
        Ok(Self {
            exe: std::env::current_exe().context("couldn't find the running lucy program")?,
            log,
            env: socket.map_or_else(Vec::new, |socket| capture_env(std::env::vars(), socket)),
        })
    }
}

/// The variables a service core needs from the installing shell: `PATH` (for commands and MCP
/// servers), where Lucy keeps things (`LUCY_*`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`), and
/// `LUCY_SOCKET` pinned to `socket` so the core listens where this `lucy` looks. API keys
/// (`LUCY_KEY_*`) are never copied into a service file.
pub fn capture_env(
    vars: impl IntoIterator<Item = (String, String)>,
    socket: &Path,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vars
        .into_iter()
        .filter(|(name, value)| {
            let wanted = matches!(name.as_str(), "PATH" | "XDG_CONFIG_HOME" | "XDG_DATA_HOME")
                || (name.starts_with("LUCY_")
                    && !name.starts_with("LUCY_KEY_")
                    && name != "LUCY_SOCKET"
                    && name != "LUCY_CORE_IDLE_SECS");
            wanted && !value.contains(['\n', '\r', '\0'])
        })
        .collect();
    env.push(("LUCY_SOCKET".into(), socket.display().to_string()));
    env.sort();
    env
}

/// A systemd user unit that runs the core from login until stopped, restarting it after a
/// crash (a clean `lucy core stop` leaves it stopped).
pub fn systemd_unit(spec: &ServiceSpec) -> String {
    let mut unit = String::from(
        "# Lucy's background core, written by `lucy service install`.\n\
         # Remove it with `lucy service uninstall`. Re-run install after changing PATH or LUCY_*.\n\
         [Unit]\n\
         Description=Lucy core (personal AI assistant)\n\
         \n\
         [Service]\n",
    );
    let exe = systemd_quoted(&spec.exe.display().to_string()).replace('$', "$$");
    unit += &format!("ExecStart={exe} core run --service\n");
    for (name, value) in &spec.env {
        unit += &format!(
            "Environment={}\n",
            systemd_quoted(&format!("{name}={value}"))
        );
    }
    let log = spec.log.display().to_string().replace('%', "%%");
    unit += &format!(
        "StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    );
    unit
}

/// A double-quoted systemd word: C-style escapes, and `%` doubled so it isn't a specifier.
fn systemd_quoted(text: &str) -> String {
    let escaped = text
        .replace('\\', r"\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

/// Undo [`systemd_quoted`] (for values written without `$` doubling), resolving `%t` (the
/// runtime dir) as systemd does.
fn systemd_unquoted(word: &str) -> Option<String> {
    let inner = word.strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => text.push(chars.next()?),
            '%' => match chars.next()? {
                't' => text += &std::env::var("XDG_RUNTIME_DIR").ok()?,
                c => text.push(c),
            },
            c => text.push(c),
        }
    }
    Some(text)
}

/// A launchd agent that runs the core at login and restarts it if it crashes.
pub fn launchd_plist(spec: &ServiceSpec) -> String {
    agent_plist(
        "Lucy's background core, written by `lucy service install`. \
         Remove it with `lucy service uninstall`.",
        LAUNCHD_LABEL,
        &["core", "run", "--service"],
        spec,
        "  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n\
         \x20 <key>ProcessType</key>\n  <string>Background</string>\n",
    )
}

/// A launchd agent that starts the app at login, in the background (and leaves it gone if you
/// quit it).
pub fn app_launchd_plist(spec: &ServiceSpec) -> String {
    agent_plist(
        "Lucy's desktop app, written by `lucy service install` because [app] autostart is on. \
         Remove it with `lucy service uninstall`.",
        APP_LAUNCHD_LABEL,
        &[BACKGROUND],
        spec,
        "  <key>LimitLoadToSessionType</key>\n  <string>Aqua</string>\n\
         \x20 <key>ProcessType</key>\n  <string>Interactive</string>\n",
    )
}

/// A launchd agent running `spec.exe` with `args` at load, logging to `spec.log`; `extra` is
/// more of the top-level dict.
fn agent_plist(
    comment: &str,
    label: &str,
    args: &[&str],
    spec: &ServiceSpec,
    extra: &str,
) -> String {
    let string = |text: &str| format!("<string>{}</string>", xml_escaped(text));
    let mut plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <!-- {comment} -->\n\
         <plist version=\"1.0\">\n<dict>\n"
    );
    plist += &format!("  <key>Label</key>\n  {}\n", string(label));
    plist += "  <key>ProgramArguments</key>\n  <array>\n";
    let exe = spec.exe.display().to_string();
    for arg in std::iter::once(exe.as_str()).chain(args.iter().copied()) {
        plist += &format!("    {}\n", string(arg));
    }
    plist += "  </array>\n  <key>EnvironmentVariables</key>\n  <dict>\n";
    for (name, value) in &spec.env {
        plist += &format!(
            "    <key>{}</key>\n    {}\n",
            xml_escaped(name),
            string(value)
        );
    }
    let log = string(&spec.log.display().to_string());
    plist += &format!(
        "  </dict>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         {extra}\
         \x20 <key>StandardOutPath</key>\n  {log}\n\
         \x20 <key>StandardErrorPath</key>\n  {log}\n\
         </dict>\n</plist>\n"
    );
    plist
}

fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescaped(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The command line Windows runs at login.
pub fn run_key_command(exe: &Path) -> String {
    format!("\"{}\" core run --service", exe.display())
}

/// An XDG autostart entry that starts the app, in the background, when your desktop session
/// does (GNOME, KDE, and systemd-based sessions such as uwsm's run these).
pub fn desktop_entry(spec: &ServiceSpec) -> String {
    let mut exec = String::from("env");
    for (name, value) in &spec.env {
        exec += &format!(" {}", exec_quoted(&format!("{name}={value}")));
    }
    exec += &format!(
        " {} {BACKGROUND}",
        exec_quoted(&spec.exe.display().to_string())
    );
    // The Exec value is itself a string value: backslashes doubled, and `%` is a field code.
    let exec = exec.replace('\\', r"\\").replace('%', "%%");
    format!(
        "# Lucy's desktop app, written by `lucy service install` because [app] autostart is on.\n\
         # Remove it with `lucy service uninstall`.\n\
         [Desktop Entry]\n\
         Type=Application\n\
         Name=Lucy\n\
         Comment=Lucy, your AI assistant, on the desktop\n\
         Exec={exec}\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// A double-quoted Exec argument: `"`, `` ` ``, `$` and `\` escaped with a backslash.
fn exec_quoted(text: &str) -> String {
    let mut quoted = String::from("\"");
    for c in text.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// The `LUCY_SOCKET` a service file pins, if any.
fn pinned_socket(platform: &Platform, file: &str) -> Option<PathBuf> {
    let value = match platform {
        Platform::Systemd { .. } => file.lines().find_map(|line| {
            let word = line.strip_prefix("Environment=")?;
            systemd_unquoted(word)?
                .strip_prefix("LUCY_SOCKET=")
                .map(str::to_owned)
        })?,
        Platform::Launchd { .. } => {
            let after = file.split_once("<key>LUCY_SOCKET</key>")?.1;
            let start = after.split_once("<string>")?.1;
            xml_unescaped(start.split_once("</string>")?.0)
        }
        Platform::RunKey => return None,
    };
    Some(PathBuf::from(value))
}

/// What a command printed, and whether it succeeded.
pub struct Ran {
    pub ok: bool,
    pub output: String,
}

/// Runs the service manager's commands (a fake one in tests).
pub trait Runner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Ran>;
}

/// Runs real commands.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Ran> {
        let output = Command::new(program).args(args).output()?;
        let mut text = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text += stdout.trim();
        }
        Ok(Ran {
            ok: output.status.success(),
            output: text,
        })
    }
}

/// The service manager in use, and where its files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Platform {
    /// systemd for the core; XDG autostart (`autostart_dir`) for the sprite.
    Systemd {
        unit_dir: PathBuf,
        autostart_dir: PathBuf,
    },
    Launchd {
        agents_dir: PathBuf,
        uid: u32,
    },
    RunKey,
}

/// An installed service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The unit or plist file, or the registry value.
    pub location: String,
    /// The socket the service's core listens at, when the service file pins one.
    pub socket: Option<PathBuf>,
}

pub struct ServiceManager<R> {
    pub platform: Platform,
    runner: R,
}

impl ServiceManager<SystemRunner> {
    /// This OS's service manager; `None` where Lucy doesn't know one.
    pub fn for_this_os() -> Option<Self> {
        let platform = if cfg!(windows) {
            Platform::RunKey
        } else if cfg!(target_os = "macos") {
            Platform::Launchd {
                agents_dir: dirs::home_dir()?.join("Library/LaunchAgents"),
                uid: current_uid(),
            }
        } else {
            let config = dirs::config_dir()?;
            Platform::Systemd {
                unit_dir: config.join("systemd/user"),
                autostart_dir: config.join("autostart"),
            }
        };
        Some(Self::new(platform, SystemRunner))
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

impl<R: Runner> ServiceManager<R> {
    pub fn new(platform: Platform, runner: R) -> Self {
        Self { platform, runner }
    }

    /// Register the service to start at login (replacing an earlier install). Doesn't start
    /// it; see [`ServiceManager::start`].
    pub fn install(&self, spec: &ServiceSpec) -> Result<()> {
        match &self.platform {
            Platform::Systemd { unit_dir, .. } => {
                write(&unit_dir.join(UNIT_NAME), &systemd_unit(spec))?;
                self.systemctl(&["daemon-reload"])?;
                self.systemctl(&["enable", UNIT_NAME])?;
            }
            Platform::Launchd { agents_dir, uid } => {
                let plist = self.plist_path(agents_dir);
                write(&plist, &launchd_plist(spec))?;
                // Unload an earlier version first; failing just means it wasn't loaded.
                self.runner
                    .run(
                        "launchctl",
                        &["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")],
                    )
                    .context("couldn't run `launchctl`")?;
                let plist = plist.display().to_string();
                self.run_ok("launchctl", &["bootstrap", &format!("gui/{uid}"), &plist])?;
            }
            Platform::RunKey => {
                let command = run_key_command(&spec.exe);
                self.run_ok(
                    "reg",
                    &[
                        "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// Stop starting the core at login and remove the service. False if it wasn't installed.
    /// A running service core is stopped abruptly, so stop it gracefully first.
    pub fn uninstall(&self) -> Result<bool> {
        match &self.platform {
            Platform::Systemd { unit_dir, .. } => {
                let unit = unit_dir.join(UNIT_NAME);
                if !unit.exists() {
                    return Ok(false);
                }
                self.systemctl(&["disable", "--now", UNIT_NAME])?;
                remove(&unit)?;
                self.systemctl(&["daemon-reload"])?;
            }
            Platform::Launchd { agents_dir, uid } => {
                let plist = self.plist_path(agents_dir);
                if !plist.exists() {
                    return Ok(false);
                }
                self.runner
                    .run(
                        "launchctl",
                        &["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")],
                    )
                    .context("couldn't run `launchctl`")?;
                remove(&plist)?;
            }
            Platform::RunKey => {
                if self.installed()?.is_none() {
                    return Ok(false);
                }
                self.run_ok("reg", &["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])?;
            }
        }
        Ok(true)
    }

    /// The installed service, if any.
    pub fn installed(&self) -> Result<Option<Installed>> {
        let file = match &self.platform {
            Platform::Systemd { unit_dir, .. } => unit_dir.join(UNIT_NAME),
            Platform::Launchd { agents_dir, .. } => self.plist_path(agents_dir),
            Platform::RunKey => {
                let ran = self
                    .runner
                    .run("reg", &["query", RUN_KEY, "/v", RUN_VALUE])
                    .context("couldn't run `reg`")?;
                return Ok(ran.ok.then(|| Installed {
                    location: format!(r"{RUN_KEY}\{RUN_VALUE}"),
                    socket: None,
                }));
            }
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("couldn't read {}", file.display()));
            }
        };
        Ok(Some(Installed {
            location: file.display().to_string(),
            socket: pinned_socket(&self.platform, &text),
        }))
    }

    /// Start the installed service now. False when there's no manager to ask (Windows), so
    /// the caller starts `lucy core run --service` itself.
    pub fn start(&self) -> Result<bool> {
        self.wait_until_stopped()?;
        match &self.platform {
            Platform::Systemd { .. } => self.systemctl(&["start", UNIT_NAME])?,
            Platform::Launchd { uid, .. } => self.run_ok(
                "launchctl",
                &["kickstart", &format!("gui/{uid}/{LAUNCHD_LABEL}")],
            )?,
            Platform::RunKey => return Ok(false),
        }
        Ok(true)
    }

    /// Wait (a few seconds at most) for a service core that's on its way out to finish. A
    /// stopped core closes its socket before it exits, and starting a service its manager
    /// still sees as running does nothing.
    fn wait_until_stopped(&self) -> Result<()> {
        const POLL: std::time::Duration = std::time::Duration::from_millis(100);
        for _ in 0..50 {
            let running = match &self.platform {
                Platform::Systemd { .. } => {
                    self.runner
                        .run("systemctl", &["--user", "is-active", "--quiet", UNIT_NAME])
                        .context("couldn't run `systemctl`")?
                        .ok
                }
                Platform::Launchd { uid, .. } => {
                    let ran = self
                        .runner
                        .run(
                            "launchctl",
                            &["print", &format!("gui/{uid}/{LAUNCHD_LABEL}")],
                        )
                        .context("couldn't run `launchctl`")?;
                    ran.ok && ran.output.contains("state = running")
                }
                Platform::RunKey => false,
            };
            if !running {
                return Ok(());
            }
            std::thread::sleep(POLL);
        }
        bail!("Lucy's core service is still shutting down; try again in a moment")
    }

    /// Start the app at login (replacing an earlier entry, and v0.4.0's sprite entry). Doesn't
    /// start it now.
    pub fn install_app(&self, spec: &ServiceSpec) -> Result<()> {
        match &self.platform {
            Platform::Systemd { autostart_dir, .. } => {
                write(&autostart_dir.join(APP_DESKTOP_FILE), &desktop_entry(spec))?;
            }
            Platform::Launchd { agents_dir, .. } => {
                write(
                    &agents_dir.join(format!("{APP_LAUNCHD_LABEL}.plist")),
                    &app_launchd_plist(spec),
                )?;
            }
            Platform::RunKey => {
                let command = format!("\"{}\" {BACKGROUND}", spec.exe.display());
                self.run_ok(
                    "reg",
                    &[
                        "add",
                        RUN_KEY,
                        "/v",
                        APP_RUN_VALUE,
                        "/t",
                        "REG_SZ",
                        "/d",
                        &command,
                        "/f",
                    ],
                )?;
            }
        }
        if let Some(old) = self.login_entry(Entry::OldSprite)? {
            self.remove_login_entry(Entry::OldSprite, &old)?;
        }
        Ok(())
    }

    /// Stop starting the app (or v0.4.0's sprite) at login. False if neither was set to.
    pub fn uninstall_app(&self) -> Result<bool> {
        let mut removed = false;
        for entry in [Entry::App, Entry::OldSprite] {
            if let Some(location) = self.login_entry(entry)? {
                self.remove_login_entry(entry, &location)?;
                removed = true;
            }
        }
        Ok(removed)
    }

    /// Where the app's login entry is (or v0.4.0's sprite entry), if there is one.
    pub fn app_installed(&self) -> Result<Option<String>> {
        match self.login_entry(Entry::App)? {
            Some(location) => Ok(Some(location)),
            None => self.login_entry(Entry::OldSprite),
        }
    }

    fn login_entry(&self, entry: Entry) -> Result<Option<String>> {
        let (file, label, value) = match entry {
            Entry::App => (APP_DESKTOP_FILE, APP_LAUNCHD_LABEL, APP_RUN_VALUE),
            Entry::OldSprite => (
                OLD_SPRITE_DESKTOP_FILE,
                OLD_SPRITE_LAUNCHD_LABEL,
                OLD_SPRITE_RUN_VALUE,
            ),
        };
        let path = match &self.platform {
            Platform::Systemd { autostart_dir, .. } => autostart_dir.join(file),
            Platform::Launchd { agents_dir, .. } => agents_dir.join(format!("{label}.plist")),
            Platform::RunKey => {
                let ran = self
                    .runner
                    .run("reg", &["query", RUN_KEY, "/v", value])
                    .context("couldn't run `reg`")?;
                return Ok(ran.ok.then(|| format!(r"{RUN_KEY}\{value}")));
            }
        };
        Ok(path.exists().then(|| path.display().to_string()))
    }

    fn remove_login_entry(&self, entry: Entry, location: &str) -> Result<()> {
        match &self.platform {
            Platform::RunKey => {
                let value = match entry {
                    Entry::App => APP_RUN_VALUE,
                    Entry::OldSprite => OLD_SPRITE_RUN_VALUE,
                };
                self.run_ok("reg", &["delete", RUN_KEY, "/v", value, "/f"])
            }
            _ => remove(Path::new(location)),
        }
    }

    fn plist_path(&self, agents_dir: &Path) -> PathBuf {
        agents_dir.join(format!("{LAUNCHD_LABEL}.plist"))
    }

    fn systemctl(&self, args: &[&str]) -> Result<()> {
        let mut all = vec!["--user"];
        all.extend_from_slice(args);
        self.run_ok("systemctl", &all)
    }

    /// Run a command, turning a failure into an error that shows what it printed.
    fn run_ok(&self, program: &str, args: &[&str]) -> Result<()> {
        let ran = self
            .runner
            .run(program, args)
            .with_context(|| format!("couldn't run `{program}`"))?;
        if !ran.ok {
            let command = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            bail!("`{command}` failed: {}", ran.output);
        }
        Ok(())
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("couldn't write {}", path.display()))
}

fn remove(path: &Path) -> Result<()> {
    std::fs::remove_file(path).with_context(|| format!("couldn't remove {}", path.display()))
}

// --- Commands ----------------------------------------------------------------------------------

/// This OS's service manager, or an error saying Lucy doesn't know one.
fn manager() -> Result<ServiceManager<SystemRunner>> {
    ServiceManager::for_this_os().context("Lucy doesn't know this system's service manager")
}

/// `lucy service install`: stop any running core, register the service and start it.
pub async fn install(force: bool) -> Result<()> {
    let endpoint = Endpoint::default_for_user();
    let manager = manager()?;
    let log = core_process::log_path().context("no data directory; set LUCY_DATA_DIR")?;
    let socket = match &endpoint {
        Endpoint::Path(path) => Some(path.as_path()),
        _ => None,
    };
    let spec = ServiceSpec::current(log, socket)?;
    // systemd and launchd open the log themselves and won't create its folder.
    if let Some(dir) = spec.log.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("couldn't create {}", dir.display()))?;
    }

    if core_process::stop(&endpoint, force).await? {
        println!("Stopped the running core.");
    }
    manager.install(&spec)?;
    let location = manager
        .installed()?
        .map_or_else(String::new, |installed| installed.location);
    if !manager.start()? {
        core_process::spawn_core(true)?;
    }
    let status = core_process::wait_until_up(&endpoint).await?;
    println!("Installed Lucy's core as a login service ({location}).");
    println!(
        "It's running now (v{}, pid {}) and starts whenever you log in.",
        status.core_version, status.pid
    );
    println!(
        "It runs {}; after upgrading Lucy, run `lucy service restart`.",
        spec.exe.display()
    );
    for note in install_notes(&spec) {
        println!("{note}");
    }
    app_at_login(&manager, socket)?;
    Ok(())
}

/// Add the desktop app to the login items when `[app] autostart` (or the older
/// `[sprite] autostart`) is on, and take it off when it's off.
fn app_at_login<R: Runner>(manager: &ServiceManager<R>, socket: Option<&Path>) -> Result<()> {
    let autostart = lucy_core::config::default_path()
        .map(|path| lucy_core::config::Config::load(&path))
        .transpose()?
        .flatten()
        .is_some_and(|config| config.starts_at_login());
    if !autostart {
        if manager.uninstall_app()? {
            println!("Took Lucy's app off your login items ([app] autostart is off).");
        }
        return Ok(());
    }
    let Some(exe) = crate::sprite::find() else {
        println!(
            "[app] autostart is on, but Lucy's desktop app (lucy-desktop) isn't installed; \
             install it and run `lucy service install` again."
        );
        return Ok(());
    };
    let log = crate::sprite::log_path().context("no data directory; set LUCY_DATA_DIR")?;
    // The desktop session has its own PATH; the app finds `lucy` next to it.
    let env = socket.map_or_else(Vec::new, |socket| {
        capture_env(std::env::vars(), socket)
            .into_iter()
            .filter(|(name, _)| name != "PATH")
            .collect()
    });
    manager.install_app(&ServiceSpec { exe, log, env })?;
    let location = manager.app_installed()?.unwrap_or_default();
    println!("Lucy's app starts at login too ({location}); `lucy app` opens it now.");
    Ok(())
}

/// Things worth knowing about the environment the service was installed from.
fn install_notes(spec: &ServiceSpec) -> Vec<String> {
    let mut notes = Vec::new();
    if cfg!(windows) {
        if std::env::vars().any(|(name, _)| name.starts_with("LUCY_")) {
            notes.push(
                "The service sees your user environment variables, not this shell's; set any \
                 LUCY_* you need with `setx`."
                    .into(),
            );
        }
    } else {
        notes.push(
            "It uses this shell's PATH and LUCY_* settings; re-run `lucy service install` if \
             they change."
                .into(),
        );
    }
    if std::env::vars().any(|(name, _)| name.starts_with("LUCY_KEY_") || name.ends_with("_API_KEY"))
    {
        notes.push(
            "API keys in environment variables don't reach the service; keep them in the \
             keyring with `lucy auth set <name>`."
                .into(),
        );
    }
    let dev_build = spec
        .exe
        .components()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            pair[0].as_os_str() == "target"
                && matches!(pair[1].as_os_str().to_str(), Some("debug" | "release"))
        });
    if dev_build {
        notes.push(
            "That's a development build; for everyday use, `cargo install --path \
             crates/lucy-cli` and install the service again."
                .into(),
        );
    }
    notes
}

/// `lucy service uninstall`: stop the service's core and remove the service.
pub async fn uninstall(force: bool) -> Result<()> {
    let endpoint = Endpoint::default_for_user();
    let manager = manager()?;
    let Some(installed) = manager.installed()? else {
        println!("Lucy's core isn't installed as a service.");
        if let Some(location) = manager.app_installed()? {
            manager.uninstall_app()?;
            println!("Took Lucy's app off your login items ({location}).");
        }
        return Ok(());
    };
    if let Ok(Some(status)) = core_process::status(&endpoint).await
        && status.service
    {
        core_process::stop(&endpoint, force).await?;
    }
    manager.uninstall()?;
    println!(
        "Removed Lucy's login service ({}). The core starts on demand again.",
        installed.location
    );
    if let Some(location) = manager.app_installed()? {
        manager.uninstall_app()?;
        println!("Took Lucy's app off your login items ({location}).");
    }
    Ok(())
}

/// `lucy service restart`: stop the service's core and start it again (picking up a new
/// build).
pub async fn restart(force: bool) -> Result<()> {
    let endpoint = Endpoint::default_for_user();
    if core_process::service_for(&endpoint).is_none() {
        bail!(
            "Lucy's core isn't installed as a service (`lucy service install`). `lucy core \
             stop` stops a core started on demand; the next `lucy` starts a fresh one."
        );
    }
    core_process::stop(&endpoint, force).await?;
    core_process::start(&endpoint)?;
    let status = core_process::wait_until_up(&endpoint).await?;
    println!(
        "Restarted Lucy's core (v{}, pid {}).",
        status.core_version, status.pid
    );
    Ok(())
}

/// `lucy service status`.
pub async fn show_status() -> Result<()> {
    let endpoint = Endpoint::default_for_user();
    let (installed, app) = match ServiceManager::for_this_os() {
        Some(manager) => (manager.installed()?, manager.app_installed()?),
        None => (None, None),
    };
    let service = match &installed {
        None => "not installed (`lucy service install` starts the core at login)".to_owned(),
        Some(installed) => match (&installed.socket, &endpoint) {
            (Some(socket), Endpoint::Path(path)) if socket != path => format!(
                "installed for another socket, {} ({})",
                socket.display(),
                installed.location
            ),
            _ => format!("installed, starts at login ({})", installed.location),
        },
    };
    println!("Service:  {service}");
    if let Some(location) = app {
        println!("App:      starts at login ({location})");
    }
    match core_process::status(&endpoint).await {
        Err(_) => println!(
            "Core:     an older version is running; stop it with `pkill -f \"lucy core run\"`"
        ),
        Ok(None) => println!("Core:     not running (the next `lucy` starts it)"),
        Ok(Some(status)) => {
            let how = if status.service {
                "as a service"
            } else {
                "on demand, until 30 idle minutes"
            };
            println!(
                "Core:     running {how} · v{} · pid {}",
                status.core_version, status.pid
            );
            for chat in &status.busy {
                let title = chat.title.as_deref().unwrap_or("(untitled)");
                println!("Busy:     \"{title}\"");
            }
        }
    }
    println!("Socket:   {endpoint}");
    if let Some(log) = core_process::log_path() {
        println!("Log:      {}", log.display());
    }
    Ok(())
}

/// A login entry the desktop app manages: its own, or the sprite's from v0.4.0.
#[derive(Clone, Copy)]
enum Entry {
    App,
    OldSprite,
}
