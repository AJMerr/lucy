//! `lucy ask`: one question, the answer streamed to stdout as plain text. Tool activity and
//! approval prompts go to the terminal (stderr), so stdout stays clean for scripts.

use std::io::{BufRead, IsTerminal, Read, Write};

use anyhow::{Result, bail};
use lucy_protocol::ToolCall;
use lucy_protocol::rpc::{
    ApproveParams, CancelResult, ChatSendParams, ChatSendResult, Decision, Event, SessionParams,
    TurnOutcome, method,
};
use serde_json::Value;

use crate::core_process;
use crate::tool_display;

pub fn run(prompt: Option<String>, yes: bool) -> Result<()> {
    let text = match prompt {
        Some(prompt) => prompt,
        None if !std::io::stdin().is_terminal() => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            input
        }
        None => bail!("nothing to ask: pass a prompt or pipe one in"),
    };
    if text.trim().is_empty() {
        bail!("nothing to ask: the prompt is empty");
    }
    tokio::runtime::Runtime::new()?.block_on(ask(text, yes))
}

async fn ask(text: String, yes: bool) -> Result<()> {
    let mut client = core_process::connect().await?;
    let mut events = client.take_events().expect("fresh client has its events");
    let sent: ChatSendResult = client
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                session_id: None,
                text,
                cwd: std::env::current_dir()
                    .ok()
                    .map(|dir| dir.display().to_string()),
            },
        )
        .await?;

    let mut out = std::io::stdout();
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
                    write!(out, "{text}")?;
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
                Some(Event::TurnFinished { turn_id, outcome, error, .. })
                    if turn_id == sent.turn_id =>
                {
                    writeln!(out)?;
                    return match outcome {
                        TurnOutcome::Complete => Ok(()),
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
