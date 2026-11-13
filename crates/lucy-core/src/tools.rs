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
use serde_json::{Value, json};

use lucy_protocol::{AgentLevel, Mode};

use crate::config::{ToolRule, ToolSettings};
use crate::generate::{self, Generator};
use crate::mcp::ExternalServer;
pub use crate::mcp::ToolOutput;
use crate::memory::{self, Memory};
use crate::policy::{self, CallFacts, CallKind, Decision, Policy};

pub const BUILTIN: &str = "builtin";
/// What `generate_image` is listed under, and the rule name that covers it.
pub const GENERATOR_SERVER: &str = "images";
/// What the memory tools are listed under, and the rule name that covers them all.
pub const MEMORY_SERVER: &str = "memory";
/// What the scheduling tools are listed under, and the rule name that covers them all.
pub const SCHEDULES_SERVER: &str = "schedules";

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
    /// Files that run code at startup, and Lucy's own settings: changing them always asks.
    protected: PrivatePaths,
    settings: ToolSettings,
    access: Access,
    /// `generate_image`, when a generator is set: ready, or why it can't make pictures.
    generator: Option<GeneratorSlot>,
    /// `remember`, `forget` and `recall`, unless memory is off or the chat is incognito.
    memory: Option<Arc<Memory>>,
    /// `schedule`, `list_schedules` and `cancel_schedule`.
    schedules: Option<Arc<dyn ScheduleBook>>,
}

/// The chat's mode and agent level, which decide how much runs without asking.
#[derive(Debug, Clone, Default)]
pub struct Access {
    pub mode: Mode,
    pub level: AgentLevel,
    /// `[agent] workspace`, home-expanded.
    pub workspace: Option<PathBuf>,
    /// The chat is driven from another machine: memory changes ask, and protected paths
    /// are refused rather than asked about.
    pub remote: bool,
    /// Lucy's own config and data folders: changing anything in them always asks.
    pub lucy_folders: Vec<PathBuf>,
}

