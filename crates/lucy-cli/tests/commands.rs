use lucy_cli::commands::{Command, Input, parse};
use lucy_protocol::Mode;

fn command(line: &str) -> Command {
    match parse(line) {
        Ok(Input::Command(command)) => command,
        other => panic!("{line:?} parsed as {other:?}"),
    }
}

fn error(line: &str) -> String {
    parse(line).expect_err(line)
}

#[test]
fn plain_text_is_a_message() {
    assert_eq!(parse("hey Lucy"), Ok(Input::Message("hey Lucy".into())));
    assert_eq!(
        parse("  multi\nline  "),
        Ok(Input::Message("multi\nline".into()))
    );
}

#[test]
fn double_slash_sends_a_literal_slash() {
    assert_eq!(
        parse("//etc/hosts is weird"),
        Ok(Input::Message("/etc/hosts is weird".into()))
    );
}

#[test]
fn blank_input_is_nothing() {
    assert_eq!(parse("   "), Ok(Input::Nothing));
}

#[test]
fn commands_without_arguments() {
    assert_eq!(command("/help"), Command::Help);
    assert_eq!(command("/quit"), Command::Quit);
    assert_eq!(command("/exit"), Command::Quit);
    assert_eq!(command("/new"), Command::New);
    assert_eq!(command("/sessions"), Command::Sessions);
    assert_eq!(command("  /HELP  "), Command::Help);
}

#[test]
fn commands_with_arguments() {
    assert_eq!(command("/resume 01jc8"), Command::Resume("01jc8".into()));
    assert_eq!(command("/delete 01jc8"), Command::Delete("01jc8".into()));
    assert_eq!(
        command("/rename Netrunning 101"),
        Command::Rename("Netrunning 101".into())
    );
    assert_eq!(command("/model"), Command::Model(None));
    assert_eq!(
        command("/model claude"),
        Command::Model(Some("claude".into()))
    );
    assert_eq!(command("/plain"), Command::Plain(None));
    assert_eq!(command("/plain on"), Command::Plain(Some(true)));
    assert_eq!(command("/plain OFF"), Command::Plain(Some(false)));
    assert_eq!(command("/mode"), Command::Mode(None));
    assert_eq!(command("/mode agent"), Command::Mode(Some(Mode::Agent)));
    assert_eq!(command("/mode assist"), Command::Mode(Some(Mode::Assist)));
}

#[test]
fn bad_commands_explain_themselves() {
    assert!(error("/frobnicate").contains("unknown command /frobnicate"));
    assert!(error("/frobnicate").contains("/help"));
    assert!(error("/resume").contains("usage: /resume <session id>"));
    assert!(error("/delete").contains("usage: /delete <session id>"));
    assert!(error("/rename").contains("usage: /rename <title>"));
    assert!(error("/plain maybe").contains("usage: /plain [on|off]"));
    assert!(error("/mode chaos").contains("usage: /mode [assist|agent]"));
    assert!(error("/model a b").contains("usage: /model [profile]"));
}
