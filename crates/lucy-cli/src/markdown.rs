//! Line-by-line Markdown rendering for streamed replies.
//!
//! Text arrives in arbitrary chunks. The line in progress is shown raw ([`Op::Raw`]); once it
//! ends it is replaced by its styled form ([`Op::Line`]). Code blocks are highlighted a line at
//! a time, so nothing is held back.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// More of the current, unfinished line, unstyled.
    Raw(String),
    /// A finished line, styled. Replaces whatever `Raw` text was shown for it.
    Line(String),
}

pub struct Renderer {
    color: bool,
    pending: String,
    /// Bytes of `pending` already emitted as `Raw`.
    shown: usize,
    code: Option<Option<HighlightLines<'static>>>,
}

const RESET: &str = "\x1b[0m";
const RULE_WIDTH: usize = 40;

impl Renderer {
    /// `color: false` passes text through untouched (pipes, `NO_COLOR`).
    pub fn new(color: bool) -> Self {
        Self {
            color,
            pending: String::new(),
            shown: 0,
            code: None,
        }
    }

    pub fn push(&mut self, chunk: &str) -> Vec<Op> {
        self.pending.push_str(chunk);
        let mut ops = Vec::new();
        while let Some(end) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=end).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            ops.push(Op::Line(self.render(line)));
            self.shown = 0;
        }
        if self.pending.len() > self.shown {
            ops.push(Op::Raw(self.pending[self.shown..].to_owned()));
            self.shown = self.pending.len();
        }
        ops
    }

    /// End of the reply: render any unfinished line and reset.
    pub fn finish(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            ops.push(Op::Line(self.render(&line)));
        }
        self.shown = 0;
        self.code = None;
        ops
    }

    fn render(&mut self, line: &str) -> String {
        if !self.color {
            return line.to_owned();
        }
        let mut styled = self.style_line(line);
        if styled.contains('\x1b') && !styled.ends_with(RESET) {
            styled.push_str(RESET);
        }
        styled
    }

    fn style_line(&mut self, line: &str) -> String {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if let Some(highlighter) = &mut self.code {
            if trimmed.starts_with("```") {
                self.code = None;
                return dim("╰─");
            }
            let code = match highlighter {
                Some(h) => highlight(h, line),
                None => line.to_owned(),
            };
            return format!("{}{code}", dim("│ "));
        }
        if let Some(lang) = trimmed.strip_prefix("```") {
            let lang = lang.trim();
            self.code = Some(highlighter_for(lang));
            return dim(&format!(
                "╭─ {}",
                if lang.is_empty() { "code" } else { lang }
            ));
        }
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            return paint("1;35", &inline(trimmed[hashes..].trim()));
        }
        if is_rule(trimmed) {
            return dim(&"─".repeat(RULE_WIDTH));
        }
        if let Some(quote) = trimmed.strip_prefix('>') {
            return format!("{}{}", dim("│ "), inline(quote.trim_start()));
        }
        for bullet in ["- ", "* ", "+ "] {
            if let Some(item) = trimmed.strip_prefix(bullet) {
                return format!("{indent}{} {}", paint("36", "•"), inline(item));
            }
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && trimmed[digits..].starts_with(". ") {
            let (number, item) = trimmed.split_at(digits + 1);
            return format!("{indent}{} {}", paint("36", number), inline(&item[1..]));
        }
        inline(line)
    }
}

fn is_rule(line: &str) -> bool {
    let marks: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| marks.chars().all(|c| c.to_string() == *m))
}

fn paint(codes: &str, text: &str) -> String {
    format!("\x1b[{codes}m{text}{RESET}")
}

fn dim(text: &str) -> String {
    paint("2", text)
}

fn assets() -> &'static (SyntaxSet, Theme) {
    static ASSETS: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .remove("base16-ocean.dark")
            .expect("bundled theme");
        (SyntaxSet::load_defaults_newlines(), theme)
    })
}

fn highlighter_for(lang: &str) -> Option<HighlightLines<'static>> {
    if lang.is_empty() {
        return None;
    }
    let (syntaxes, theme) = assets();
    syntaxes
        .find_syntax_by_token(lang)
        .map(|syntax| HighlightLines::new(syntax, theme))
}

fn highlight(highlighter: &mut HighlightLines<'static>, line: &str) -> String {
    let (syntaxes, _) = assets();
    let with_newline = format!("{line}\n");
    match highlighter.highlight_line(&with_newline, syntaxes) {
        Ok(ranges) => as_24_bit_terminal_escaped(&ranges, false).replace('\n', ""),
        Err(_) => line.to_owned(),
    }
}

/// Inline spans: `code`, **bold**, *italic*/_italic_, [links](url).
fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if chars[i] == '`'
            && let Some(len) = rest[1..].find('`')
        {
            let code = &rest[1..1 + len];
            out += &paint("36", code);
            i += code.chars().count() + 2;
            continue;
        }
        if rest.starts_with("**")
            && let Some(inner) = emphasis(&rest, "**", false)
        {
            out += &paint("1", &inline(inner));
            i += inner.chars().count() + 4;
            continue;
        }
        if chars[i] == '*' || (chars[i] == '_' && (i == 0 || !chars[i - 1].is_alphanumeric())) {
            let marker = if chars[i] == '*' { "*" } else { "_" };
            if let Some(inner) = emphasis(&rest, marker, marker == "_") {
                out += &paint("3", &inline(inner));
                i += inner.chars().count() + 2;
                continue;
            }
        }
        if chars[i] == '['
            && let Some((label, url, len)) = link(&rest)
        {
            out += &paint("4", label);
            out += &dim(&format!(" ({url})"));
            i += len;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Text between an opening and closing `marker` at the start of `text`, following CommonMark's
/// flanking rule: no space just inside the markers (so `2 * 3 * 4` stays literal). With
/// `word_bound`, the closer must not be followed by a letter (so `snake_case_names` stays too).
fn emphasis<'a>(text: &'a str, marker: &str, word_bound: bool) -> Option<&'a str> {
    let body = text.strip_prefix(marker)?;
    if body.starts_with(char::is_whitespace) {
        return None;
    }
    let mut from = 0;
    while let Some(pos) = body[from..].find(marker) {
        let end = from + pos;
        let after = &body[end + marker.len()..];
        let closes = end > 0
            && !body[..end].ends_with(char::is_whitespace)
            && !(word_bound && after.starts_with(char::is_alphanumeric));
        if closes {
            return Some(&body[..end]);
        }
        from = end + marker.len();
    }
    None
}

/// `[label](url)` at the start of `text`: label, url, and length in chars.
fn link(text: &str) -> Option<(&str, &str, usize)> {
    let close = text.find("](")?;
    let label = &text[1..close];
    let url_end = text[close + 2..].find(')')?;
    let url = &text[close + 2..close + 2 + url_end];
    let len = text[..close + 3 + url_end].chars().count();
    Some((label, url, len))
}
