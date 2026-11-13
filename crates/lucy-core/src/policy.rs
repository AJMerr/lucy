//! Which tool calls run by themselves, for each mode and agent level (ADR 0004 and 0008).
//! Pure: the caller describes the call; this decides. Config rules and "always" approvals are
//! applied before this, by the tool box.

use std::path::{Component, Path, PathBuf};

use lucy_protocol::{AgentLevel, Mode};
use lucy_tools::PrivatePaths;

/// What a call does, as far as the rules care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// Reads, and stays on this machine (or reads a remote service's own data).
    Read,
    /// Changes local files at known paths.
    Write,
    /// Runs a shell command in the working folder.
    Command,
    /// Goes online (fetches, posts) or otherwise reaches outside this machine.
    Online,
    /// Unknown side effects: an unlabelled or changing tool from another server.
    Other,
}

#[derive(Debug, Clone)]
pub struct CallFacts {
    pub kind: CallKind,
    /// From a server reached over the network: its paths aren't on this machine.
    pub remote: bool,
    /// Absolute paths the call names (relative ones already resolved).
    pub paths: Vec<PathBuf>,
}

pub struct Policy<'a> {
    pub mode: Mode,
    pub level: AgentLevel,
    /// The chat's working folder.
    pub folder: &'a Path,
    pub home: Option<&'a Path>,
    /// `[agent] workspace`; the working folder when unset.
    pub workspace: Option<&'a Path>,
    pub private: &'a PrivatePaths,
    /// Files that run code at startup, and Lucy's own settings: changing them always asks.
    pub protected: &'a PrivatePaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Needs the user's approval, for this reason.
    Ask(String),
}

/// Why `call` asks even when config rules or an "always" approval would let it run: it
/// touches a private path, changes a protected file, or (below level 3) changes a file outside
/// the folder it may write in.
pub fn always_asks(call: &CallFacts, policy: &Policy) -> Option<String> {
    if call.remote {
        return None;
    }
    if let Some(path) = call.paths.iter().find(|p| is_private(p, policy.private)) {
        return Some(format!("touches a private path ({})", path.display()));
    }
    if let Some(path) = protected_change(call, policy) {
        return Some(protected_reason(path));
    }
    let unrestricted = policy.mode == Mode::Agent && policy.level == AgentLevel::Unrestricted;
    if call.kind == CallKind::Write && !unrestricted {
        match write_area(policy) {
            Some((area, name)) => {
                if let Some(path) = call.paths.iter().find(|p| !within(p, area)) {
                    return Some(format!(
                        "changes a file outside {name} ({})",
                        path.display()
                    ));
                }
            }
            None => return Some(HOME_WRITE.into()),
        }
    }
    None
}

/// Why a change in a folder that is (or holds) home asks.
const HOME_WRITE: &str = "changes a file in your home folder (set [agent] workspace to a folder \
                          she may change freely)";

/// A file `call` would change that runs code at startup or holds Lucy's settings: such a
/// change asks at every level (and from another machine, is refused).
pub fn protected_change<'c>(call: &'c CallFacts, policy: &Policy) -> Option<&'c PathBuf> {
    if call.remote || call.kind != CallKind::Write {
        return None;
    }
    call.paths.iter().find(|p| is_private(p, policy.protected))
}

fn protected_reason(path: &Path) -> String {
    format!(
        "changes a file that runs code at startup or holds Lucy's settings ({})",
        path.display()
    )
}

/// Where an agent's file changes may run without asking, and what to call it. Never home (or
/// a folder holding it): chats with no folder of their own work there, and home holds what
/// runs at login.
fn write_area<'a>(policy: &Policy<'a>) -> Option<(&'a Path, &'static str)> {
    let (area, name) = match (policy.mode, policy.level) {
        (Mode::Agent, AgentLevel::Workspace) => {
            (policy.workspace.unwrap_or(policy.folder), "the workspace")
        }
        _ => (policy.folder, "the working folder"),
    };
    let holds_home = policy.home.is_some_and(|home| within(home, area));
    (!holds_home).then_some((area, name))
}

