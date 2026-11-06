//! The interactive chat loop: read input, run slash commands, stream replies.

use std::future::Future;
use std::io::Write;

use anyhow::{Context, Result, bail};
use lucy_client::{Client, ClientError};
use lucy_protocol::rpc::{
    ApproveParams, CancelResult, ChatSendParams, ChatSendResult, Decision, Event, ProfileInfo,
    RenameParams, ResolveParams, Session, SessionParams, SessionSettings, SessionSummary,
    SetLevelParams, SetModeParams, SetPlainParams, SetProfileParams, ToolRuleInfo, ToolsList,
    ToolsListParams, TurnOutcome, UndoParams, UndoResult, method,
};
use lucy_protocol::{AgentLevel, Mode, Role, ToolCall};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::commands::{COMMANDS, Command, Input, parse};
use crate::markdown::{Op, Renderer};
use crate::tool_display;

pub enum ReadLine {
    Text(String),
    /// Ctrl+C at the prompt.
    Interrupted,
    /// Ctrl+D.
    Eof,
}

/// Where input lines come from: the terminal, or a script in tests.
pub trait LineSource {
    fn read_line(&mut self, prompt: &str) -> impl Future<Output = ReadLine>;
}

pub enum Start {
    New,
    /// Continue the most recently active session.
    Continue,
    /// Resume the session matching this ID prefix.
    Resume(String),
}

pub struct ChatOptions {
    /// Styled output with cursor movement; `false` writes plain text only.
    pub color: bool,
    /// Terminal width, for redrawing wrapped lines.
    pub width: usize,
    pub start: Start,
}

pub const PROMPT: &str = "you › ";

/// How many past messages to show when resuming.
const RECAP: usize = 4;

pub async fn run_chat<S: LineSource, W: Write>(
    client: &mut Client,
    input: &mut S,
    out: &mut W,
    interrupts: &mut mpsc::Receiver<()>,
    options: ChatOptions,
) -> Result<()> {
    let mut events = client.take_events().context("chat events already taken")?;
    let mut chat = Chat {
        client,
        out,
        color: options.color,
        width: options.width.max(20),
        session: None,
        last_tool: None,
    };

    chat.banner().await?;
    let start = match options.start {
        Start::New => Ok(()),
        Start::Continue => chat.continue_latest().await,
        Start::Resume(prefix) => chat.resume(&prefix).await,
    };
    if let Err(err) = start {
        chat.problem(&err)?;
    }

    loop {
        let text = match input.read_line(PROMPT).await {
            ReadLine::Eof => break,
            ReadLine::Interrupted => {
                chat.note("(Ctrl+D or /quit to leave)")?;
                continue;
            }
            ReadLine::Text(text) => text,
        };
        let outcome = match parse(&text) {
            Err(message) => chat.error(&message).map_err(Into::into),
            Ok(Input::Nothing) => Ok(()),
            Ok(Input::Command(Command::Quit)) => break,
            Ok(Input::Command(command)) => chat.command(command, input).await,
            Ok(Input::Message(text)) => chat.send(text, &mut events, interrupts, input).await,
        };
        if let Err(err) = outcome {
            chat.problem(&err)?;
        }
    }
    Ok(())
}

struct Chat<'a, W: Write> {
    client: &'a mut Client,
    out: &'a mut W,
    color: bool,
    width: usize,
    /// `None` until the first message (or a command that needs a session).
    session: Option<String>,
    /// The most recent tool call, its output and whether it failed, for `/last`.
    last_tool: Option<(ToolCall, String, bool)>,
}

