//! `lucy service`: the files each OS's service manager reads, and the commands that register
//! them, run against a fake manager.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use lucy_cli::service::{
    Platform, Ran, Runner, ServiceManager, ServiceSpec, capture_env, desktop_entry, launchd_plist,
    run_key_command, systemd_unit,
};

fn spec(exe: &str, log: &str, env: &[(&str, &str)]) -> ServiceSpec {
    ServiceSpec {
        exe: exe.into(),
        log: log.into(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn plain_spec() -> ServiceSpec {
    spec(
        "/home/a b/.cargo/bin/lucy",
        "/home/a b/.local/share/lucy/core.log",
        &[
            ("LUCY_SOCKET", "/run/user/1000/lucy/core.sock"),
            ("PATH", "/home/a b/.cargo/bin:/usr/bin"),
        ],
    )
}

// --- Service files ---------------------------------------------------------------------------

#[test]
fn systemd_unit_runs_the_core_as_a_service_and_logs_to_core_log() {
    let unit = systemd_unit(&plain_spec());

    let lines: Vec<&str> = unit.lines().collect();
    for expected in [
        r#"ExecStart="/home/a b/.cargo/bin/lucy" core run --service"#,
        r#"Environment="LUCY_SOCKET=/run/user/1000/lucy/core.sock""#,
        r#"Environment="PATH=/home/a b/.cargo/bin:/usr/bin""#,
        "StandardOutput=append:/home/a b/.local/share/lucy/core.log",
        "StandardError=append:/home/a b/.local/share/lucy/core.log",
        "Restart=on-failure",
        "WantedBy=default.target",
    ] {
        assert!(lines.contains(&expected), "missing {expected:?} in\n{unit}");
    }
}

#[test]
fn systemd_unit_escapes_specifiers_variables_and_quotes() {
    let unit = systemd_unit(&spec(
        r#"/opt/50% off/$HOME/"q"\lucy"#,
        "/tmp/100%/core.log",
        &[("NOTE", r#"a"b\c%d$e"#)],
    ));

    let lines: Vec<&str> = unit.lines().collect();
    for expected in [
        r#"ExecStart="/opt/50%% off/$$HOME/\"q\"\\lucy" core run --service"#,
        r#"Environment="NOTE=a\"b\\c%%d$e""#,
        "StandardError=append:/tmp/100%%/core.log",
    ] {
        assert!(lines.contains(&expected), "missing {expected:?} in\n{unit}");
    }
}

#[test]
fn launchd_plist_runs_the_core_at_login_and_restarts_it_after_a_crash() {
    let plist = launchd_plist(&spec(
        "/Users/a&b/<x>/lucy",
        "/Users/a&b/Library/Application Support/lucy/core.log",
        &[("PATH", "/opt/homebrew/bin:/usr/bin")],
    ));

    let squashed: String = plist.split_whitespace().collect();
    for expected in [
        "<key>Label</key><string>io.github.ajmerr.lucy.core</string>",
        "<key>ProgramArguments</key><array><string>/Users/a&amp;b/&lt;x&gt;/lucy</string>\
         <string>core</string><string>run</string><string>--service</string></array>",
        "<key>EnvironmentVariables</key><dict><key>PATH</key>\
         <string>/opt/homebrew/bin:/usr/bin</string></dict>",
        "<key>RunAtLoad</key><true/>",
        "<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>",
        "<key>StandardErrorPath</key><string>/Users/a&amp;b/Library/ApplicationSupport/lucy/core.log</string>",
    ] {
        assert!(
            squashed.contains(expected),
            "missing {expected:?} in\n{plist}"
        );
    }
}

#[test]
fn run_key_command_quotes_the_program() {
    assert_eq!(
        run_key_command(Path::new(r"C:\Users\A B\.cargo\bin\lucy.exe")),
        r#""C:\Users\A B\.cargo\bin\lucy.exe" core run --service"#
    );
}

#[test]
fn the_service_gets_path_and_where_lucy_keeps_things_but_no_keys() {
    let vars = [
        ("PATH", "/usr/bin"),
        ("HOME", "/home/a"),
        ("FOO", "bar"),
        ("LUCY_CONFIG", "/etc/lucy.toml"),
        ("LUCY_DATA_DIR", "/data/lucy"),
        ("LUCY_KEY_OPENAI", "sk-secret"),
        ("LUCY_CORE_IDLE_SECS", "5"),
        ("LUCY_SOCKET", "/old.sock"),
        ("XDG_CONFIG_HOME", "/home/a/.cfg"),
        ("XDG_DATA_HOME", "/home/a/.data"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("LUCY_ODD", "two\nlines"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));

    let env = capture_env(vars, Path::new("/run/user/1000/lucy/core.sock"));

    let expected = [
        ("LUCY_CONFIG", "/etc/lucy.toml"),
        ("LUCY_DATA_DIR", "/data/lucy"),
        ("LUCY_SOCKET", "/run/user/1000/lucy/core.sock"),
        ("PATH", "/usr/bin"),
        ("XDG_CONFIG_HOME", "/home/a/.cfg"),
        ("XDG_DATA_HOME", "/home/a/.data"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    assert_eq!(env, expected);
}

// --- Registering with a (fake) service manager ------------------------------------------------

/// Records every command; any whose text starts with a `failing` prefix fails with that output.
/// Asking whether the service is still running says yes `still_running` more times, then no.
#[derive(Default)]
struct FakeRunner {
    calls: RefCell<Vec<String>>,
    failing: Vec<(&'static str, &'static str)>,
    still_running: std::cell::Cell<usize>,
}

impl FakeRunner {
    fn failing(failing: &[(&'static str, &'static str)]) -> Self {
        Self {
            failing: failing.to_vec(),
            ..Self::default()
        }
    }

    fn take(&self) -> Vec<String> {
        self.calls.take()
    }
}

impl Runner for &FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Ran> {
        let call = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        if call.starts_with("systemctl --user is-active") || call.starts_with("launchctl print") {
            let running = self.still_running.get();
            self.still_running.set(running.saturating_sub(1));
            self.calls.borrow_mut().push(call);
            return Ok(Ran {
                ok: running > 0,
                output: if running > 0 {
                    "state = running"
                } else {
                    "inactive"
                }
                .into(),
            });
        }
        let failure = self
            .failing
            .iter()
            .find(|(prefix, _)| call.starts_with(prefix));
        self.calls.borrow_mut().push(call);
        Ok(match failure {
            Some((_, output)) => Ran {
                ok: false,
                output: output.to_string(),
            },
            None => Ran {
                ok: true,
                output: String::new(),
            },
        })
    }
}

fn systemd<'a>(dir: &Path, runner: &'a FakeRunner) -> ServiceManager<&'a FakeRunner> {
    ServiceManager::new(
        Platform::Systemd {
            unit_dir: dir.join("systemd/user"),
            autostart_dir: dir.join("autostart"),
        },
        runner,
    )
}

fn unit_path(dir: &Path) -> PathBuf {
    dir.join("systemd/user/lucy-core.service")
}

#[test]
fn systemd_install_writes_the_unit_and_enables_it() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);

    manager.install(&plain_spec()).unwrap();

    let unit = std::fs::read_to_string(unit_path(dir.path())).unwrap();
    assert!(unit.contains("core run --service"), "{unit}");
    assert_eq!(
        runner.take(),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable lucy-core.service",
        ]
    );
    let installed = manager.installed().unwrap().expect("installed");
    assert_eq!(
        installed.socket.as_deref(),
        Some(Path::new("/run/user/1000/lucy/core.sock"))
    );
}

#[test]
fn reinstalling_replaces_the_unit() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);
    manager.install(&plain_spec()).unwrap();

    let moved = spec(
        "/usr/local/bin/lucy",
        "/var/log/lucy.log",
        &[("LUCY_SOCKET", "/run/lucy.sock")],
    );
    manager.install(&moved).unwrap();

    let unit = std::fs::read_to_string(unit_path(dir.path())).unwrap();
    assert!(unit.contains("/usr/local/bin/lucy"), "{unit}");
    assert!(!unit.contains(".cargo"), "{unit}");
    let entries = std::fs::read_dir(dir.path().join("systemd/user")).unwrap();
    assert_eq!(entries.count(), 1);
}