pub fn decide(call: &CallFacts, policy: &Policy) -> Decision {
    let ask = |reason: String| Decision::Ask(reason);
    // Secrets always ask, at every level.
    if !call.remote
        && let Some(path) = call.paths.iter().find(|p| is_private(p, policy.private))
    {
        return ask(format!("touches a private path ({})", path.display()));
    }
    // So do files that run at startup or hold Lucy's settings.
    if let Some(path) = protected_change(call, policy) {
        return ask(protected_reason(path));
    }
    let agent = policy.mode == Mode::Agent;
    if agent && policy.level == AgentLevel::Supervised {
        return ask("supervised: every call asks".into());
    }
    if agent && policy.level == AgentLevel::Unrestricted {
        return Decision::Run;
    }
    let workspace = policy.workspace.unwrap_or(policy.folder);

    match call.kind {
        CallKind::Read if call.remote => Decision::Run,
        CallKind::Read => match call.paths.iter().find(|p| {
            // An agent may read wherever it may write.
            let writable = agent
                && (within(p, policy.folder)
                    || (policy.level == AgentLevel::Workspace && within(p, workspace)));
            !within_home(p, policy) && !writable
        }) {
            None => Decision::Run,
            Some(path) => ask(format!(
                "reads outside your home folder ({})",
                path.display()
            )),
        },
        CallKind::Write if !agent => ask("changes a file".into()),
        CallKind::Write if call.paths.is_empty() => ask("changes files it doesn't name".into()),
        CallKind::Write => match write_area(policy) {
            None => ask(HOME_WRITE.into()),
            Some((area, name)) => match call.paths.iter().find(|p| !within(p, area)) {
                None => Decision::Run,
                Some(path) => ask(format!(
                    "changes a file outside {name} ({})",
                    path.display()
                )),
            },
        },
        // Commands run freely only in a workspace that isn't home (or a folder holding it).
        CallKind::Command
            if agent
                && policy.level == AgentLevel::Workspace
                && within(policy.folder, workspace)
                && !policy.home.is_some_and(|home| within(home, workspace)) =>
        {
            Decision::Run
        }
        CallKind::Command => ask("runs a shell command".into()),
        CallKind::Online => ask("goes online".into()),
        CallKind::Other if call.remote => ask("can change things on a remote service".into()),
        CallKind::Other => ask("can change or delete things".into()),
    }
}

fn is_private(path: &Path, private: &PrivatePaths) -> bool {
    private.contains(path) || private.contains(&real_path(path))
}

fn within_home(path: &Path, policy: &Policy) -> bool {
    policy.home.is_some_and(|home| within(path, home))
}

/// Whether `path`, symlinks resolved, is `area` or inside it.
fn within(path: &Path, area: &Path) -> bool {
    real_path(path).starts_with(real_path(area))
}

/// `path` with symlinks resolved, so a link can't smuggle a call past the rules. Paths that
/// don't exist yet resolve through their nearest existing parent, the way the system will once
/// the missing folders are created: `new/../../x` is two folders up, not inside `new`.
pub fn real_path(path: &Path) -> PathBuf {
    real_path_within(path, 40)
}

/// [`real_path`], following at most `links` dangling symlinks (a loop ends there).
fn real_path_within(path: &Path, links: u32) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    // A link to something that doesn't exist yet: where it points is what gets written.
    if links > 0
        && let Ok(target) = std::fs::read_link(path)
    {
        let target = match path.parent() {
            Some(parent) => parent.join(target),
            None => target,
        };
        return real_path_within(&target, links - 1);
    }
    let mut components = path.components();
    match components.next_back() {
        Some(Component::Normal(name)) => real_path_within(components.as_path(), links).join(name),
        Some(Component::ParentDir) => {
            let above = real_path_within(components.as_path(), links);
            match above.parent() {
                Some(up) => up.to_owned(),
                None => above,
            }
        }
        Some(Component::CurDir) => real_path_within(components.as_path(), links),
        _ => path.to_owned(),
    }
}
