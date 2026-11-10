//! Pictures in the terminal, drawn with kitty's graphics protocol (kitty, Ghostty and WezTerm
//! speak it). Elsewhere the chat shows where the picture was saved instead.

/// The protocol's largest payload per escape.
const CHUNK: usize = 4096;

/// Whether this terminal draws pictures, judging by `env` (`std::env::var` in practice).
pub fn supported(env: impl Fn(&str) -> Option<String>) -> bool {
    // tmux swallows the escapes unless it's set up to pass them through.
    if env("TMUX").is_some() {
        return false;
    }
    let term = env("TERM").unwrap_or_default();
    let program = env("TERM_PROGRAM").unwrap_or_default();
    matches!(term.as_str(), "xterm-kitty" | "xterm-ghostty")
        || matches!(program.as_str(), "ghostty" | "WezTerm")
        || env("KITTY_WINDOW_ID").is_some()
}

/// The escapes that draw a PNG (`data`, base64) `columns` wide, keeping its shape, then move
/// to the next line.
pub fn draw(data: &str, columns: u16) -> String {
    let chunks: Vec<&str> = data
        .as_bytes()
        .chunks(CHUNK)
        // Base64 is ASCII, so any byte boundary is a character boundary.
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    let mut out = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = i + 1 < chunks.len();
        let keys = match (i, chunks.len()) {
            (0, 1) => format!("f=100,a=T,c={columns}"),
            (0, _) => format!("f=100,a=T,c={columns},m=1"),
            _ => format!("m={}", u8::from(more)),
        };
        out += &format!("\x1b_G{keys};{chunk}\x1b\\");
    }
    out.push('\n');
    out
}
