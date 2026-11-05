//! `lucy ask`: one question, the answer streamed to stdout as plain text.

use std::io::{IsTerminal, Read, Write};

use anyhow::{Result, bail};
use lucy_protocol::rpc::{
    CancelResult, ChatSendParams, ChatSendResult, Event, SessionParams, TurnOutcome, method,
};

use crate::core_process;

pub fn run(prompt: Option<String>) -> Result<()> {
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
    tokio::runtime::Runtime::new()?.block_on(ask(text))
}

async fn ask(text: String) -> Result<()> {
    let mut client = core_process::connect().await?;
    let mut events = client.take_events().expect("fresh client has its events");
    let sent: ChatSendResult = client
        .call(
            method::CHAT_SEND,
            ChatSendParams {
                session_id: None,
                text,
            },
        )
        .await?;

    let mut out = std::io::stdout();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                None => bail!("lost connection to the Lucy core"),
                Some(Event::Delta { turn_id, text, .. }) if turn_id == sent.turn_id => {
                    write!(out, "{text}")?;
                    out.flush()?;
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
