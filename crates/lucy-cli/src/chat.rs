//! The interactive chat loop: read input, run slash commands, stream replies.

use std::future::Future;
use std::io::Write;

use anyhow::{Context, Result, bail};
use lucy_client::{Client, ClientError};
use lucy_protocol::rpc::{
    CancelResult, ChatSendParams, ChatSendResult, Event, ProfileInfo, RenameParams, ResolveParams,
    Session, SessionParams, SessionSettings, SessionSummary, SetPlainParams, SetProfileParams,
    TurnOutcome, method,
};
use lucy_protocol::{Mode, Role};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::commands::{COMMANDS, Command, Input, parse};
use crate::markdown::{Op, Renderer};

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
            Ok(Input::Command(command)) => chat.command(command).await,
            Ok(Input::Message(text)) => chat.send(text, &mut events, interrupts).await,
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

        let skip = session.messages.len().saturating_sub(RECAP);
        for message in &session.messages[skip..] {
            match message.role {
                Role::User => {
                    let line = self.styled("36", &format!("{PROMPT}{}", message.content));
                    self.say(&line)?;
                }
                _ => {
                    let mut renderer = Renderer::new(self.color);
                    let mut ops = renderer.push(&message.content);
                    ops.extend(renderer.finish());
                    let mut printer = Printer::new(self.out, self.color, self.width);
                    for op in ops {
                        printer.op(op)?;
                    }
                    if message.incomplete {
                        self.note("(cut off)")?;
                    }
                    self.say("")?;
                }
            }
        }
        Ok(())
    }

    async fn send(
        &mut self,
        text: String,
        events: &mut mpsc::UnboundedReceiver<Event>,
        interrupts: &mut mpsc::Receiver<()>,
    ) -> Result<()> {
        // A Ctrl+C pressed between replies must not cancel this one.
        while interrupts.try_recv().is_ok() {}
        let sent: ChatSendResult = self
            .call(
                method::CHAT_SEND,
                ChatSendParams {
                    session_id: self.session.clone(),
                    text,
                },
            )
            .await?;
        self.session = Some(sent.session_id.clone());

        let mut renderer = Renderer::new(self.color);
        let mut printer = Printer::new(self.out, self.color, self.width);
        let (outcome, error) = loop {
            tokio::select! {
                event = events.recv() => match event {
                    None => bail!(ClientError::Disconnected),
                    Some(Event::Delta { turn_id, text, .. }) if turn_id == sent.turn_id => {
                        for op in renderer.push(&text) {
                            printer.op(op)?;
                        }
                    }
                    Some(Event::TurnFinished { turn_id, outcome, error, .. })
                        if turn_id == sent.turn_id =>
                    {
                        for op in renderer.finish() {
                            printer.op(op)?;
                        }
                        break (outcome, error);
                    }
                    Some(_) => {}
                },
                Some(()) = interrupts.recv() => {
                    // `self.client`, not `self.call`: the printer is borrowing `self.out`.
                    let _: Result<CancelResult, _> = self
                        .client
                        .call(method::CHAT_CANCEL, SessionParams { session_id: sent.session_id.clone() })
                        .await;
                }
            }
        };
        match outcome {
            TurnOutcome::Complete => {}
            TurnOutcome::Cancelled => self.note("(stopped)")?,
            TurnOutcome::Failed => self.error(error.as_deref().unwrap_or("the reply failed"))?,
        }
        self.say("")?;
        Ok(())
    }

    async fn command(&mut self, command: Command) -> Result<()> {
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
            Command::Mode(None) | Command::Mode(Some(Mode::Assist)) => {
                self.note("Mode: assist (chat; tools arrive in step 8).")?;
            }
            Command::Mode(Some(Mode::Agent)) => {
                self.note("Agent mode isn't built yet (milestone 1, step 9); staying in assist.")?;
            }
        }
        Ok(())
    }
}

/// Writes renderer output, replacing a raw partial line with its styled form.
struct Printer<'a, W: Write> {
    out: &'a mut W,
    color: bool,
    width: usize,
    /// Raw text shown for the current, unfinished line.
    raw: String,
}

impl<'a, W: Write> Printer<'a, W> {
    fn new(out: &'a mut W, color: bool, width: usize) -> Self {
        Self {
            out,
            color,
            width,
            raw: String::new(),
        }
    }

    fn op(&mut self, op: Op) -> std::io::Result<()> {
        match op {
            Op::Raw(text) => {
                write!(self.out, "{text}")?;
                self.raw.push_str(&text);
            }
            Op::Line(line) if self.color => {
                if !self.raw.is_empty() {
                    // Back to the start of the (possibly wrapped) raw text, then clear it.
                    let rows_up = self.raw.chars().count().saturating_sub(1) / self.width;
                    write!(self.out, "\r")?;
                    if rows_up > 0 {
                        write!(self.out, "\x1b[{rows_up}A")?;
                    }
                    write!(self.out, "\x1b[J")?;
                }
                writeln!(self.out, "{line}")?;
                self.raw.clear();
            }
            Op::Line(line) => {
                // Plain output: the line equals its raw text, so finish what's missing.
                let rest = line.strip_prefix(self.raw.as_str()).unwrap_or(&line);
                writeln!(self.out, "{rest}")?;
                self.raw.clear();
            }
        }
        self.out.flush()
    }
}