impl<W: Write> Chat<'_, W> {
    async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R> {
        Ok(self.client.call(method, params).await?)
    }

    /// Show a failed command; only a lost connection ends the chat.
    fn problem(&mut self, err: &anyhow::Error) -> Result<()> {
        if matches!(err.downcast_ref(), Some(ClientError::Disconnected)) {
            bail!("lost connection to the Lucy core");
        }
        let message = match err.downcast_ref::<ClientError>() {
            Some(ClientError::Rpc(rpc)) => rpc.message.clone(),
            _ => format!("{err:#}"),
        };
        self.error(&message)?;
        Ok(())
    }

    fn styled(&self, codes: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{codes}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    fn say(&mut self, text: &str) -> std::io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.out.flush()
    }

    fn note(&mut self, text: &str) -> std::io::Result<()> {
        let text = self.styled("2", text);
        self.say(&text)
    }

    fn error(&mut self, text: &str) -> std::io::Result<()> {
        let text = self.styled("31", &format!("error: {text}"));
        self.say(&text)
    }

    async fn banner(&mut self) -> Result<()> {
        let profiles: Vec<ProfileInfo> = self.call(method::PROFILES_LIST, json!({})).await?;
        let default = profiles.iter().find(|p| p.is_default);
        let model = default.map_or(String::new(), |p| format!(" · {} ({})", p.name, p.model));
        let title = self.styled("1;35", "Lucy");
        let rest = self.styled(
            "2",
            &format!("{model} · /help for commands · Ctrl+D to quit"),
        );
        self.say(&format!("{title}{rest}"))?;
        Ok(())
    }

    async fn ensure_session(&mut self) -> Result<String> {
        if let Some(id) = &self.session {
            return Ok(id.clone());
        }
        let id: String = self.call(method::SESSIONS_CREATE, json!({})).await?;
        self.session = Some(id.clone());
        Ok(id)
    }

    async fn settings(&self) -> Result<Option<SessionSettings>> {
        match &self.session {
            Some(id) => Ok(Some(
                self.call(
                    method::SESSION_SETTINGS,
                    SessionParams {
                        session_id: id.clone(),
                    },
                )
                .await?,
            )),
            None => Ok(None),
        }
    }

    async fn continue_latest(&mut self) -> Result<()> {
        let sessions: Vec<SessionSummary> = self.call(method::SESSIONS_LIST, json!({})).await?;
        match sessions.first() {
            Some(latest) => self.resume(&latest.id.clone()).await,
            None => Ok(self.note("No saved chats yet; starting a new one.")?),
        }
    }

    async fn resume(&mut self, prefix: &str) -> Result<()> {
        let id: String = self
            .call(
                method::SESSIONS_RESOLVE,
                ResolveParams {
                    prefix: prefix.into(),
                },
            )
            .await?;
        let session: Session = self
            .call(
                method::SESSIONS_LOAD,
                SessionParams {
                    session_id: id.clone(),
                },
            )
            .await?;
        self.session = Some(id);
        let title = session.summary.title.as_deref().unwrap_or("untitled chat");
        self.note(&format!("Resumed “{title}”."))?;

        // Tool results are shown as the calls that made them, not as messages.
        let shown: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.role != Role::Tool)
            .collect();
        for message in &shown[shown.len().saturating_sub(RECAP)..] {
            match message.role {
                Role::User => {
                    let line = self.styled("36", &format!("{PROMPT}{}", message.content));
                    self.say(&line)?;
                }
                _ => {
                    if !message.content.is_empty() {
                        let mut renderer = Renderer::new(self.color);
                        let mut ops = renderer.push(&message.content);
                        ops.extend(renderer.finish());
                        let mut printer = Printer::new(self.color, self.width);
                        for op in ops {
                            printer.op(self.out, op)?;
                        }
                    }
                    for call in &message.tool_calls {
                        self.note(&tool_display::started(call))?;
                    }
                    if message.incomplete {
                        self.note("(cut off)")?;
                    }
                    if message.tool_calls.is_empty() {
                        self.say("")?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn send<S: LineSource>(
        &mut self,
        text: String,
        events: &mut mpsc::UnboundedReceiver<Event>,
        interrupts: &mut mpsc::Receiver<()>,
        input: &mut S,
    ) -> Result<()> {
        // A Ctrl+C pressed between replies must not cancel this one.
        while interrupts.try_recv().is_ok() {}
        let sent: ChatSendResult = self
            .call(
                method::CHAT_SEND,
                ChatSendParams {
                    session_id: self.session.clone(),
                    text,
                    cwd: std::env::current_dir()
                        .ok()
                        .map(|dir| dir.display().to_string()),
                },
            )
            .await?;
        self.session = Some(sent.session_id.clone());
        let session_id = sent.session_id.clone();

        let mut renderer = Renderer::new(self.color);
        let mut printer = Printer::new(self.color, self.width);
        // The call whose `⚙` line is printed and waiting for its outcome.
        let mut running: Option<ToolCall> = None;
        let (outcome, error) = loop {
            tokio::select! {
                event = events.recv() => match event {
                    None => bail!(ClientError::Disconnected),
                    Some(event) if turn_of(&event) != Some(sent.turn_id) => {}
                    Some(Event::Delta { text, .. }) => {
                        for op in renderer.push(&text) {
                            printer.op(self.out, op)?;
                        }
                    }
                    Some(Event::ToolApproval { call, reason, preview, .. }) => {
                        self.end_text(&mut renderer, &mut printer)?;
                        let answer = self.approval(&call, &reason, preview.as_deref(), input).await?;
                        let decision = answer.unwrap_or(Decision::Deny);
                        let approved: Result<Value> = self
                            .call(
                                method::TOOL_APPROVE,
                                ApproveParams {
                                    session_id: session_id.clone(),
                                    call_id: call.id.clone(),
                                    decision,
                                },
                            )
                            .await;
                        if let Err(err) = approved {
                            self.problem(&err)?;
                        }
                        if answer.is_none() {
                            self.cancel(&session_id).await;
                        }
                    }
                    Some(Event::ToolStarted { call, .. }) => {
                        self.end_text(&mut renderer, &mut printer)?;
                        let line = self.styled("2", &tool_display::started(&call));
                        write!(self.out, "{line}")?;
                        self.out.flush()?;
                        running = Some(call);
                    }
                    Some(Event::ToolFinished { output, is_error, .. }) => {
                        let mark = tool_display::finished(&output, is_error);
                        let mark = self.styled(if is_error { "31" } else { "32" }, &mark);
                        self.say(&format!("  {mark}"))?;
                        if let Some(call) = running.take() {
                            self.last_tool = Some((call, output, is_error));
                        }
                    }
                    Some(Event::TurnFinished { outcome, error, usage, .. }) => {
                        self.end_text(&mut renderer, &mut printer)?;
                        if running.take().is_some() {
                            self.say("")?;
                        }
                        if let Some(usage) = usage.filter(|u| u.steps > 1) {
                            self.note(&tool_display::usage(&usage))?;
                        }
                        break (outcome, error);
                    }
                    Some(_) => {}
                },
                Some(()) = interrupts.recv() => self.cancel(&session_id).await,
            }
        };
        match outcome {
            TurnOutcome::Complete => {}
            TurnOutcome::Cancelled => self.note("(stopped)")?,
            TurnOutcome::Failed => self.error(error.as_deref().unwrap_or("the reply failed"))?,
            TurnOutcome::Capped => self.note(&format!(
                "({}; say \"continue\" to keep going with a fresh budget)",
                error.as_deref().unwrap_or("stopped at a cap")
            ))?,
        }
        self.say("")?;
        Ok(())
    }

    async fn cancel(&self, session_id: &str) {
        let _: Result<CancelResult, _> = self
            .client
            .call(
                method::CHAT_CANCEL,
                SessionParams {
                    session_id: session_id.to_owned(),
                },
            )
            .await;
    }

    /// Finish the streamed text so far, so tool lines start on a fresh line.
    fn end_text(&mut self, renderer: &mut Renderer, printer: &mut Printer) -> Result<()> {
        for op in renderer.finish() {
            printer.op(self.out, op)?;
        }
        *renderer = Renderer::new(self.color);
        Ok(())
    }

    fn describe_mode(&mut self, settings: &SessionSettings) -> Result<()> {
        let text = match settings.mode {
            Mode::Assist => "Mode: assist. Chat with tools; anything risky asks first.".to_owned(),
            Mode::Agent => format!(
                "Mode: agent, level {}. Lucy works through tasks on her own, within her step and \
                 cost limits (/level to change, /undo to roll back file changes).",
                level_name(settings.level)
            ),
        };
        self.note(&text)?;
        Ok(())
    }

    /// Show the files the last reply with changes touched; restore them if the user agrees.
    async fn undo<S: LineSource>(&mut self, input: &mut S) -> Result<()> {
        let Some(session_id) = self.session.clone() else {
            self.note("Nothing to undo yet.")?;
            return Ok(());
        };
        let preview: UndoResult = self
            .call(
                method::SESSION_UNDO,
                UndoParams {
                    session_id: session_id.clone(),
                    apply: false,
                },
            )
            .await?;
        if preview.files.is_empty() {
            self.note("Nothing to undo: no file changes recorded in this chat.")?;
            return Ok(());
        }
        self.say("The last reply that changed files:")?;
        for file in &preview.files {
            let action = if file.created { "delete" } else { "restore" };
            let line = format!("  {action:7}  {}", file.path);
            let line = self.styled(if file.created { "31" } else { "33" }, &line);
            self.say(&line)?;
        }
        let answer = match input.read_line("Undo these changes? [y/N] › ").await {
            ReadLine::Text(answer) => answer,
            _ => String::new(),
        };
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            self.note("Left as they are.")?;
            return Ok(());
        }
        let done: UndoResult = self
            .call(
                method::SESSION_UNDO,
                UndoParams {
                    session_id,
                    apply: true,
                },
            )
            .await?;
        self.note(&format!(
            "Undone ({}). /undo again to go further back.",
            match done.files.len() {
                1 => "1 file".to_owned(),
                n => format!("{n} files"),
            }
        ))?;
        Ok(())
    }

    /// Show what a tool wants to do and ask. `None` means Ctrl+C: decline and stop the reply.
    async fn approval<S: LineSource>(
        &mut self,
        call: &ToolCall,
        reason: &str,
        preview: Option<&str>,
        input: &mut S,
    ) -> Result<Option<Decision>> {
        for line in tool_display::approval(call, reason, preview, self.color) {
            self.say(&line)?;
        }
        loop {
            match input.read_line(tool_display::APPROVAL_PROMPT).await {
                ReadLine::Text(answer) => match tool_display::parse_decision(&answer) {
                    Some(decision) => return Ok(Some(decision)),
                    None => self.note("Type y (yes), a (always this chat) or n (no).")?,
                },
                ReadLine::Interrupted => return Ok(None),
                ReadLine::Eof => return Ok(Some(Decision::Deny)),
            }
        }
    }

    async fn command<S: LineSource>(&mut self, command: Command, input: &mut S) -> Result<()> {
        match command {
            Command::Help => {
                let width = COMMANDS
                    .iter()
                    .map(|(name, _)| name.len())
                    .max()
                    .unwrap_or(0);
                for (name, help) in COMMANDS {
                    let name = self.styled("36", &format!("{name:width$}"));
                    self.say(&format!("  {name}  {help}"))?;
                }
            }
            Command::Quit => {}
            Command::New => {
                self.session = None;
                self.note("New chat.")?;
            }
            Command::Sessions => {
                let sessions: Vec<SessionSummary> =
                    self.call(method::SESSIONS_LIST, json!({})).await?;
                if sessions.is_empty() {
                    self.note("No saved chats yet.")?;
                }
                for s in sessions {
                    let current = self.session.as_deref() == Some(s.id.as_str());
                    let marker = if current { "›" } else { " " };
                    let id = self.styled("36", &s.id[..10]);
                    let title = s.title.as_deref().unwrap_or("(untitled)");
                    let count = self.styled("2", &format!("({} messages)", s.message_count));
                    self.say(&format!("{marker} {id}  {title}  {count}"))?;
                }
            }
            Command::Resume(prefix) => self.resume(&prefix).await?,
            Command::Rename(title) => {
                let Some(id) = self.session.clone() else {
                    bail!("nothing to rename yet; say something first");
                };
                let _: Value = self
                    .call(
                        method::SESSIONS_RENAME,
                        RenameParams {
                            session_id: id,
                            title: title.clone(),
                        },
                    )
                    .await?;
                self.note(&format!("Renamed to “{title}”."))?;
            }
            Command::Delete(prefix) => {
                let id: String = self
                    .call(method::SESSIONS_RESOLVE, ResolveParams { prefix })
                    .await?;
                let _: Value = self
                    .call(
                        method::SESSIONS_DELETE,
                        SessionParams {
                            session_id: id.clone(),
                        },
                    )
                    .await?;
                if self.session.as_deref() == Some(id.as_str()) {
                    self.session = None;
                    self.note("Deleted; starting a new chat.")?;
                } else {
                    self.note("Deleted.")?;
                }
            }
            Command::Model(None) => {
                let profiles: Vec<ProfileInfo> =
                    self.call(method::PROFILES_LIST, json!({})).await?;
                let current = self.settings().await?.map(|s| s.profile);
                for p in profiles {
                    let active = current.as_deref().map_or(p.is_default, |c| c == p.name);
                    let marker = if active { "›" } else { " " };
                    let default = if p.is_default { "  (default)" } else { "" };
                    let name = self.styled("36", &p.name);
                    let model = self.styled("2", &format!("{}{default}", p.model));
                    self.say(&format!("{marker} {name}  {model}"))?;
                }
            }
            Command::Model(Some(profile)) => {
                let session_id = self.ensure_session().await?;
                let settings: SessionSettings = self
                    .call(
                        method::SESSION_SET_PROFILE,
                        SetProfileParams {
                            session_id,
                            profile,
                        },
                    )
                    .await?;
                let profiles: Vec<ProfileInfo> =
                    self.call(method::PROFILES_LIST, json!({})).await?;
                let model = profiles
                    .iter()
                    .find(|p| p.name == settings.profile)
                    .map_or("?", |p| p.model.as_str());
                self.note(&format!(
                    "Switched this chat to {} ({model}).",
                    settings.profile
                ))?;
            }
            Command::Plain(choice) => {
                let session_id = self.ensure_session().await?;
                let current = self.settings().await?.is_some_and(|s| s.plain);
                let plain = choice.unwrap_or(!current);
                let _: SessionSettings = self
                    .call(
                        method::SESSION_SET_PLAIN,
                        SetPlainParams { session_id, plain },
                    )
                    .await?;
                self.note(if plain {
                    "Plain mode on."
                } else {
                    "Plain mode off."
                })?;
            }
            Command::Mode(None) => {
                let session_id = self.ensure_session().await?;
                let settings: SessionSettings = self
                    .call(method::SESSION_SETTINGS, SessionParams { session_id })
                    .await?;
                self.describe_mode(&settings)?;
            }
            Command::Mode(Some(mode)) => {
                let session_id = self.ensure_session().await?;
                let settings: SessionSettings = self
                    .call(method::SESSION_SET_MODE, SetModeParams { session_id, mode })
                    .await?;
                self.describe_mode(&settings)?;
            }
            Command::Level(None) => {
                let session_id = self.ensure_session().await?;
                let settings: SessionSettings = self
                    .call(method::SESSION_SETTINGS, SessionParams { session_id })
                    .await?;
                self.note(&format!("Agent level: {}.", level_name(settings.level)))?;
            }
            Command::Level(Some(level)) => {
                let session_id = self.ensure_session().await?;
                let settings: SessionSettings = self
                    .call(
                        method::SESSION_SET_LEVEL,
                        SetLevelParams { session_id, level },
                    )
                    .await?;
                self.describe_mode(&settings)?;
            }
            Command::Undo => self.undo(input).await?,
            Command::Tools => {
                let list: ToolsList = self
                    .call(
                        method::TOOLS_LIST,
                        ToolsListParams {
                            session_id: self.session.clone(),
                        },
                    )
                    .await?;
                let tools = list.tools;
                if tools.is_empty() && list.problems.is_empty() {
                    self.note("No tools: the built-ins are off and no MCP servers are set up.")?;
                }
                for problem in &list.problems {
                    let line = format!("  {}: {}", problem.server, problem.error);
                    self.error(&line)?;
                }
                let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
                for tool in &tools {
                    let (rule, color) = match tool.rule {
                        ToolRuleInfo::Auto => ("runs", "32"),
                        ToolRuleInfo::AutoInHome => ("runs*", "32"),
                        ToolRuleInfo::AutoInFolder => ("runs†", "32"),
                        ToolRuleInfo::Ask => ("asks", "33"),
                        ToolRuleInfo::Deny => ("blocked", "31"),
                    };
                    let name = self.styled("36", &format!("{:width$}", tool.name));
                    let rule = self.styled(color, &format!("{rule:7}"));
                    let description = self.styled("2", &tool.description);
                    self.say(&format!("  {name}  {rule}  {description}"))?;
                }
                if tools.iter().any(|t| t.rule == ToolRuleInfo::AutoInHome) {
                    self.note("  * runs by itself in your home folder; asks for private files and anywhere else")?;
                }
                if tools.iter().any(|t| t.rule == ToolRuleInfo::AutoInFolder) {
                    self.note("  † changes files in the working folder (the workspace at level 2) by itself; asks elsewhere")?;
                }
            }
            Command::Last => match self.last_tool.clone() {
                None => self.note("No tool calls yet in this chat.")?,
                Some((call, output, is_error)) => {
                    let title = self.styled("1", &format!("{} ({})", call.name, call.id));
                    self.say(&title)?;
                    let args = serde_json::to_string_pretty(&call.arguments)?;
                    self.say(&args)?;
                    self.note(if is_error { "failed:" } else { "output:" })?;
                    self.say(output.trim_end())?;
                }
            },
        }
        Ok(())
    }
}

fn level_name(level: AgentLevel) -> &'static str {
    match level {
        AgentLevel::Supervised => "0 (supervised: every tool call asks)",
        AgentLevel::Guarded => "1 (guarded: changes inside the working folder run by themselves)",
        AgentLevel::Workspace => "2 (workspace: anything local inside the workspace runs)",
        AgentLevel::Unrestricted => "3 (unrestricted: everything runs except private files)",
    }
}

/// The turn an event belongs to.
fn turn_of(event: &Event) -> Option<u64> {
    match event {
        Event::UserMessage { .. } => None,
        Event::TurnStarted { turn_id, .. }
        | Event::Delta { turn_id, .. }
        | Event::ToolApproval { turn_id, .. }
        | Event::ToolStarted { turn_id, .. }
        | Event::ToolFinished { turn_id, .. }
        | Event::TurnFinished { turn_id, .. } => Some(*turn_id),
    }
}

/// Writes renderer output, replacing a raw partial line with its styled form.
struct Printer {
    color: bool,
    width: usize,
    /// Raw text shown for the current, unfinished line.
    raw: String,
}

impl Printer {
    fn new(color: bool, width: usize) -> Self {
        Self {
            color,
            width,
            raw: String::new(),
        }
    }

    fn op(&mut self, out: &mut impl Write, op: Op) -> std::io::Result<()> {
        match op {
            Op::Raw(text) => {
                write!(out, "{text}")?;
                self.raw.push_str(&text);
            }
            Op::Line(line) if self.color => {
                if !self.raw.is_empty() {
                    // Back to the start of the (possibly wrapped) raw text, then clear it.
                    let rows_up = self.raw.chars().count().saturating_sub(1) / self.width;
                    write!(out, "\r")?;
                    if rows_up > 0 {
                        write!(out, "\x1b[{rows_up}A")?;
                    }
                    write!(out, "\x1b[J")?;
                }
                writeln!(out, "{line}")?;
                self.raw.clear();
            }
            Op::Line(line) => {
                // Plain output: the line equals its raw text, so finish what's missing.
                let rest = line.strip_prefix(self.raw.as_str()).unwrap_or(&line);
                writeln!(out, "{rest}")?;
                self.raw.clear();
            }
        }
        out.flush()
    }
}
