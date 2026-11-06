//! The real terminal: line editing, history, completion, and launching the chat.

use std::future::Future;
use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Result;
use lucy_protocol::rpc::{ProfileInfo, method};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Cmd, Context, Editor, KeyCode, KeyEvent, Modifiers};
use serde_json::json;
use tokio::sync::mpsc;

use crate::chat::{ChatOptions, LineSource, ReadLine, Start, run_chat};
use crate::commands::COMMANDS;
use crate::core_process;

/// Run the interactive chat in this terminal.
pub fn run(start: Start) -> Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let mut client = core_process::connect().await?;
        let profiles: Vec<ProfileInfo> = client.call(method::PROFILES_LIST, json!({})).await?;
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let history = lucy_core::sessions::default_path()
            .and_then(|db| db.parent().map(|dir| dir.join("history.txt")));
        let mut input = Terminal::new(
            profiles.into_iter().map(|p| p.name).collect(),
            history,
            color,
        )?;

        // Ctrl+C while a reply streams arrives as a signal (the line editor isn't active then).
        let (interrupt_tx, mut interrupts) = mpsc::channel(4);
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if interrupt_tx.send(()).await.is_err() {
                    break;
                }
            }
        });

        let width = terminal_size::terminal_size().map_or(80, |(w, _)| w.0 as usize);
        let mut out = std::io::stdout();
        run_chat(
            &mut client,
            &mut input,
            &mut out,
            &mut interrupts,
            ChatOptions {
                color,
                width,
                start,
            },
        )
        .await
    })
}

struct Terminal {
    /// Taken while a blocking `readline` runs on another thread.
    editor: Option<Editor<LucyHelper, DefaultHistory>>,
    history: Option<PathBuf>,
    color: bool,
}

impl Terminal {
    fn new(profiles: Vec<String>, history: Option<PathBuf>, color: bool) -> Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(LucyHelper { profiles }));
        editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::ALT), Cmd::Newline);
        if let Some(path) = &history {
            let _ = editor.load_history(path);
        }
        Ok(Self {
            editor: Some(editor),
            history,
            color,
        })
    }
}

impl LineSource for Terminal {
    fn read_line(&mut self, prompt: &str) -> impl Future<Output = ReadLine> {
        let editor = self.editor.take();
        // Approval answers (y/a/n) aren't worth keeping in history.
        let remember = prompt == crate::chat::PROMPT;
        let raw = prompt.to_owned();
        let styled = if self.color {
            format!("\x1b[1;36m{prompt}\x1b[0m")
        } else {
            raw.clone()
        };
        async move {
            let Some(mut editor) = editor else {
                return ReadLine::Eof;
            };
            let (mut editor, result) = tokio::task::spawn_blocking(move || {
                let result = editor.readline(&(raw.as_str(), styled.as_str()));
                (editor, result)
            })
            .await
            .expect("line editor thread panicked");

            let line = match result {
                Ok(line) => {
                    if remember && !line.trim().is_empty() {
                        let _ = editor.add_history_entry(line.as_str());
                        if let Some(path) = &self.history {
                            let _ = editor.save_history(path);
                        }
                    }
                    // A trailing `\` continued the input onto a new line.
                    ReadLine::Text(line.replace("\\\n", "\n"))
                }
                Err(ReadlineError::Interrupted) => ReadLine::Interrupted,
                Err(_) => ReadLine::Eof,
            };
            self.editor = Some(editor);
            line
        }
    }
}

#[derive(rustyline::Helper, rustyline::Hinter, rustyline::Highlighter)]
struct LucyHelper {
    profiles: Vec<String>,
}

impl Completer for LucyHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before = &line[..pos];
        let pairs = |start: usize, options: Vec<String>| {
            let pairs = options
                .into_iter()
                .map(|o| Pair {
                    display: o.clone(),
                    replacement: o,
                })
                .collect();
            Ok((start, pairs))
        };
        if let Some(arg) = before.strip_prefix("/model ") {
            let matches = self
                .profiles
                .iter()
                .filter(|p| p.starts_with(arg))
                .cloned()
                .collect();
            return pairs(pos - arg.len(), matches);
        }
        if before.starts_with('/') && !before.contains(char::is_whitespace) {
            let matches = COMMANDS
                .iter()
                .map(|(name, _)| name.to_string())
                .filter(|name| name.starts_with(before))
                .collect();
            return pairs(0, matches);
        }
        pairs(pos, Vec::new())
    }
}

impl Validator for LucyHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        Ok(if ctx.input().ends_with('\\') {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Valid(None)
        })
    }
}