#[test]
fn systemd_uninstall_disables_and_removes_the_unit_once() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);
    manager.install(&plain_spec()).unwrap();
    runner.take();

    assert!(manager.uninstall().unwrap());

    assert!(!unit_path(dir.path()).exists());
    assert_eq!(
        runner.take(),
        [
            "systemctl --user disable --now lucy-core.service",
            "systemctl --user daemon-reload",
        ]
    );
    assert!(manager.installed().unwrap().is_none());
    assert!(!manager.uninstall().unwrap(), "nothing left to remove");
    assert!(runner.take().is_empty());
}

#[test]
fn systemd_start_asks_systemctl() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();

    assert!(systemd(dir.path(), &runner).start().unwrap());

    assert_eq!(
        runner.take(),
        [
            "systemctl --user is-active --quiet lucy-core.service",
            "systemctl --user start lucy-core.service",
        ]
    );
}

#[test]
fn start_waits_for_a_core_that_is_still_shutting_down() {
    // Right after `lucy core stop` the socket is gone but the process may still be exiting,
    // and starting a unit systemd still sees as running does nothing.
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner {
        still_running: 2.into(),
        ..FakeRunner::default()
    };

    assert!(systemd(dir.path(), &runner).start().unwrap());

    assert_eq!(
        runner.take(),
        [
            "systemctl --user is-active --quiet lucy-core.service",
            "systemctl --user is-active --quiet lucy-core.service",
            "systemctl --user is-active --quiet lucy-core.service",
            "systemctl --user start lucy-core.service",
        ]
    );
}

