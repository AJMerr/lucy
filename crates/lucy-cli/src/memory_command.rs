//! `lucy memory`: what Lucy remembers about you (memory.md), shown or edited in `$EDITOR`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use lucy_client::Client;
use lucy_protocol::rpc::{MemorySetParams, MemoryView, method};
use serde_json::json;

use crate::core_process;
use crate::safe_text::visible;

pub fn run(edit_it: bool) -> Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let client = core_process::connect().await?;
        if edit_it {
            edit(&client, open_editor).await
        } else {
            show(&client, &mut std::io::stdout()).await
        }
    })
}

/// Print her memory, and where it lives.
pub async fn show(client: &Client, out: &mut impl Write) -> Result<()> {
    let memory: MemoryView = client.call(method::MEMORY_GET, json!({})).await?;
    if memory.facts.is_empty() && memory.text.trim().is_empty() {
        writeln!(
            out,
            "Lucy remembers nothing yet. She saves facts as you chat (`lucy memory edit` to add \
             your own).\n({})",
            memory.path
        )?;
        return Ok(());
    }
    writeln!(out, "{}", visible(memory.text.trim_end()))?;
    writeln!(out, "\n({}; `lucy memory edit` to change it)", memory.path)?;
    Ok(())
}

/// Let `editor` change a copy of her memory, then save it if it changed.
pub async fn edit(client: &Client, editor: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    let memory: MemoryView = client.call(method::MEMORY_GET, json!({})).await?;
    // A copy, so this works with a core on another machine too.
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("memory.md");
    std::fs::write(&file, &memory.text)?;
    editor(&file)?;
    let text = std::fs::read_to_string(&file)?;
    if text == memory.text {
        println!("No changes.");
        return Ok(());
    }
    let _: MemoryView = client
        .call(method::MEMORY_SET, MemorySetParams { text })
        .await?;
    println!("Saved.");
    Ok(())
}

/// Open `file` in `$VISUAL` or `$EDITOR` (vi if neither is set) and wait for it.
fn open_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });
    let mut words = editor.split_whitespace();
    let program = words.next().context("$EDITOR is empty")?;
    let status = std::process::Command::new(program)
        .args(words)
        .arg(file)
        .status()
        .with_context(|| format!("couldn't start {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}; nothing saved");
    }
    Ok(())
}
