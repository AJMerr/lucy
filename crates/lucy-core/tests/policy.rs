//! Which tool calls run by themselves, per mode and agent level (ADR 0004). Pure rules over a
//! real temp folder standing in for home, so symlinks resolve like they would on disk.

use std::path::{Path, PathBuf};

use lucy_core::policy::{CallFacts, CallKind, Decision, Policy, decide};
use lucy_protocol::{AgentLevel, Mode};
use lucy_tools::PrivatePaths;

struct Home {
    dir: tempfile::TempDir,
    /// Somewhere outside home.
    elsewhere: tempfile::TempDir,
    private: PrivatePaths,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["projects/app", "projects/other", ".ssh", "notes"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join(".ssh/config"), "Host x").unwrap();
        let private = PrivatePaths::defaults(Some(&real(dir.path())));
        Self {
            dir,
            elsewhere: tempfile::tempdir().unwrap(),
            private,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        real(self.dir.path()).join(rel)
    }

    fn outside(&self, rel: &str) -> PathBuf {
        real(self.elsewhere.path()).join(rel)
    }

    /// The chat works in `~/projects/app`; `workspace` is `[agent] workspace`.
    fn decide(
        &self,
        mode: Mode,
        level: AgentLevel,
        workspace: Option<&str>,
        call: CallFacts,
    ) -> Decision {
        let folder = self.path("projects/app");
        let home = self.path("");
        let workspace = workspace.map(|w| self.path(w));
        decide(
            &call,
            &Policy {
                mode,
                level,
                folder: &folder,
                home: Some(&home),
                workspace: workspace.as_deref(),
                private: &self.private,
            },
        )
    }

    fn agent(&self, level: AgentLevel, call: CallFacts) -> Decision {
        self.decide(Mode::Agent, level, None, call)
    }

    fn assist(&self, call: CallFacts) -> Decision {
        self.decide(Mode::Assist, AgentLevel::Guarded, None, call)
    }
}

/// Temp folders sit behind symlinks on macOS (/var -> /private/var); compare real paths.
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

fn local(kind: CallKind, paths: Vec<PathBuf>) -> CallFacts {
    CallFacts {
        kind,
        remote: false,
        paths,
    }
}

fn remote(kind: CallKind) -> CallFacts {
    CallFacts {
        kind,
        remote: true,
        paths: vec![PathBuf::from("/home/someone/.ssh/id_rsa")],
    }
}

fn runs(decision: &Decision) -> bool {
    *decision == Decision::Run
}

#[track_caller]
fn assert_runs(decision: Decision) {
    assert!(runs(&decision), "expected Run, got {decision:?}");
}

#[track_caller]
fn assert_asks(decision: Decision) {
    assert!(!runs(&decision), "expected Ask, got {decision:?}");
}

// --- Assist mode (unchanged from step 8) ---------------------------------------------------

#[test]
fn assist_runs_reads_in_home_and_asks_for_everything_else() {
    let home = Home::new();

    assert_runs(home.assist(local(CallKind::Read, vec![home.path("notes")])));
    assert_asks(home.assist(local(CallKind::Read, vec![home.outside("x")])));
    assert_asks(home.assist(local(CallKind::Read, vec![home.path(".ssh/config")])));
    assert_asks(home.assist(local(CallKind::Write, vec![home.path("projects/app/a.rs")])));
    assert_asks(home.assist(local(CallKind::Command, vec![])));
    assert_asks(home.assist(local(CallKind::Online, vec![])));
}

// --- Level 0: Supervised -------------------------------------------------------------------

#[test]
fn supervised_asks_for_every_call() {
    let home = Home::new();

    assert_asks(home.agent(
        AgentLevel::Supervised,
        local(CallKind::Read, vec![home.path("projects/app/a.rs")]),
    ));
    assert_asks(home.agent(AgentLevel::Supervised, remote(CallKind::Read)));
}

// --- Level 1: Guarded ----------------------------------------------------------------------

#[test]
fn guarded_writes_freely_inside_the_working_folder_only() {
    let home = Home::new();
    let g = AgentLevel::Guarded;

    assert_runs(home.agent(
        g,
        local(CallKind::Write, vec![home.path("projects/app/src/new.rs")]),
    ));
    assert_asks(home.agent(
        g,
        local(CallKind::Write, vec![home.path("projects/other/x.rs")]),
    ));
    assert_asks(home.agent(g, local(CallKind::Write, vec![home.outside("x.rs")])));
    // A write the rules can't place (no path argument) asks.
    assert_asks(home.agent(g, local(CallKind::Write, vec![])));
}

#[test]
fn guarded_reads_home_but_asks_for_commands_online_and_unknown_tools() {
    let home = Home::new();
    let g = AgentLevel::Guarded;

    assert_runs(home.agent(g, local(CallKind::Read, vec![home.path("notes")])));
    assert_asks(home.agent(g, local(CallKind::Read, vec![home.outside("x")])));
    assert_asks(home.agent(g, local(CallKind::Command, vec![])));
    assert_asks(home.agent(g, local(CallKind::Online, vec![])));
    assert_asks(home.agent(g, local(CallKind::Other, vec![])));
}

