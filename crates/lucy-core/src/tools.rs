//! The tools offered in one chat turn, and the rules deciding which calls run by themselves
//! (ADR 0004 and 0008). Permission checks live here, in the core, never in front ends.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lucy_protocol::rpc::{ToolInfo, ToolRuleInfo};
use lucy_protocol::{Message, Role, ToolCall};
use lucy_providers::ToolSpec;
use lucy_tools::PrivatePaths;
use lucy_tools::builtin::{self, BuiltinOptions};
use lucy_tools::paths::resolve;
use rmcp::RoleClient;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use serde_json::Value;

use crate::config::{ToolRule, ToolSettings};

pub const BUILTIN: &str = "builtin";
/// Longest tool output passed to the model.
const MAX_OUTPUT: usize = 100 * 1024;
/// Argument names treated as file paths when checking where a read-only tool looks.
const PATH_KEYS: &[&str] = &["path", "paths", "file", "files", "dir", "directory", "root"];

struct Tool {
    server: String,
    def: rmcp::model::Tool,
}

/// What to do with one tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Run,
    /// Needs the user's approval first.
    Ask {
        reason: String,
        preview: Option<String>,
    },
    /// Don't run it; tell the model why.
    Refuse(String),
}

pub struct ToolBox {
    builtin: Option<RunningService<RoleClient, ()>>,
    tools: Vec<Tool>,
    cwd: PathBuf,
    home: Option<PathBuf>,
    private: PrivatePaths,
    settings: ToolSettings,
}

impl ToolBox {
    /// Connect the tool servers for a turn working in `cwd`.
    pub async fn open(settings: &ToolSettings, cwd: PathBuf) -> Result<Self, String> {
        let home = dirs::home_dir();
        let private = PrivatePaths::new(&settings.private_paths, home.as_deref())
            .map_err(|err| format!("bad pattern in tools.private_paths: {err}"))?;
        let mut tools = Vec::new();
        let builtin = if settings.builtin {
            let client = builtin::connect(BuiltinOptions {
                cwd: cwd.clone(),
                private: private.clone(),
            })
            .await
            .map_err(|err| format!("built-in tools failed to start: {err}"))?;
            let listed = client
                .list_all_tools()
                .await
                .map_err(|err| format!("could not list built-in tools: {err}"))?;
            tools.extend(listed.into_iter().map(|def| Tool {
                server: BUILTIN.into(),
                def,
            }));
            Some(client)
        } else {
            None
        };
        Ok(Self {
            builtin,
            tools,
            cwd,
            home,
            private,
            settings: settings.clone(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The tools to offer the model (minus any the config denies outright).
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .filter(|t| self.settings.rules.get(t.def.name.as_ref()) != Some(&ToolRule::Deny))
            .map(|t| ToolSpec {
                name: t.def.name.to_string(),
                description: t.def.description.as_deref().unwrap_or_default().to_owned(),
                input_schema: Value::Object((*t.def.input_schema).clone()),
            })
            .collect()
    }

    pub fn infos(&self, allowed: &HashSet<String>) -> Vec<ToolInfo> {
        self.tools
            .iter()
            .map(|t| ToolInfo {
                name: t.def.name.to_string(),
                server: t.server.clone(),
                description: first_sentence(t.def.description.as_deref().unwrap_or_default()),
                rule: self.rule_info(t, allowed),
            })
            .collect()
    }

    fn rule_info(&self, tool: &Tool, allowed: &HashSet<String>) -> ToolRuleInfo {
        let name = tool.def.name.as_ref();
        match self.settings.rules.get(name) {
            Some(ToolRule::Auto) => ToolRuleInfo::Auto,
            Some(ToolRule::Ask) => ToolRuleInfo::Ask,
            Some(ToolRule::Deny) => ToolRuleInfo::Deny,
            None if allowed.contains(name) => ToolRuleInfo::Auto,
            None if reads_locally(tool) => ToolRuleInfo::AutoInHome,
            None => ToolRuleInfo::Ask,
        }
    }

    /// Whether `call` may run now. `allowed` holds tools the user approved "always" in this
    /// session.
    pub fn verdict(&self, call: &ToolCall, allowed: &HashSet<String>) -> Verdict {
        let Some(tool) = self.find(&call.name) else {
            return Verdict::Refuse(format!("There is no tool named `{}`.", call.name));
        };
        if !call.arguments.is_object() {
            return Verdict::Refuse(format!(
                "The arguments weren't a valid JSON object ({}). Call the tool again with valid \
                 arguments.",
                call.arguments
            ));
        }
        let ask = |reason: &str| Verdict::Ask {
            reason: reason.to_owned(),
            preview: self.preview(tool, call),
        };
        match self.settings.rules.get(&call.name) {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Auto) => return Verdict::Run,
            Some(ToolRule::Ask) => return ask("your settings say to ask"),
            None => {}
        }
        if allowed.contains(&call.name) {
            return Verdict::Run;
        }
        if reads_locally(tool) {
            return match self
                .paths(tool, &call.arguments)
                .find_map(|p| self.concern(&p))
            {
                Some(reason) => ask(&reason),
                None => Verdict::Run,
            };
        }
        ask(&risk(tool))
    }

    /// Run a call, returning what to tell the model and whether it failed.
    pub async fn call(&self, call: &ToolCall) -> (String, bool) {
        let Some(tool) = self.find(&call.name) else {
            return (format!("There is no tool named `{}`.", call.name), true);
        };
        let client = match tool.server.as_str() {
            BUILTIN => self.builtin.as_ref(),
            _ => None,
        };
        let Some(client) = client else {
            return (format!("The {} server isn't running.", tool.server), true);
        };
        let mut params = CallToolRequestParams::new(call.name.clone());
        if let Value::Object(arguments) = &call.arguments {
            params = params.with_arguments(arguments.clone());
        }
        match client.call_tool(params).await {
            Ok(result) => {
                let mut parts: Vec<String> = result
                    .content
                    .iter()
                    .map(|c| match c.as_text() {
                        Some(text) => text.text.clone(),
                        None => "[non-text content omitted]".into(),
                    })
                    .collect();
                if parts.is_empty()
                    && let Some(structured) = &result.structured_content
                {
                    parts.push(structured.to_string());
                }
                let text = truncate(&parts.join("\n"));
                (text, result.is_error == Some(true))
            }
            Err(err) => (format!("The tool failed: {err}"), true),
        }
    }

    pub async fn close(self) {
        if let Some(client) = self.builtin {
            let _ = client.cancel().await;
        }
    }

    fn find(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.def.name == name)
    }

    fn preview(&self, tool: &Tool, call: &ToolCall) -> Option<String> {
        match tool.server.as_str() {
            BUILTIN => builtin::preview(&call.name, &call.arguments, &self.cwd),
            _ => None,
        }
    }

    /// Every path a call would touch. A tool taking an optional `path` defaults to the
    /// working folder.
    fn paths<'a>(
        &'a self,
        tool: &Tool,
        arguments: &'a Value,
    ) -> impl Iterator<Item = PathBuf> + 'a {
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Value::Object(args) = arguments {
            for (key, value) in args {
                let is_path = PATH_KEYS.contains(&key.as_str())
                    || key.ends_with("_path")
                    || key.ends_with("_dir");
                if !is_path {
                    continue;
                }
                match value {
                    Value::String(p) => paths.push(resolve(p, &self.cwd)),
                    Value::Array(items) => paths.extend(
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|p| resolve(p, &self.cwd)),
                    ),
                    _ => {}
                }
            }
            let takes_path = tool
                .def
                .input_schema
                .get("properties")
                .and_then(|p| p.get("path"))
                .is_some();
            if takes_path && !args.contains_key("path") {
                paths.push(self.cwd.clone());
            }
        }
        paths.into_iter()
    }

    /// Why reading `path` needs approval, if it does.
    fn concern(&self, path: &Path) -> Option<String> {
        let real = real_path(path);
        if self.private.contains(path) || self.private.contains(&real) {
            return Some(format!("reads a private path ({})", path.display()));
        }
        let inside_home = self
            .home
            .as_deref()
            .is_some_and(|home| real.starts_with(real_path(home)));
        (!inside_home).then(|| format!("reads outside your home folder ({})", path.display()))
    }
}