#[test]
fn a_failing_systemctl_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::failing(&[(
        "systemctl --user enable",
        "Failed to connect to bus: No medium found",
    )]);

    let err = systemd(dir.path(), &runner)
        .install(&plain_spec())
        .unwrap_err();

    let message = format!("{err:#}");
    assert!(message.contains("Failed to connect to bus"), "{message}");
    assert!(message.contains("systemctl --user enable"), "{message}");
}

#[test]
fn the_installed_socket_survives_awkward_characters() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let awkward = r#"/tmp/a "b" \c %d $e & <f>/core.sock"#;
    let with_socket = spec("/bin/lucy", "/tmp/core.log", &[("LUCY_SOCKET", awkward)]);
    let launchd = ServiceManager::new(
        Platform::Launchd {
            agents_dir: dir.path().join("LaunchAgents"),
            uid: 501,
        },
        &runner,
    );

    for manager in [systemd(dir.path(), &runner), launchd] {
        manager.install(&with_socket).unwrap();
        let installed = manager.installed().unwrap().expect("installed");
        assert_eq!(installed.socket.as_deref(), Some(Path::new(awkward)));
    }
}

#[test]
fn launchd_install_reloads_the_agent_and_uninstall_unloads_it() {
    let dir = tempfile::tempdir().unwrap();
    // Booting out an agent that isn't loaded fails; install doesn't mind.
    let runner =
        FakeRunner::failing(&[("launchctl bootout", "Boot-out failed: 3: No such process")]);
    let manager = ServiceManager::new(
        Platform::Launchd {
            agents_dir: dir.path().join("LaunchAgents"),
            uid: 501,
        },
        &runner,
    );
    let plist = dir
        .path()
        .join("LaunchAgents")
        .join("io.github.ajmerr.lucy.core.plist");

    manager.install(&plain_spec()).unwrap();
    assert!(plist.exists());
    assert_eq!(
        runner.take(),
        [
            "launchctl bootout gui/501/io.github.ajmerr.lucy.core".to_string(),
            format!("launchctl bootstrap gui/501 {}", plist.display()),
        ]
    );

    assert!(manager.start().unwrap());
    assert_eq!(
        runner.take(),
        [
            "launchctl print gui/501/io.github.ajmerr.lucy.core",
            "launchctl kickstart gui/501/io.github.ajmerr.lucy.core",
        ]
    );

    assert!(manager.uninstall().unwrap());
    assert!(!plist.exists());
    assert_eq!(
        runner.take(),
        ["launchctl bootout gui/501/io.github.ajmerr.lucy.core"]
    );
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[test]
fn run_key_install_and_uninstall_use_reg() {
    let runner = FakeRunner::default();
    let manager = ServiceManager::new(Platform::RunKey, &runner);
    let exe = spec(r"C:\Users\A B\.cargo\bin\lucy.exe", r"C:\log.txt", &[]);

    manager.install(&exe).unwrap();
    assert_eq!(
        runner.take(),
        [format!(
            r#"reg add {RUN_KEY} /v Lucy /t REG_SZ /d "C:\Users\A B\.cargo\bin\lucy.exe" core run --service /f"#
        )]
    );

    let installed = manager.installed().unwrap().expect("installed");
    assert_eq!(installed.socket, None);
    assert_eq!(runner.take(), [format!("reg query {RUN_KEY} /v Lucy")]);

    assert!(
        !manager.start().unwrap(),
        "no manager to start it; Lucy spawns it"
    );
    assert!(runner.take().is_empty());

    assert!(manager.uninstall().unwrap());
    assert_eq!(
        runner.take(),
        [
            format!("reg query {RUN_KEY} /v Lucy"),
            format!("reg delete {RUN_KEY} /v Lucy /f"),
        ]
    );
}

#[test]
fn a_missing_run_key_is_not_installed() {
    let runner = FakeRunner::failing(&[("reg query", "ERROR: The system was unable to find")]);
    let manager = ServiceManager::new(Platform::RunKey, &runner);

    assert!(manager.installed().unwrap().is_none());
    assert!(!manager.uninstall().unwrap());
    assert_eq!(
        runner.take(),
        [
            format!("reg query {RUN_KEY} /v Lucy"),
            format!("reg query {RUN_KEY} /v Lucy"),
        ]
    );
}

// --- The app at login -------------------------------------------------------------------------

fn app_spec() -> ServiceSpec {
    spec(
        "/home/a b/.cargo/bin/lucy-desktop",
        "/home/a b/.local/share/lucy/desktop.log",
        &[("LUCY_SOCKET", "/run/user/1000/lucy/core.sock")],
    )
}

#[test]
fn desktop_entry_starts_the_app_in_the_background_with_lucys_settings() {
    let entry = desktop_entry(&app_spec());

    let lines: Vec<&str> = entry.lines().collect();
    for expected in [
        "[Desktop Entry]",
        "Type=Application",
        "Name=Lucy",
        r#"Exec=env "LUCY_SOCKET=/run/user/1000/lucy/core.sock" "/home/a b/.cargo/bin/lucy-desktop" background"#,
        "NoDisplay=true",
    ] {
        assert!(
            lines.contains(&expected),
            "missing {expected:?} in\n{entry}"
        );
    }
}

#[test]
fn desktop_entry_escapes_what_exec_lines_treat_specially() {
    let entry = desktop_entry(&spec(
        r#"/opt/50% off/$x/"q"\desktop"#,
        "/tmp/desktop.log",
        &[("NOTE", "a`b")],
    ));

    // Quoted arguments escape " ` $ and \ with a backslash; then the value as a whole doubles
    // backslashes and %.
    let exec = entry
        .lines()
        .find(|line| line.starts_with("Exec="))
        .expect("an Exec line");
    assert_eq!(
        exec,
        r#"Exec=env "NOTE=a\\`b" "/opt/50%% off/\\$x/\\"q\\"\\\\desktop" background"#
    );
}

#[test]
fn systemd_app_autostart_is_a_desktop_entry_added_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);
    let entry = dir.path().join("autostart").join("lucy-desktop.desktop");
    assert_eq!(manager.app_installed().unwrap(), None);

    manager.install_app(&app_spec()).unwrap();

    let text = std::fs::read_to_string(&entry).unwrap();
    assert!(text.contains("lucy-desktop"), "{text}");
    assert_eq!(
        manager.app_installed().unwrap(),
        Some(entry.display().to_string())
    );
    assert!(manager.uninstall_app().unwrap());
    assert!(!entry.exists());
    assert!(!manager.uninstall_app().unwrap(), "already gone");
    assert!(
        runner.take().is_empty(),
        "the session reads the folder itself"
    );
}