#[test]
fn remote_reads_run_and_remote_changes_ask_at_guarded() {
    let home = Home::new();

    assert_runs(home.agent(AgentLevel::Guarded, remote(CallKind::Read)));
    assert_asks(home.agent(AgentLevel::Guarded, remote(CallKind::Other)));
}

// --- Level 2: Workspace --------------------------------------------------------------------

#[test]
fn workspace_runs_local_work_anywhere_inside_it() {
    let home = Home::new();
    let w = AgentLevel::Workspace;
    let ws = Some("projects");

    let write = |path: PathBuf| local(CallKind::Write, vec![path]);
    assert_runs(home.decide(Mode::Agent, w, ws, write(home.path("projects/other/x.rs"))));
    assert_asks(home.decide(Mode::Agent, w, ws, write(home.path("notes/x.md"))));
    // Commands run when the chat's folder is inside the workspace.
    assert_runs(home.decide(Mode::Agent, w, ws, local(CallKind::Command, vec![])));
    assert_asks(home.decide(
        Mode::Agent,
        w,
        Some("notes"),
        local(CallKind::Command, vec![]),
    ));
    // Reads keep Guarded's reach.
    assert_runs(home.decide(
        Mode::Agent,
        w,
        ws,
        local(CallKind::Read, vec![home.path("notes")]),
    ));
}

#[test]
fn workspace_still_asks_before_going_online_or_running_unknown_tools() {
    let home = Home::new();
    let w = AgentLevel::Workspace;

    assert_asks(home.decide(
        Mode::Agent,
        w,
        Some("projects"),
        local(CallKind::Online, vec![]),
    ));
    assert_asks(home.decide(
        Mode::Agent,
        w,
        Some("projects"),
        local(CallKind::Other, vec![]),
    ));
}

#[test]
fn without_a_configured_workspace_the_working_folder_is_it() {
    let home = Home::new();
    let w = AgentLevel::Workspace;

    let write = |rel: &str| local(CallKind::Write, vec![home.path(rel)]);
    assert_runs(home.decide(Mode::Agent, w, None, write("projects/app/x.rs")));
    assert_asks(home.decide(Mode::Agent, w, None, write("projects/other/x.rs")));
}

// --- Level 3: Unrestricted -----------------------------------------------------------------

#[test]
fn unrestricted_runs_everything_but_private_paths() {
    let home = Home::new();
    let u = AgentLevel::Unrestricted;

    assert_runs(home.agent(u, local(CallKind::Command, vec![])));
    assert_runs(home.agent(u, local(CallKind::Online, vec![])));
    assert_runs(home.agent(u, local(CallKind::Other, vec![])));
    assert_runs(home.agent(u, local(CallKind::Write, vec![home.outside("x")])));
    assert_asks(home.agent(u, local(CallKind::Read, vec![home.path(".ssh/config")])));
    assert_asks(home.agent(
        u,
        local(CallKind::Write, vec![home.path(".ssh/authorized_keys")]),
    ));
}

// --- Private paths and symlinks, at every level ----------------------------------------------

#[test]
fn private_files_inside_the_working_folder_still_ask() {
    let home = Home::new();
    let env_file = home.path("projects/app/.env");

    for level in [
        AgentLevel::Guarded,
        AgentLevel::Workspace,
        AgentLevel::Unrestricted,
    ] {
        assert_asks(home.agent(level, local(CallKind::Write, vec![env_file.clone()])));
        assert_asks(home.agent(level, local(CallKind::Read, vec![env_file.clone()])));
    }
}

#[cfg(unix)]
#[test]
fn symlinks_are_judged_by_where_they_lead() {
    let home = Home::new();
    let to_ssh = home.path("projects/app/innocent.txt");
    std::os::unix::fs::symlink(home.path(".ssh/config"), &to_ssh).unwrap();
    let to_other = home.path("projects/app/shortcut");
    std::os::unix::fs::symlink(home.path("projects/other"), &to_other).unwrap();

    // Into ~/.ssh: private, so it asks even at Unrestricted.
    assert_asks(home.agent(
        AgentLevel::Unrestricted,
        local(CallKind::Write, vec![to_ssh.clone()]),
    ));
    // Out of the working folder: Guarded asks.
    assert_asks(home.agent(
        AgentLevel::Guarded,
        local(CallKind::Write, vec![to_other.join("x.rs")]),
    ));
}

#[test]
fn every_ask_says_why() {
    let home = Home::new();

    let Decision::Ask(reason) = home.agent(AgentLevel::Guarded, local(CallKind::Command, vec![]))
    else {
        panic!("should ask");
    };

    assert!(!reason.trim().is_empty());
}

#[test]
fn an_agent_reads_where_it_may_write_even_outside_home() {
    let home = Home::new();
    let folder = home.outside("project");
    std::fs::create_dir_all(&folder).unwrap();
    let decide_at = |mode: Mode, level: AgentLevel| {
        decide(
            &local(CallKind::Read, vec![folder.join("src")]),
            &Policy {
                mode,
                level,
                folder: &folder,
                home: Some(&home.path("")),
                workspace: None,
                private: &home.private,
            },
        )
    };

    assert_runs(decide_at(Mode::Agent, AgentLevel::Guarded));
    assert_runs(decide_at(Mode::Agent, AgentLevel::Workspace));
    // Assist mode keeps to home.
    assert_asks(decide_at(Mode::Assist, AgentLevel::Guarded));
}
