//! Which tool calls run by themselves, for each mode and agent level (ADR 0004 and 0008).
//! Pure: the caller describes the call; this decides. Config rules and "always" approvals are
//! applied before this, by the tool box.

use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Needs the user's approval, for this reason.
    Ask(String),
}

pub fn decide(call: &CallFacts, policy: &Policy) -> Decision {
    let ask = |reason: String| Decision::Ask(reason);
    // Secrets always ask, at every level.
    if !call.remote
        && let Some(path) = call.paths.iter().find(|p| is_private(p, policy.private))
    {
        return ask(format!("touches a private path ({})", path.display()));
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
        CallKind::Write => {
            let (area, name) = match policy.level {
                AgentLevel::Workspace => (workspace, "the workspace"),
                _ => (policy.folder, "the working folder"),
            };
            match call.paths.iter().find(|p| !within(p, area)) {
                None => Decision::Run,
                Some(path) => ask(format!(
                    "changes a file outside {name} ({})",
                    path.display()
                )),
            }
        }
        CallKind::Command
            if agent
                && policy.level == AgentLevel::Workspace
                && within(policy.folder, workspace) =>
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
/// don't exist yet resolve through their nearest existing parent.
pub fn real_path(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => real_path(parent).join(name),
        _ => path.to_owned(),
    }
}