#[test]
fn the_sprites_old_login_entries_are_cleared_away() {
    // v0.4.0 started `lucy-sprite` at login; the app replaces it.
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);
    let old = dir.path().join("autostart").join("lucy-sprite.desktop");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "[Desktop Entry]\nExec=lucy-sprite\n").unwrap();
    assert_eq!(
        manager.app_installed().unwrap(),
        Some(old.display().to_string()),
        "the old entry still counts as starting it at login"
    );

    manager.install_app(&app_spec()).unwrap();
    assert!(!old.exists(), "installing replaces it");

    manager.uninstall_app().unwrap();
    std::fs::write(&old, "[Desktop Entry]\nExec=lucy-sprite\n").unwrap();
    assert!(
        manager.uninstall_app().unwrap(),
        "uninstalling removes it too"
    );
    assert!(!old.exists());
}

#[test]
fn launchd_app_autostart_is_an_agent_for_the_next_login() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = ServiceManager::new(
        Platform::Launchd {
            agents_dir: dir.path().join("LaunchAgents"),
            uid: 501,
        },
        &runner,
    );
    let plist = dir
        .path()
        .join("LaunchAgents")
        .join("io.github.ajmerr.lucy.desktop.plist");

    manager.install_app(&app_spec()).unwrap();

    let text = std::fs::read_to_string(&plist).unwrap();
    for expected in [
        "<string>io.github.ajmerr.lucy.desktop</string>",
        "<string>/home/a b/.cargo/bin/lucy-desktop</string>",
        "<string>background</string>",
        "<key>LUCY_SOCKET</key>",
        "<key>RunAtLoad</key>",
        "<string>/home/a b/.local/share/lucy/desktop.log</string>",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(!text.contains("KeepAlive"), "quitting it keeps it gone");
    assert!(runner.take().is_empty(), "it starts at the next login");
    assert_eq!(
        manager.app_installed().unwrap(),
        Some(plist.display().to_string())
    );

    assert!(manager.uninstall_app().unwrap());
    assert!(!plist.exists());
}