impl ToolBox {
    /// The tools for a turn working in `cwd`: the built-ins (if on) plus `external` servers'.
    pub async fn open(
        settings: &ToolSettings,
        cwd: PathBuf,
        external: Vec<Arc<ExternalServer>>,
    ) -> Result<Self, String> {
        let home = dirs::home_dir();
        let patterns: Vec<&String> = settings
            .private_paths
            .iter()
            .chain(&settings.extra_private_paths)
            .collect();
        let private = PrivatePaths::new(&patterns, home.as_deref())
            .map_err(|err| format!("bad pattern in tools.private_paths: {err}"))?;
        let protected = PrivatePaths::protected(home.as_deref(), &[]);
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
            protected,
            settings: settings.clone(),
            access: Access::default(),
            generator: None,
            memory: None,
            schedules: None,
        })
    }

    /// Apply a chat's mode and level (Assist by default).
    pub fn with_access(mut self, access: Access) -> Self {
        self.protected = PrivatePaths::protected(self.home.as_deref(), &access.lucy_folders);
        self.access = access;
        self
    }

    /// Offer `generate_image`, made by `generator`.
    pub fn with_generator(mut self, generator: Option<GeneratorSlot>) -> Self {
        self.generator = generator;
        self
    }

    /// Offer `remember`, `forget` and `recall` on `memory`.
    pub fn with_memory(mut self, memory: Option<Arc<Memory>>) -> Self {
        self.memory = memory;
        self
    }

    /// Offer `schedule`, `list_schedules` and `cancel_schedule`, kept in `book`.
    pub fn with_schedules(mut self, book: Option<Arc<dyn ScheduleBook>>) -> Self {
        self.schedules = book;
        self
    }

    fn schedule_rule(&self, name: &str) -> Option<ToolRule> {
        let rules = &self.settings.rules;
        rules
            .get(name)
            .or_else(|| rules.get(SCHEDULES_SERVER))
            .copied()
    }

    fn schedule_specs(&self) -> Vec<ToolSpec> {
        if self.schedules.is_none() {
            return Vec::new();
        }
        schedule_tools()
            .into_iter()
            .filter(|spec| self.schedule_rule(&spec.name) != Some(ToolRule::Deny))
            .collect()
    }

    fn schedule_infos(&self) -> Vec<ToolInfo> {
        if self.schedules.is_none() {
            return Vec::new();
        }
        schedule_tools()
            .into_iter()
            .map(|spec| ToolInfo {
                rule: match (self.schedule_rule(&spec.name), spec.name.as_str()) {
                    (Some(ToolRule::Deny), _) => ToolRuleInfo::Deny,
                    (_, SCHEDULE) => ToolRuleInfo::Ask,
                    (Some(ToolRule::Ask), _) => ToolRuleInfo::Ask,
                    _ => ToolRuleInfo::Auto,
                },
                description: first_sentence(&spec.description),
                server: SCHEDULES_SERVER.into(),
                name: spec.name,
            })
            .collect()
    }

    /// Scheduling always shows you what and when first ("always" doesn't skip it); listing
    /// and cancelling just run.
    fn schedule_verdict(&self, book: &dyn ScheduleBook, call: &ToolCall) -> Verdict {
        match self.schedule_rule(&call.name) {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Ask) if call.name != SCHEDULE => {
                return Verdict::Ask {
                    reason: "your settings say to ask".into(),
                    preview: None,
                };
            }
            _ => {}
        }
        if call.name != SCHEDULE {
            return Verdict::Run;
        }
        match book.describe(&call.arguments) {
            Ok((reason, preview)) => Verdict::Ask {
                reason,
                preview: Some(preview),
            },
            Err(err) => Verdict::Refuse(format!("Not scheduled: {err}. Call it again, fixed.")),
        }
    }

    fn schedule_call(&self, book: &dyn ScheduleBook, call: &ToolCall) -> ToolOutput {
        let done = match call.name.as_str() {
            SCHEDULE => book.create(&call.arguments),
            LIST_SCHEDULES => Ok(book.list()),
            _ => book.cancel(call.arguments["which"].as_str().unwrap_or_default()),
        };
        match done {
            Ok(text) => ToolOutput {
                text,
                ..ToolOutput::default()
            },
            Err(err) => ToolOutput::failed(err),
        }
    }

    /// The config rule for a memory tool: by its name, then by `memory`.
    fn memory_rule(&self, name: &str) -> Option<ToolRule> {
        let rules = &self.settings.rules;
        rules
            .get(name)
            .or_else(|| rules.get(MEMORY_SERVER))
            .copied()
    }

    fn memory_specs(&self) -> Vec<ToolSpec> {
        if self.memory.is_none() {
            return Vec::new();
        }
        memory_tools()
            .into_iter()
            .filter(|spec| self.memory_rule(&spec.name) != Some(ToolRule::Deny))
            .collect()
    }

    fn memory_infos(&self, allowed: &HashSet<String>) -> Vec<ToolInfo> {
        if self.memory.is_none() {
            return Vec::new();
        }
        memory_tools()
            .into_iter()
            .map(|spec| ToolInfo {
                rule: match self.memory_rule(&spec.name) {
                    Some(ToolRule::Auto) => ToolRuleInfo::Auto,
                    Some(ToolRule::Ask) => ToolRuleInfo::Ask,
                    Some(ToolRule::Deny) => ToolRuleInfo::Deny,
                    None if allowed.contains(&spec.name) => ToolRuleInfo::Auto,
                    None => ToolRuleInfo::Auto,
                },
                description: first_sentence(&spec.description),
                server: MEMORY_SERVER.into(),
                name: spec.name,
            })
            .collect()
    }

    /// Memory tools run by themselves, except a sensitive fact (always asks) and a secret
    /// (always refused).
    fn memory_verdict(&self, call: &ToolCall, allowed: &HashSet<String>) -> Verdict {
        let fact = call.arguments["fact"].as_str().unwrap_or_default();
        let ask = |reason: String| Verdict::Ask {
            reason,
            preview: (!fact.is_empty()).then(|| fact.to_owned()),
        };
        if call.name == MEMORY_REMEMBER {
            let heading = call.arguments["heading"].as_str().unwrap_or_default();
            if let Some(kind) = memory::looks_secret(fact).or_else(|| memory::looks_secret(heading))
            {
                return Verdict::Refuse(memory::refusal(kind));
            }
            if call.arguments["sensitive"].as_bool() == Some(true)
                && self.memory_rule(&call.name) != Some(ToolRule::Deny)
            {
                return ask("remembers something sensitive about you".into());
            }
        }
        // Memory steers every later chat, so another machine changes it only with approval.
        if self.access.remote && self.memory_rule(&call.name) != Some(ToolRule::Deny) {
            return ask("changes Lucy's memory from another machine".into());
        }
        match self.memory_rule(&call.name) {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Auto) => return Verdict::Run,
            Some(ToolRule::Ask) => return ask("your settings say to ask".into()),
            None => {}
        }
        let supervised =
            self.access.mode == Mode::Agent && self.access.level == AgentLevel::Supervised;
        if supervised && !allowed.contains(&call.name) {
            return ask("supervised: every call asks".into());
        }
        Verdict::Run
    }

    fn memory_call(&self, memory: &Memory, call: &ToolCall) -> ToolOutput {
        let arg = |key: &str| call.arguments[key].as_str().unwrap_or_default();
        let done = match call.name.as_str() {
            MEMORY_REMEMBER => {
                let replaces = Some(arg("replaces")).filter(|r| !r.is_empty());
                memory.remember(arg("heading"), arg("fact"), replaces)
            }
            MEMORY_FORGET => memory.forget(arg("fact")),
            _ => {
                let found = memory.recall(arg("query"), 20);
                Ok(if found.is_empty() {
                    "Nothing in memory matches that.".into()
                } else {
                    found
                        .iter()
                        .map(|fact| format!("- {} ({})", fact.text, fact.heading))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
        };
        match done {
            Ok(text) => ToolOutput {
                text,
                ..ToolOutput::default()
            },
            Err(err) => ToolOutput::failed(err),
        }
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
        self.tools.is_empty()
            && self.generator.is_none()
            && self.memory.is_none()
            && self.schedules.is_none()
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
            .chain(self.memory_specs())
            .chain(self.schedule_specs())
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
            .chain(self.memory_infos(allowed))
            .chain(self.schedule_infos())
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
        if is_memory_tool(&call.name) && self.memory.is_some() {
            return self.memory_verdict(call, allowed);
        }
        if is_schedule_tool(&call.name)
            && let Some(book) = &self.schedules
        {
            return self.schedule_verdict(book.as_ref(), call);
        }
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
        let rule = self.rule(tool);
        match rule {
            Some(ToolRule::Deny) => {
                return Verdict::Refuse("The user's settings don't allow this tool.".into());
            }
            Some(ToolRule::Ask) => return ask("your settings say to ask".into()),
            Some(ToolRule::Auto) | None => {}
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
        // From another machine, files that run at startup or hold Lucy's settings are off
        // limits.
        if self.access.remote
            && let Some(path) = policy::protected_change(&facts, &self.policy())
        {
            return Verdict::Refuse(format!(
                "{} runs code at startup or holds Lucy's settings, so it can't be changed from \
                 another machine.",
                path.display()
            ));
        }
        // Private and protected paths, and changes outside the folder, ask even for a tool set
        // to "auto" or approved "always".
        if let Some(reason) = policy::always_asks(&facts, &self.policy()) {
            return ask(reason);
        }
        if rule == Some(ToolRule::Auto) || allowed.contains(&call.name) {
            return Verdict::Run;
        }
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
            protected: &self.protected,
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
        if is_memory_tool(&call.name)
            && let Some(memory) = &self.memory
        {
            return self.memory_call(memory, call);
        }
        if is_schedule_tool(&call.name)
            && let Some(book) = &self.schedules
        {
            return self.schedule_call(book.as_ref(), call);
        }
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

const MEMORY_REMEMBER: &str = "remember";
const MEMORY_FORGET: &str = "forget";
const MEMORY_RECALL: &str = "recall";

fn is_memory_tool(name: &str) -> bool {
    matches!(name, MEMORY_REMEMBER | MEMORY_FORGET | MEMORY_RECALL)
}

/// `remember`, `forget` and `recall`, as offered to the model.
fn memory_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: MEMORY_REMEMBER.into(),
            description: "Save a fact about the user for future chats. Use it on your own, \
                          without being asked, whenever they tell you something worth \
                          keeping: who they are, preferences, people in their life, projects, \
                          ongoing plans. Say in a few words that you'll remember it. Set \
                          `sensitive` for health, money, relationships, home address or ID \
                          numbers: the user is asked first. Never save secrets (passwords, \
                          keys, card numbers). To change a fact, pass the old one as \
                          `replaces`."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "fact": {"type": "string", "description": "One short fact, e.g. \"Has a dog called Biscuit\"."},
                    "heading": {
                        "type": "string",
                        "description": "Where it goes: \"About you\", \"Preferences\", \"People\" or \"Projects\" (or another short heading).",
                    },
                    "sensitive": {"type": "boolean", "description": "Health, money, relationships, home address or ID numbers."},
                    "replaces": {"type": "string", "description": "The fact this one updates, as saved."},
                },
                "required": ["fact", "heading"],
            }),
        },
        ToolSpec {
            name: MEMORY_FORGET.into(),
            description: "Remove a saved fact about the user, when it's wrong, out of date, or \
                          they ask you to forget it."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "fact": {"type": "string", "description": "The fact as saved (or a unique part of it)."},
                },
                "required": ["fact"],
            }),
        },
        ToolSpec {
            name: MEMORY_RECALL.into(),
            description: "Search what you remember about the user, by keyword, when they mention \
                          something you might have saved that isn't in your prompt."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to look for, e.g. \"sister birthday\"."},
                },
                "required": ["query"],
            }),
        },
    ]
}

