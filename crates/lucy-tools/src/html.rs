//! Just enough HTML-to-text for a model to read a web page.

use std::sync::LazyLock;

use regex::Regex;

static HIDDEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(script|style|noscript|template|svg)\b.*?</(script|style|noscript|template|svg)\s*>|<!--.*?-->")
        .unwrap()
});
static BREAKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<(br|/?(p|div|section|article|header|footer|main|nav|aside|h[1-6]|li|ul|ol|tr|table|blockquote|pre|title)\b)[^>]*>")
        .unwrap()
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static ENTITIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[a-zA-Z]+);").unwrap());

pub fn to_text(html: &str) -> String {
    let text = HIDDEN.replace_all(html, "");
    let text = BREAKS.replace_all(&text, "\n");
    let text = TAGS.replace_all(&text, "");
    let text = ENTITIES.replace_all(&text, |caps: &regex::Captures| decode(&caps[1], &caps[0]));

    let mut out = String::new();
    let mut blank = true;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank {
                out.push('\n');
            }
            blank = true;
        } else {
            out.push_str(&line);
            out.push('\n');
            blank = false;
        }
    }
    out.trim().to_owned()
}

fn decode(entity: &str, original: &str) -> String {
    let code = match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
            .ok()
            .and_then(char::from_u32),
        e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
        _ => None,
    };
    code.map_or_else(|| original.to_owned(), String::from)
}