#[test]
fn run_key_app_autostart_uses_its_own_value() {
    // The old LucySprite value isn't there, so asking about it fails.
    let runner = FakeRunner::failing(&[("reg query", "ERROR: not found")]);
    let manager = ServiceManager::new(Platform::RunKey, &runner);
    let app = spec(
        r"C:\Program Files\Lucy\lucy-desktop.exe",
        r"C:\log.txt",
        &[],
    );

    manager.install_app(&app).unwrap();
    assert_eq!(
        runner.take(),
        [
            format!(
                r#"reg add {RUN_KEY} /v LucyDesktop /t REG_SZ /d "C:\Program Files\Lucy\lucy-desktop.exe" background /f"#
            ),
            format!("reg query {RUN_KEY} /v LucySprite"),
        ]
    );
}

#[test]
fn a_unit_that_pins_the_socket_under_the_runtime_dir_is_understood() {
    // How the home-manager module writes it, since it can't know the runtime dir.
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let manager = systemd(dir.path(), &runner);
    std::fs::create_dir_all(dir.path().join("systemd/user")).unwrap();
    std::fs::write(
        unit_path(dir.path()),
        "[Service]\nExecStart=/nix/store/x-lucy/bin/lucy core run --service\n\
         Environment=\"LUCY_SOCKET=%t/lucy/core.sock\"\n",
    )
    .unwrap();

    let installed = manager.installed().unwrap().expect("installed");
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => assert_eq!(
            installed.socket,
            Some(PathBuf::from(runtime).join("lucy/core.sock"))
        ),
        None => assert_eq!(installed.socket, None, "nowhere to resolve %t"),
    }
}
