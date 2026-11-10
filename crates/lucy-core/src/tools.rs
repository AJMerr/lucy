//! The tools offered in one chat turn, and the rules deciding which calls run by themselves
//! (ADR 0004 and 0008). Permission checks live here, in the core, never in front ends.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

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

use lucy_protocol::{AgentLevel, Mode};

use crate::config::{ToolRule, ToolSettings};
use crate::generate::{self, Generator};
use crate::mcp::ExternalServer;
pub use crate::mcp::ToolOutput;
use crate::policy::{self, CallFacts, CallKind, Decision, Policy};

pub const BUILTIN: &str = "builtin";
/// What `generate_image` is listed under, and the rule name that covers it.
pub const GENERATOR_SERVER: &str = "images";

/// The picture maker for `generate_image`, or why there isn't one.
pub type GeneratorSlot = Result<Arc<Generator>, String>;
/// Longest tool output passed to the model.
const MAX_OUTPUT: usize = 100 * 1024;
/// Providers accept tool names of at most 64 characters from `[a-zA-Z0-9_-]`.
const MAX_NAME: usize = 64;
/// Argument names treated as file paths when checking where a read-only tool looks.
const PATH_KEYS: &[&str] = &["path", "paths", "file", "files", "dir", "directory", "root"];

struct Tool {
    /// What the model calls it: the built-in's own name, or `<server>__<tool>`.
    name: String,
    server: String,
    def: rmcp::model::Tool,
    /// External servers' tools; `None` for built-ins.
    external: Option<Arc<ExternalServer>>,
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
    access: Access,
    /// `generate_image`, when a generator is set: ready, or why it can't make pictures.
    generator: Option<GeneratorSlot>,
}

/// The chat's mode and agent level, which decide how much runs without asking.
#[derive(Debug, Clone, Default)]
pub struct Access {
    pub mode: Mode,
    pub level: AgentLevel,
    /// `[agent] workspace`, home-expanded.
    pub workspace: Option<PathBuf>,
}

