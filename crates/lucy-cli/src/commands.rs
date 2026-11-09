//! Parsing what the user types at the chat prompt.

use lucy_protocol::{AgentLevel, Mode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Nothing,
    Message(String),
    /// `/image`: make a picture from this description.
    Image(String),
    Command(Command),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Quit,
    New,
    Sessions,
    Resume(String),
    Rename(String),
    Delete(String),
    /// List profiles, or switch to one.
    Model(Option<String>),
    /// Toggle, or set explicitly.
    Plain(Option<bool>),
    /// Show, or switch.
    Mode(Option<Mode>),
    /// Show, or set, the agent level.
    Level(Option<AgentLevel>),
    /// Restore the files the last reply changed.
    Undo,
    /// List tools and whether each runs by itself or asks.
    Tools,
    /// Show the last tool call in full.
    Last,
}

/// Every command with its usage, for /help and Tab completion.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show this list"),
    ("/image", "<description>  make a picture straight away"),
    (
        "/model",
        "[profile]  list profiles, or switch this chat to one",
    ),
    (
        "/plain",
        "[on|off]  drop Lucy's voice (toggles without an argument)",
    ),
    ("/mode", "[assist|agent]  show or switch mode"),
    (
        "/level",
        "[0-3]  agent freedom: 0 supervised, 1 guarded, 2 workspace, 3 unrestricted",
    ),
    (
        "/undo",
        "restore the files Lucy's last reply changed (repeat to go further back)",
    ),
    ("/tools", "list Lucy's tools and which ones ask first"),
    ("/last", "show the last tool call's full input and output"),
    ("/new", "start a new chat"),
    ("/sessions", "list saved chats"),
    (
        "/resume",
        "<id>  continue a saved chat (any unique ID prefix)",
    ),
    ("/rename", "<title>  rename this chat"),
    ("/delete", "<id>  delete a saved chat"),
    ("/quit", "leave (Ctrl+D also works)"),
];

/// Classify one submitted input. `Err` carries a message for the user.
pub fn parse(line: &str) -> Result<Input, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Input::Nothing);
    }
    if let Some(literal) = line.strip_prefix("//") {
        return Ok(Input::Message(format!("/{literal}")));
    }
    let Some(rest) = line.strip_prefix('/') else {
        return Ok(Input::Message(line.to_owned()));
    };

    let (name, arg) = match rest.split_once(char::is_whitespace) {
        Some((name, arg)) => (name, Some(arg.trim()).filter(|a| !a.is_empty())),
        None => (rest, None),
    };
    let usage = |text: &str| Err(format!("usage: {text}"));
    match (name.to_ascii_lowercase().as_str(), arg) {
        ("image", Some(prompt)) => return Ok(Input::Image(prompt.to_owned())),
        ("image", None) => return usage("/image <description>"),
        _ => {}
    }
    let command = match (name.to_ascii_lowercase().as_str(), arg) {
        ("help", _) => Command::Help,
        ("quit" | "exit", _) => Command::Quit,
        ("new", _) => Command::New,
        ("sessions", _) => Command::Sessions,
        ("tools", _) => Command::Tools,
        ("last", _) => Command::Last,
        ("undo", _) => Command::Undo,
        ("level", None) => Command::Level(None),
        ("level", Some(arg)) => match arg
            .parse::<u8>()
            .ok()
            .and_then(|n| AgentLevel::try_from(n).ok())
        {
            Some(level) => Command::Level(Some(level)),
            None => return usage("/level [0-3]"),
        },
        ("resume", Some(id)) => Command::Resume(id.to_owned()),
        ("resume", None) => return usage("/resume <session id>"),
        ("delete", Some(id)) => Command::Delete(id.to_owned()),
        ("delete", None) => return usage("/delete <session id>"),
        ("rename", Some(title)) => Command::Rename(title.to_owned()),
        ("rename", None) => return usage("/rename <title>"),
        ("model", None) => Command::Model(None),
        ("model", Some(profile)) if !profile.contains(char::is_whitespace) => {
            Command::Model(Some(profile.to_owned()))
        }
        ("model", Some(_)) => return usage("/model [profile]"),
        ("plain", None) => Command::Plain(None),
        ("plain", Some(arg)) => match arg.to_ascii_lowercase().as_str() {
            "on" => Command::Plain(Some(true)),
            "off" => Command::Plain(Some(false)),
            _ => return usage("/plain [on|off]"),
        },
        ("mode", None) => Command::Mode(None),
        ("mode", Some(arg)) => match arg.to_ascii_lowercase().as_str() {
            "assist" => Command::Mode(Some(Mode::Assist)),
            "agent" => Command::Mode(Some(Mode::Agent)),
            _ => return usage("/mode [assist|agent]"),
        },
        (other, _) => return Err(format!("unknown command /{other}; try /help")),
    };
    Ok(Input::Command(command))
}
