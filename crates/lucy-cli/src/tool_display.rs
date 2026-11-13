//! How tool calls look in the terminal: one compact line each, and the approval prompt. Names,
//! arguments and output come from the model and the tools, so they're made safe to print.

use lucy_protocol::ToolCall;
use lucy_protocol::rpc::Decision;
use serde_json::Value;

use crate::safe_text::{exact, visible};

/// Argument names worth showing first, in order of preference.
const KEY_ARGS: &[&str] = &[
    "command", "path", "url", "pattern", "query", "name", "fact", "prompt",
];
const SUMMARY_CHARS: usize = 60;

pub const APPROVAL_PROMPT: &str = "Allow? [y]es · [a]lways this chat · [N]o › ";
/// For a call only the machine running Lucy may approve.
pub const LOCAL_ONLY_PROMPT: &str = "[Enter] wait for it there · [n] decline › ";

/// `⚙ read_file  src/main.rs`, without the outcome.
pub fn started(call: &ToolCall) -> String {
    let summary = summary(&call.arguments);
    let name = visible(&call.name);
    if summary.is_empty() {
        format!("⚙ {name}")
    } else {
        format!("⚙ {name}  {}", visible(&summary))
    }
}

/// `✓ 2.1 KB`, or `✗` and the first line of the error.
pub fn finished(output: &str, is_error: bool) -> String {
    if is_error {
        let first = output
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed");
        format!("✗ {}", visible(&shorten(first.trim(), 80)))
    } else {
        format!("✓ {}", size(output.len()))
    }
}

/// The most telling argument, shortened to one line.
pub fn summary(arguments: &Value) -> String {
    let Value::Object(args) = arguments else {
        return shorten(&arguments.to_string(), SUMMARY_CHARS);
    };
    let key = KEY_ARGS
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .or_else(|| args.values().find_map(Value::as_str));
    match key {
        Some(text) => shorten(text, SUMMARY_CHARS),
        None if args.is_empty() => String::new(),
        None => shorten(&arguments.to_string(), SUMMARY_CHARS),
    }
}

/// The approval request: what Lucy wants to do, why it asks, and any preview.
pub fn approval(call: &ToolCall, reason: &str, preview: Option<&str>, color: bool) -> Vec<String> {
    let paint = |codes: &str, text: &str| {
        if color {
            format!("\x1b[{codes}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let mut lines = vec![format!(
        "{} {}",
        paint(
            "1;33",
            &format!("Lucy wants to use {}", visible(&call.name))
        ),
        paint("2", &format!("({})", visible(reason)))
    )];
    let full_command = call.arguments.get("command").and_then(Value::as_str);
    match (full_command, preview) {
        // Exactly as it will run: a carriage return or a lookalike space can't hide anything.
        (Some(command), _) => lines.extend(exact(command).lines().map(|l| format!("  $ {l}"))),
        (None, Some(_)) => {}
        (None, None) => {
            let args = serde_json::to_string_pretty(&call.arguments).unwrap_or_default();
            lines.extend(exact(&args).lines().map(|l| format!("  {l}")));
        }
    }
    if let Some(preview) = preview {
        for line in visible(preview).lines() {
            let styled = match line.chars().next() {
                _ if line.starts_with("+++") || line.starts_with("---") => paint("1", line),
                Some('+') => paint("32", line),
                Some('-') => paint("31", line),
                Some('@') => paint("36", line),
                _ => line.to_owned(),
            };
            lines.push(format!("  {styled}"));
        }
    }
    lines
}

/// `y`, `a` or `n` (and their spellings out). Empty means no.
pub fn parse_decision(answer: &str) -> Option<Decision> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(Decision::Once),
        "a" | "always" => Some(Decision::Always),
        "" | "n" | "no" => Some(Decision::Deny),
        _ => None,
    }
}

fn shorten(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let more = line.len() < text.trim_end().len();
    if line.chars().count() <= max && !more {
        return line.to_owned();
    }
    let short: String = line.chars().take(max - 1).collect();
    format!("{short}…")
}

fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// `· 7 steps · 12.3k tokens · $0.02` for a reply that took several model steps.
pub fn usage(usage: &lucy_protocol::rpc::TurnUsage) -> String {
    let tokens = usage.input_tokens + usage.output_tokens;
    let tokens = if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    };
    let mut line = format!("· {} steps · {tokens} tokens", usage.steps);
    if let Some(cost) = usage.cost_usd.filter(|c| *c > 0.0) {
        line.push_str(&format!(" · ${cost:.2}"));
    }
    line
}