/// Read-only and stays on this machine: runs without asking (paths permitting).
fn reads_locally(tool: &Tool) -> bool {
    let hints = tool.def.annotations.as_ref();
    hints.and_then(|h| h.read_only_hint) == Some(true)
        && hints.and_then(|h| h.open_world_hint) != Some(true)
}

/// Why a tool that isn't a local read needs approval.
fn risk(tool: &Tool) -> String {
    let hints = tool.def.annotations.clone().unwrap_or_default();
    let reason = match (tool.server.as_str(), tool.def.name.as_ref()) {
        (BUILTIN, "run_command") => "runs a shell command",
        (BUILTIN, "write_file" | "edit_file") => "changes a file",
        (BUILTIN, "fetch_url") => "goes online",
        _ if hints.read_only_hint == Some(true) => "reaches outside this computer",
        // Unlabelled tools are treated as the MCP spec says: possibly destructive.
        _ if hints.destructive_hint != Some(false) => "can change or delete things",
        _ => "can change things",
    };
    reason.to_owned()
}

/// `path` with symlinks resolved, so a link can't smuggle a private file past the rules. Paths
/// that don't exist yet resolve through their nearest existing parent.
fn real_path(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => real_path(parent).join(name),
        _ => path.to_owned(),
    }
}

fn first_sentence(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    match line.find(". ") {
        Some(end) => line[..=end].to_owned(),
        None => line.to_owned(),
    }
}

fn truncate(text: &str) -> String {
    if text.len() <= MAX_OUTPUT {
        return text.to_owned();
    }
    let mut cut = MAX_OUTPUT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[… output cut off at 100 KB]", &text[..cut])
}

/// Give every tool call in `history` a result. A reply interrupted (or a core stopped) between
/// asking for tools and running them leaves calls unanswered, which providers reject.
pub fn close_dangling_calls(history: Vec<Message>) -> Vec<Message> {
    const NO_RESULT: &str = "No result: the reply was interrupted before this ran.";
    let mut out = Vec::with_capacity(history.len());
    let mut pending: Vec<String> = Vec::new();
    for message in history {
        if message.role == Role::Tool {
            if let Some(id) = &message.tool_call_id {
                pending.retain(|p| p != id);
            }
            out.push(message);
            continue;
        }
        out.extend(
            pending
                .drain(..)
                .map(|id| Message::tool_result(id, NO_RESULT, true)),
        );
        pending = message.tool_calls.iter().map(|c| c.id.clone()).collect();
        out.push(message);
    }
    out.extend(
        pending
            .drain(..)
            .map(|id| Message::tool_result(id, NO_RESULT, true)),
    );
    out
}
