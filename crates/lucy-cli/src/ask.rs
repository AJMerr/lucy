//! `lucy ask` and `lucy agent`: one question or task, the answer streamed to stdout as plain
//! text. Tool activity and approval prompts go to the terminal (stderr), so stdout stays clean
//! for scripts.

use std::io::{BufRead, IsTerminal, Read, Write};

use anyhow::{Result, bail};
use lucy_protocol::rpc::{
    ApproveParams, CancelResult, ChatSendParams, ChatSendResult, Decision, Event, SessionParams,
    SessionSettings, SetLevelParams, SetModeParams, TurnOutcome, method,
};
use lucy_protocol::{AgentLevel, Mode, ToolCall};
use serde_json::{Value, json};

use crate::core_process;
use crate::safe_text::visible;
use crate::tool_display;

#[derive(Debug, Clone, Copy, Default)]
pub struct AskOptions {
    /// Approve every tool call without asking.
    pub yes: bool,
    /// Run as an agent task (`lucy agent`), optionally at a given level.
    pub agent: bool,
    pub level: Option<AgentLevel>,
}

/// How a one-shot reply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Complete,
    /// An agent task hit its step or cost cap (exit code 2).
    Capped,
}

pub fn run(prompt: Option<String>, options: AskOptions) -> Result<Finish> {
    let text = match prompt {
        Some(prompt) => prompt,
        None if !std::io::stdin().is_terminal() => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            input
        }
        None => bail!("nothing to do: pass a prompt or pipe one in"),
    };
    if text.trim().is_empty() {
        bail!("nothing to do: the prompt is empty");
    }
    tokio::runtime::Runtime::new()?.block_on(ask(text, options))
}

async fn ask(text: String, options: AskOptions) -> Result<Finish> {
    let yes = options.yes;
    let mut client = core_process::connect().await?;
    let mut events = client.take_events().expect("fresh client has its events");
    // An agent task gets its own chat, switched to agent mode first.
    let session_id = if options.agent {
        let id: String = client.call(method::SESSIONS_CREATE, json!({})).await?;
        let _: SessionSettings = client
            .call(
                method::SESSION_SET_MODE,
                SetModeParams {
                    session_id: id.clone(),
                    mode: Mode::Agent,
                },
            )
            .await?;
        if let Some(level) = options.level {
            let _: SessionSettings = client
                .call(
                    method::SESSION_SET_LEVEL,
                    SetLevelParams {
                        session_id: id.clone(),
                        level,
                    },
                )
                .await?;
        }
        Some(id)
    } else {
        None
    };
    let sent: ChatSendResult = client
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                incognito: false,
                images: Vec::new(),
                session_id,
                text,
                cwd: std::env::current_dir()
                    .ok()
                    .map(|dir| dir.display().to_string()),
            },
        )
        .await?;

    let mut out = std::io::stdout();
    let on_terminal = out.is_terminal();
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let dim = |text: &str| {
        if color {
            format!("\x1b[2m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    // Whether the answer so far ends mid-line (tool lines then start on a new one).
    let mut mid_line = false;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                None => bail!("lost connection to the Lucy core"),
                Some(Event::Delta { turn_id, text, .. }) if turn_id == sent.turn_id => {
                    // On a terminal, her reply is made safe to print; piped, it is passed on as is.
                    if on_terminal {
                        write!(out, "{}", visible(&text))?;
                    } else {
                        write!(out, "{text}")?;
                    }
                    out.flush()?;
                    mid_line = !text.ends_with('\n');
                }
                Some(Event::ToolApproval { turn_id, call, reason, preview, .. })
                    if turn_id == sent.turn_id =>
                {
                    if std::mem::take(&mut mid_line) {
                        writeln!(out)?;
                    }
                    let decision = if yes {
                        Decision::Once
                    } else {
                        let lines = tool_display::approval(&call, &reason, preview.as_deref(), color);
                        let call = call.clone();
                        tokio::task::spawn_blocking(move || ask_on_terminal(&lines, &call))
                            .await??
                    };
                    let _: Value = client
                        .call(
                            method::TOOL_APPROVE,
                            ApproveParams {
                                session_id: sent.session_id.clone(),
                                call_id: call.id.clone(),
                                decision,
                            },
                        )
                        .await?;
                }
                Some(Event::ToolStarted { turn_id, call, .. }) if turn_id == sent.turn_id => {
                    if std::mem::take(&mut mid_line) {
                        writeln!(out)?;
                    }
                    eprint!("{}", dim(&tool_display::started(&call)));
                }
                Some(Event::ToolFinished { turn_id, output, is_error, .. })
                    if turn_id == sent.turn_id =>
                {
                    eprintln!("  {}", dim(&tool_display::finished(&output, is_error)));
                }
                Some(Event::TurnFinished { turn_id, outcome, error, usage, .. })
                    if turn_id == sent.turn_id =>
                {
                    writeln!(out)?;
                    if let Some(usage) = usage.filter(|u| options.agent || u.steps > 1) {
                        eprintln!("{}", dim(&tool_display::usage(&usage)));
                    }
                    return match outcome {
                        TurnOutcome::Complete => Ok(Finish::Complete),
                        TurnOutcome::Capped => {
                            eprintln!(
                                "lucy: {}; run it again or continue with `lucy -c`",
                                error.as_deref().unwrap_or("stopped at a cap")
                            );
                            Ok(Finish::Capped)
                        }
                        TurnOutcome::Cancelled => bail!("cancelled"),
                        TurnOutcome::Failed => bail!(error.unwrap_or_else(|| "reply failed".into())),
                    };
                }
                Some(_) => {}
            },
            _ = tokio::signal::ctrl_c() => {
                // Stop the reply; the core then sends TurnFinished(cancelled).
                let _: CancelResult = client
                    .call(method::CHAT_CANCEL, SessionParams { session_id: sent.session_id.clone() })
                    .await?;
            }
        }
    }
}

/// Ask on the controlling terminal, even when stdin is a pipe. Without a terminal (cron, CI),
/// the call is declined.
fn ask_on_terminal(lines: &[String], call: &ToolCall) -> Result<Decision> {
    let Some((mut reader, mut writer)) = open_terminal() else {
        eprintln!(
            "lucy: declined {} (no terminal to ask on; pass --yes to allow tools)",
            call.name
        );
        return Ok(Decision::Deny);
    };
    for line in lines {
        writeln!(writer, "{line}")?;
    }
    loop {
        write!(writer, "{}", tool_display::APPROVAL_PROMPT)?;
        writer.flush()?;
        let mut answer = String::new();
        if reader.read_line(&mut answer)? == 0 {
            return Ok(Decision::Deny);
        }
        match tool_display::parse_decision(&answer) {
            Some(decision) => return Ok(decision),
            None => writeln!(writer, "Type y (yes), a (always) or n (no).")?,
        }
    }
}

type Terminal = (std::io::BufReader<std::fs::File>, std::fs::File);

fn open_terminal() -> Option<Terminal> {
    let (input, output) = if cfg!(windows) {
        ("CONIN$", "CONOUT$")
    } else {
        ("/dev/tty", "/dev/tty")
    };
    let reader = std::fs::File::open(input).ok()?;
    let writer = std::fs::OpenOptions::new().write(true).open(output).ok()?;
    Some((std::io::BufReader::new(reader), writer))
}
