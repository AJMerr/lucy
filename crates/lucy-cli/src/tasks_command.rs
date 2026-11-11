//! `lucy tasks`: reminders and scheduled tasks, listed, paused, resumed or deleted. You make
//! them by asking her ("remind me at 5 to call Mom").

use std::io::Write;

use anyhow::{Result, bail};
use chrono::{Local, TimeZone};
use lucy_client::Client;
use lucy_protocol::rpc::{ScheduleId, ScheduleInfo, ScheduleUpdate, SchedulesList, method};
use serde_json::{Value, json};

use crate::core_process;

pub enum Action {
    List,
    Pause(String),
    Resume(String),
    Delete(String),
}

pub fn run(action: Action) -> Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let client = core_process::connect().await?;
        let out = &mut std::io::stdout();
        match action {
            Action::List => list(&client, out).await,
            Action::Pause(which) => set_paused(&client, &which, true, out).await,
            Action::Resume(which) => set_paused(&client, &which, false, out).await,
            Action::Delete(which) => delete(&client, &which, out).await,
        }
    })
}

pub async fn list(client: &Client, out: &mut impl Write) -> Result<()> {
    let all: SchedulesList = client.call(method::SCHEDULES_LIST, json!({})).await?;
    if all.schedules.is_empty() {
        writeln!(
            out,
            "Nothing is scheduled. Ask Lucy, e.g. \"remind me at 5 to call Mom\" or \"every \
             weekday at 8, summarise my email\"."
        )?;
        return Ok(());
    }
    for s in &all.schedules {
        let next = s
            .next_run_ms
            .and_then(|ms| Local.timestamp_millis_opt(ms).single())
            .map_or_else(String::new, |t| {
                format!(" · next {}", t.format("%a %-d %b %H:%M"))
            });
        let detail = match s.kind.as_str() {
            "task" => format!("task, {}, level {}", s.when, s.level),
            _ => format!("reminder, {}", s.when),
        };
        let paused = if s.paused { " · paused" } else { "" };
        writeln!(
            out,
            "{}  {}  ({detail}){next}{paused}",
            short_id(&s.id),
            s.title
        )?;
        writeln!(out, "      {}", s.what)?;
    }
    if !all.always_on {
        writeln!(
            out,
            "\nLucy's core doesn't start at login, so these only run while it's running. \
             `lucy service install` keeps it running."
        )?;
    }
    Ok(())
}

pub async fn set_paused(
    client: &Client,
    which: &str,
    paused: bool,
    out: &mut impl Write,
) -> Result<()> {
    let target = find(client, which).await?;
    let _: ScheduleInfo = client
        .call(
            method::SCHEDULES_UPDATE,
            ScheduleUpdate {
                id: target.id.clone(),
                paused: Some(paused),
                ..ScheduleUpdate::default()
            },
        )
        .await?;
    let verb = if paused { "Paused" } else { "Resumed" };
    writeln!(out, "{verb} “{}”.", target.title)?;
    Ok(())
}

pub async fn delete(client: &Client, which: &str, out: &mut impl Write) -> Result<()> {
    let target = find(client, which).await?;
    let _: Value = client
        .call(
            method::SCHEDULES_DELETE,
            ScheduleId {
                id: target.id.clone(),
            },
        )
        .await?;
    writeln!(out, "Deleted “{}”.", target.title)?;
    Ok(())
}

/// The schedule `which` names: an ID (or its start), or a title (or part of one).
async fn find(client: &Client, which: &str) -> Result<ScheduleInfo> {
    let all: SchedulesList = client.call(method::SCHEDULES_LIST, json!({})).await?;
    let wanted = which.trim().to_lowercase();
    let exact: Vec<&ScheduleInfo> = all
        .schedules
        .iter()
        .filter(|s| s.id.to_lowercase().starts_with(&wanted) || s.title.to_lowercase() == wanted)
        .collect();
    let candidates = if exact.is_empty() {
        all.schedules
            .iter()
            .filter(|s| s.title.to_lowercase().contains(&wanted))
            .collect()
    } else {
        exact
    };
    match candidates.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("nothing scheduled matches \"{which}\" (see `lucy tasks`)"),
        many => bail!(
            "\"{which}\" matches {}; use its ID",
            many.iter()
                .map(|s| format!("“{}” ({})", s.title, short_id(&s.id)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(10)]
}