impl ToolBox {
    /// The tools for a turn working in `cwd`: the built-ins (if on) plus `external` servers'.
    pub async fn open(
        settings: &ToolSettings,
        cwd: PathBuf,
        external: Vec<Arc<ExternalServer>>,
    ) -> Result<Self, String> {
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
                name: def.name.to_string(),
                server: BUILTIN.into(),
                def,
                external: None,
            }));
            Some(client)
        } else {
            None
        };
        for server in external {
            for def in server.tools() {
                tools.push(Tool {
                    name: exposed_name(server.name(), &def.name),
                    server: server.name().to_owned(),
                    def: def.clone(),
                    external: Some(Arc::clone(&server)),
                });
            }
        }
        Ok(Self {
            builtin,
            tools,
            cwd,
            home,
            private,
            settings: settings.clone(),
            access: Access::default(),
            generator: None,
        })
    }

    /// Apply a chat's mode and level (Assist by default).
    pub fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Offer `generate_image`, made by `generator`.
    pub fn with_generator(mut self, generator: Option<GeneratorSlot>) -> Self {
        self.generator = generator;
        self
    }

    /// The config rule for `generate_image`: by its name, then by `images`.
    fn generator_rule(&self) -> Option<ToolRule> {
        let rules = &self.settings.rules;
        rules
            .get(generate::TOOL)
            .or_else(|| rules.get(GENERATOR_SERVER))
            .copied()
    }

    fn generator_spec(&self) -> Option<ToolSpec> {
        self.generator.as_ref()?;
        (self.generator_rule() != Some(ToolRule::Deny)).then(|| ToolSpec {
            name: generate::TOOL.into(),
            description: generate::DESCRIPTION.into(),
            input_schema: generate::input_schema(),
        })
    }

    /// Hosted generators cost money, so they ask; ComfyUI on this machine just runs.
    fn generator_verdict(
        &self,
        slot: &GeneratorSlot,
        call: &ToolCall,
        allowed: &HashSet<String>,
    ) -> Verdict {
        let preview = call.arguments["prompt"].as_str().map(str::to_owned);
        let ask = |reason: String| Verdict::Ask {
            reason,
            preview: preview.clone(),
        };
        let generator = match slot {
            Ok(generator) => generator,
            // Nothing to pay for; the call reports what's wrong.
            Err(_) => return Verdict::Run,
        };
        match self.generator_rule() {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Auto) => return Verdict::Run,
            Some(ToolRule::Ask) => return ask("your settings say to ask".into()),
            None => {}
        }
        if allowed.contains(&call.name) {
            return Verdict::Run;
        }
        let agent = self.access.mode == Mode::Agent;
        match self.access.level {
            AgentLevel::Supervised if agent => ask("supervised: every call asks".into()),
            AgentLevel::Unrestricted if agent => Verdict::Run,
            _ if generator.hosted => ask(format!(
                "makes a picture with {}, which costs money",
                generator.label
            )),
            _ => Verdict::Run,
        }
    }

    fn generator_info(&self, allowed: &HashSet<String>) -> Option<ToolInfo> {
        let slot = self.generator.as_ref()?;
        let rule = match (self.generator_rule(), slot) {
            (Some(ToolRule::Auto), _) => ToolRuleInfo::Auto,
            (Some(ToolRule::Ask), _) => ToolRuleInfo::Ask,
            (Some(ToolRule::Deny), _) => ToolRuleInfo::Deny,
            (None, _) if allowed.contains(generate::TOOL) => ToolRuleInfo::Auto,
            (None, Ok(generator)) if generator.hosted => ToolRuleInfo::Ask,
            (None, _) => ToolRuleInfo::Auto,
        };
        Some(ToolInfo {
            name: generate::TOOL.into(),
            server: GENERATOR_SERVER.into(),
            description: first_sentence(generate::DESCRIPTION),
            rule,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.generator.is_none()
    }

    /// The tools to offer the model (minus any the config denies outright).
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .filter(|t| self.rule(t) != Some(ToolRule::Deny))
            .map(|t| ToolSpec {
                name: t.name.clone(),
                description: t.def.description.as_deref().unwrap_or_default().to_owned(),
                input_schema: Value::Object((*t.def.input_schema).clone()),
            })
            .chain(self.generator_spec())
            .collect()
    }

    pub fn infos(&self, allowed: &HashSet<String>) -> Vec<ToolInfo> {
        self.tools
            .iter()
            .map(|t| ToolInfo {
                name: t.name.clone(),
                server: t.server.clone(),
                description: first_sentence(t.def.description.as_deref().unwrap_or_default()),
                rule: self.rule_info(t, allowed),
            })
            .chain(self.generator_info(allowed))
            .collect()
    }

    /// The config rule for a tool: by its own name first, then by its server's name.
    fn rule(&self, tool: &Tool) -> Option<ToolRule> {
        let rules = &self.settings.rules;
        rules
            .get(&tool.name)
            .or_else(|| rules.get(&tool.server))
            .copied()
    }

    fn rule_info(&self, tool: &Tool, allowed: &HashSet<String>) -> ToolRuleInfo {
        match self.rule(tool) {
            Some(ToolRule::Auto) => return ToolRuleInfo::Auto,
            Some(ToolRule::Ask) => return ToolRuleInfo::Ask,
            Some(ToolRule::Deny) => return ToolRuleInfo::Deny,
            None if allowed.contains(&tool.name) => return ToolRuleInfo::Auto,
            None => {}
        }
        // How a typical call would go: reads and writes at the working folder.
        let kind = self.kind(tool);
        let facts = CallFacts {
            kind,
            remote: is_remote(tool),
            paths: match kind {
                CallKind::Read | CallKind::Write if !is_remote(tool) => vec![self.cwd.clone()],
                _ => vec![],
            },
        };
        match (policy::decide(&facts, &self.policy()), kind) {
            (Decision::Ask(_), _) => ToolRuleInfo::Ask,
            (Decision::Run, _) if facts.remote || self.access.level == AgentLevel::Unrestricted => {
                ToolRuleInfo::Auto
            }
            (Decision::Run, CallKind::Read) => ToolRuleInfo::AutoInHome,
            (Decision::Run, CallKind::Write) => ToolRuleInfo::AutoInFolder,
            (Decision::Run, _) => ToolRuleInfo::Auto,
        }
    }

    /// Whether `call` may run now. `allowed` holds tools the user approved "always" in this
    /// session.
    pub fn verdict(&self, call: &ToolCall, allowed: &HashSet<String>) -> Verdict {
        if call.name == generate::TOOL
            && let Some(slot) = &self.generator
        {
            return self.generator_verdict(slot, call, allowed);
        }
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
        let ask = |reason: String| Verdict::Ask {
            reason,
            preview: self.preview(tool, call),
        };
        match self.rule(tool) {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Auto) => return Verdict::Run,
            Some(ToolRule::Ask) => return ask("your settings say to ask".into()),
            None => {}
        }
        if allowed.contains(&call.name) {
            return Verdict::Run;
        }
        let facts = CallFacts {
            kind: self.kind(tool),
            remote: is_remote(tool),
            paths: if is_remote(tool) {
                vec![]
            } else {
                self.paths(tool, &call.arguments).collect()
            },
        };
        match policy::decide(&facts, &self.policy()) {
            Decision::Run => Verdict::Run,
            Decision::Ask(reason) => ask(reason),
        }
    }

    /// The paths a built-in write or edit will change, for undo.
    pub fn changed_files(&self, call: &ToolCall) -> Vec<PathBuf> {
        match self.find(&call.name) {
            Some(tool) if tool.external.is_none() && self.kind(tool) == CallKind::Write => {
                self.paths(tool, &call.arguments).collect()
            }
            _ => vec![],
        }
    }

    fn policy(&self) -> Policy<'_> {
        Policy {
            mode: self.access.mode,
            level: self.access.level,
            folder: &self.cwd,
            home: self.home.as_deref(),
            workspace: self.access.workspace.as_deref(),
            private: &self.private,
        }
    }

    /// What a tool does, for the rules: known for built-ins, from labels for the rest.
    fn kind(&self, tool: &Tool) -> CallKind {
        if tool.external.is_none() {
            match tool.def.name.as_ref() {
                "read_file" | "list_dir" | "search_files" => return CallKind::Read,
                "write_file" | "edit_file" => return CallKind::Write,
                "run_command" => return CallKind::Command,
                "fetch_url" => return CallKind::Online,
                _ => {}
            }
        }
        let hints = tool.def.annotations.clone().unwrap_or_default();
        let read_only = hints.read_only_hint == Some(true);
        match (read_only, hints.open_world_hint) {
            // A remote service's own data: reading it is what you connected it for.
            (true, _) if is_remote(tool) => CallKind::Read,
            (true, Some(true)) => CallKind::Online,
            (true, _) => CallKind::Read,
            // Unlabelled tools are treated as the MCP spec says: open world, possibly
            // destructive.
            (false, Some(false)) => CallKind::Write,
            (false, _) => CallKind::Other,
        }
    }

    pub async fn call(&self, call: &ToolCall) -> ToolOutput {
        if call.name == generate::TOOL
            && let Some(slot) = &self.generator
        {
            return match (slot, generate::request(&call.arguments)) {
                (Ok(generator), Ok(request)) => generator.run(&request).await,
                (Err(err), _) => ToolOutput::failed(format!("Couldn't make the picture: {err}")),
                (_, Err(err)) => ToolOutput::failed(err),
            };
        }
        let Some(tool) = self.find(&call.name) else {
            return ToolOutput::failed(format!("There is no tool named `{}`.", call.name));
        };
        let mut output = match (&tool.external, &self.builtin) {
            (Some(server), _) => server
                .call(&tool.def.name, call.arguments.clone())
                .await
                .unwrap_or_else(ToolOutput::failed),
            (None, Some(client)) => call_builtin(client, &tool.def.name, &call.arguments).await,
            (None, None) => ToolOutput::failed("The built-in tools are off."),
        };
        output.text = truncate(&output.text);
        output
    }

    pub async fn close(self) {
        if let Some(client) = self.builtin {
            let _ = client.cancel().await;
        }
    }

    fn find(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name == name)
    }

    fn preview(&self, tool: &Tool, call: &ToolCall) -> Option<String> {
        match tool.external {
            None => builtin::preview(&tool.def.name, &call.arguments, &self.cwd),
            Some(_) => None,
        }
    }

    /// Where a tool's relative paths start: the turn's folder, or the server's.
    fn base_dir(&self, tool: &Tool) -> PathBuf {
        match &tool.external {
            Some(server) => server.cwd().unwrap_or(&self.cwd).to_owned(),
            None => self.cwd.clone(),
        }
    }

    /// Every path a call would touch. A tool taking an optional `path` defaults to the
    /// working folder.
    fn paths<'a>(
        &'a self,
        tool: &Tool,
        arguments: &'a Value,
    ) -> impl Iterator<Item = PathBuf> + 'a {
        let base = self.base_dir(tool);
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
                    Value::String(p) => paths.push(resolve(p, &base)),
                    Value::Array(items) => paths.extend(
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|p| resolve(p, &base)),
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
                paths.push(base.clone());
            }
        }
        paths.into_iter()
    }
}

/// Reached over the network rather than started on this machine.
fn is_remote(tool: &Tool) -> bool {
    tool.external.as_ref().is_some_and(|s| s.cwd().is_none())
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
async fn call_builtin(
    client: &RunningService<RoleClient, ()>,
    name: &str,
    arguments: &Value,
) -> ToolOutput {
    let mut params = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(arguments) = arguments {
        params = params.with_arguments(arguments.clone());
    }
    match client.call_tool(params).await {
        Ok(result) => ToolOutput::from_result(result),
        Err(err) => ToolOutput::failed(format!("The tool failed: {err}")),
    }
}

/// `<server>__<tool>`, with anything providers reject replaced and the length capped.
fn exposed_name(server: &str, tool: &str) -> String {
    let name: String = format!("{server}__{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.chars().take(MAX_NAME).collect()
}