/// Where `schedule` keeps reminders and tasks (the core's schedules).
pub trait ScheduleBook: Send + Sync {
    /// Why it asks and what the card shows, or what's wrong with the arguments.
    fn describe(&self, arguments: &Value) -> Result<(String, String), String>;
    /// Save it; returns what to tell her.
    fn create(&self, arguments: &Value) -> Result<String, String>;
    fn list(&self) -> String;
    /// Delete the one `which` names (its ID or title).
    fn cancel(&self, which: &str) -> Result<String, String>;
}

const SCHEDULE: &str = "schedule";
const LIST_SCHEDULES: &str = "list_schedules";
const CANCEL_SCHEDULE: &str = "cancel_schedule";

fn is_schedule_tool(name: &str) -> bool {
    matches!(name, SCHEDULE | LIST_SCHEDULES | CANCEL_SCHEDULE)
}

/// `schedule`, `list_schedules` and `cancel_schedule`, as offered to the model.
fn schedule_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SCHEDULE.into(),
            description: "Set a reminder or a scheduled task. A reminder just shows the user its \
                          text at the time (\"remind me at 5 to call Mom\"). A task is a prompt \
                          you run on your own at the time, as an agent at `level` (0-3), and \
                          report on (\"every weekday at 8, summarise my email and calendar\"). \
                          Times are the user's local time (see the current date and time above). \
                          The user always sees what you're scheduling and approves it."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["reminder", "task"]},
                    "what": {"type": "string", "description": "A reminder's message, or a task's prompt (written as the instruction you'll follow then)."},
                    "title": {"type": "string", "description": "A short name, e.g. \"Morning summary\"."},
                    "when": {
                        "type": "object",
                        "description": "Exactly one of: {\"at\": \"2026-11-12T17:00\"} (once), {\"daily\": \"08:00\"}, {\"weekdays\": \"08:00\"}, {\"weekly\": {\"days\": [\"mon\", \"fri\"], \"time\": \"09:30\"}}, {\"monthly\": {\"day\": 1, \"time\": \"09:00\"}}, {\"every_minutes\": 30}.",
                    },
                    "level": {"type": "integer", "minimum": 0, "maximum": 3, "description": "Tasks: how freely you may act (0 supervised: nothing runs, 1 guarded, 2 workspace, 3 unrestricted). Default 1."},
                },
                "required": ["kind", "what", "when"],
            }),
        },
        ToolSpec {
            name: LIST_SCHEDULES.into(),
            description: "List the user's reminders and scheduled tasks, with when they run next."
                .into(),
            input_schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: CANCEL_SCHEDULE.into(),
            description: "Delete a reminder or scheduled task the user no longer wants.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "which": {"type": "string", "description": "Its title (or ID) from list_schedules."},
                },
                "required": ["which"],
            }),
        },
    ]
}
