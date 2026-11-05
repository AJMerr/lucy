use lucy_cli::markdown::{Op, Renderer};

const DOC: &str = "\
## Jacking in

Use **`nix develop`** for a *reproducible* shell. See [the docs](https://nixos.org).

- first point
  - nested point
1. step one
> stay frosty

```rust
fn main() {}
```
---
snake_case_names and 2 * 3 * 4 stay as written.
";

/// Visible text without ANSI escapes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Feed `chunks` and collect only the finished lines.
fn lines(color: bool, chunks: &[&str]) -> Vec<String> {
    let mut renderer = Renderer::new(color);
    let mut ops: Vec<Op> = chunks.iter().flat_map(|c| renderer.push(c)).collect();
    ops.extend(renderer.finish());
    ops.into_iter()
        .filter_map(|op| match op {
            Op::Line(line) => Some(line),
            Op::Raw(_) => None,
        })
        .collect()
}

#[test]
fn partial_lines_stream_raw_then_finish_styled() {
    let mut r = Renderer::new(false);
    assert_eq!(r.push("Hel"), vec![Op::Raw("Hel".into())]);
    assert_eq!(
        r.push("lo\nWor"),
        vec![Op::Line("Hello".into()), Op::Raw("Wor".into())]
    );
    assert_eq!(r.push("ld"), vec![Op::Raw("ld".into())]);
    assert_eq!(r.finish(), vec![Op::Line("World".into())]);
    assert_eq!(r.finish(), vec![]);
}

#[test]
fn chunking_never_changes_the_result() {
    let whole = lines(true, &[DOC]);
    let by_char: Vec<String> = DOC.chars().map(String::from).collect();
    let by_char: Vec<&str> = by_char.iter().map(String::as_str).collect();
    assert_eq!(lines(true, &by_char), whole);
    assert_eq!(
        lines(true, &[&DOC[..37], &DOC[37..120], &DOC[120..]]),
        whole
    );
}

#[test]
fn without_color_lines_pass_through_unchanged() {
    let expected: Vec<String> = DOC.lines().map(String::from).collect();
    assert_eq!(lines(false, &[DOC]), expected);
}

#[test]
fn markdown_is_styled_and_its_markers_removed() {
    let visible: Vec<String> = lines(true, &[DOC]).iter().map(|l| strip_ansi(l)).collect();
    assert_eq!(
        visible,
        vec![
            "Jacking in",
            "",
            "Use nix develop for a reproducible shell. See the docs (https://nixos.org).",
            "",
            "• first point",
            "  • nested point",
            "1. step one",
            "│ stay frosty",
            "",
            "╭─ rust",
            "│ fn main() {}",
            "╰─",
            "────────────────────────────────────────",
            "snake_case_names and 2 * 3 * 4 stay as written.",
        ]
    );
}

#[test]
fn styling_uses_real_escapes_and_highlights_code() {
    let styled = lines(true, &[DOC]);
    assert!(
        styled[0].contains("\x1b[1"),
        "heading not bold: {:?}",
        styled[0]
    );
    let code = styled
        .iter()
        .find(|l| strip_ansi(l) == "│ fn main() {}")
        .unwrap();
    assert!(
        code.contains("\x1b[38;2;"),
        "code not highlighted: {code:?}"
    );
    assert!(
        styled
            .iter()
            .all(|l| l.ends_with("\x1b[0m") || !l.contains('\x1b')),
        "a styled line leaks its style into the next"
    );
}

#[test]
fn unclosed_code_block_still_renders_on_finish() {
    let visible: Vec<String> = lines(true, &["```\nlet x = 1;"])
        .iter()
        .map(|l| strip_ansi(l))
        .collect();
    assert_eq!(visible, vec!["╭─ code", "│ let x = 1;"]);
}
