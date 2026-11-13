//! Pictures in the terminal: drawn inline where the terminal speaks kitty's graphics protocol
//! (kitty, Ghostty, WezTerm), otherwise left as the saved path.

use lucy_cli::inline_pictures::{draw, supported};

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

#[test]
fn ghostty_kitty_and_wezterm_show_pictures() {
    assert!(supported(env(&[("TERM", "xterm-ghostty")])));
    assert!(supported(env(&[("TERM_PROGRAM", "ghostty")])));
    assert!(supported(env(&[("TERM", "xterm-kitty")])));
    assert!(supported(env(&[("KITTY_WINDOW_ID", "1")])));
    assert!(supported(env(&[("TERM_PROGRAM", "WezTerm")])));
}

#[test]
fn other_terminals_and_tmux_dont() {
    assert!(!supported(env(&[("TERM", "xterm-256color")])));
    assert!(!supported(env(&[])));
    // tmux swallows the escape codes unless set up to pass them through.
    assert!(!supported(env(&[
        ("TERM", "xterm-ghostty"),
        ("TMUX", "/tmp/tmux-1000/default,1,0")
    ])));
}

#[test]
fn a_small_png_is_one_escape_sized_in_columns() {
    let drawn = draw("QUJD", 40).unwrap();

    assert_eq!(drawn, "\x1b_Gf=100,a=T,c=40;QUJD\x1b\\\n");
}

#[test]
fn a_big_png_is_sent_in_4096_byte_chunks() {
    let data = "A".repeat(4096 * 2 + 10);

    let drawn = draw(&data, 30).unwrap();

    let chunks: Vec<&str> = drawn
        .trim_end()
        .split("\x1b\\")
        .filter(|c| !c.is_empty())
        .collect();
    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].starts_with("\x1b_Gf=100,a=T,c=30,m=1;"));
    assert_eq!(chunks[0].split_once(';').unwrap().1.len(), 4096);
    assert_eq!(chunks[1], format!("\x1b_Gm=1;{}", "A".repeat(4096)));
    assert_eq!(chunks[2], format!("\x1b_Gm=0;{}", "A".repeat(10)));
}

#[test]
fn data_that_isnt_base64_draws_nothing() {
    // It could end the escape early and smuggle in others.
    assert_eq!(draw("QUJD\x1b\\\x1b]52;c;ZXZpbA==\x07", 40), None);
    assert_eq!(draw("", 40), None);
}
