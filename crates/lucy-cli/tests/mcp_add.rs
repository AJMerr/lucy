//! `lucy mcp add`: flags when given, prompts for whatever is missing.

use std::collections::{BTreeMap, VecDeque};

use anyhow::Result;
use lucy_cli::mcp::{AddArgs, complete_add};
use lucy_cli::wizard::Prompter;

/// Answers in order, recording each prompt; asking more than scripted is a test failure.
struct Scripted {
    answers: VecDeque<&'static str>,
    asked: Vec<String>,
}

impl Scripted {
    fn new(answers: &[&'static str]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            asked: Vec::new(),
        }
    }
}

impl Prompter for Scripted {
    fn select(&mut self, prompt: &str, _items: &[&str], _default: usize) -> Result<usize> {
        panic!("unexpected choice: {prompt}")
    }

    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String> {
        self.asked.push(prompt.to_owned());
        let answer = self
            .answers
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected question: {prompt}"));
        Ok(match (answer, default) {
            ("", Some(default)) => default.to_owned(),
            (answer, _) => answer.to_owned(),
        })
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn everything_on_the_command_line_asks_nothing() {
    let mut prompter = Scripted::new(&[]);

    let (name, server) = complete_add(
        AddArgs {
            name: Some("github".into()),
            command: strings(&["npx", "-y", "@modelcontextprotocol/server-github"]),
            keys: strings(&["GITHUB_TOKEN=github"]),
            env: strings(&["GITHUB_HOST=github.com"]),
            cwd: None,
        },
        &mut prompter,
    )
    .unwrap();

    assert_eq!(name, "github");
    assert_eq!(server.command, "npx");
    assert_eq!(server.args, ["-y", "@modelcontextprotocol/server-github"]);
    assert_eq!(server.env_keys, map(&[("GITHUB_TOKEN", "github")]));
    assert_eq!(server.env, map(&[("GITHUB_HOST", "github.com")]));
    assert!(server.enabled);
    assert!(prompter.asked.is_empty(), "asked: {:?}", prompter.asked);
}

#[test]
fn with_nothing_given_it_asks_for_name_command_and_keys() {
    let mut prompter = Scripted::new(&[
        "notes",
        r#"uvx "mcp server notes" --root ~/notes"#,
        "NOTES_TOKEN=notes",
        "OTHER_SECRET=other",
        "",
    ]);

    let (name, server) = complete_add(AddArgs::default(), &mut prompter).unwrap();

    assert_eq!(name, "notes");
    assert_eq!(server.command, "uvx");
    // Quoted words stay together, as in a shell.
    assert_eq!(server.args, ["mcp server notes", "--root", "~/notes"]);
    assert_eq!(
        server.env_keys,
        map(&[("NOTES_TOKEN", "notes"), ("OTHER_SECRET", "other")])
    );
    assert_eq!(prompter.asked.len(), 5);
}

#[test]
fn with_only_a_name_it_asks_for_the_command_and_keys() {
    let mut prompter = Scripted::new(&["mytool --stdio", ""]);

    let (name, server) = complete_add(
        AddArgs {
            name: Some("mytool".into()),
            ..AddArgs::default()
        },
        &mut prompter,
    )
    .unwrap();

    assert_eq!(name, "mytool");
    assert_eq!(server.command, "mytool");
    assert_eq!(server.args, ["--stdio"]);
    assert!(server.env_keys.is_empty());
    assert_eq!(prompter.asked.len(), 2, "asked: {:?}", prompter.asked);
    assert!(
        prompter.asked[0].to_lowercase().contains("command"),
        "{:?}",
        prompter.asked
    );
}

#[test]
fn a_bad_name_is_refused_with_the_rules() {
    let mut prompter = Scripted::new(&["My GitHub"]);

    let err = complete_add(AddArgs::default(), &mut prompter)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("My GitHub") && err.contains("lowercase"),
        "{err}"
    );
}

#[test]
fn a_key_without_a_keyring_name_is_refused() {
    let mut prompter = Scripted::new(&[]);

    let err = complete_add(
        AddArgs {
            name: Some("github".into()),
            command: strings(&["npx", "server-github"]),
            keys: strings(&["GITHUB_TOKEN"]),
            ..AddArgs::default()
        },
        &mut prompter,
    )
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("GITHUB_TOKEN") && err.contains("VAR=keyname"),
        "{err}"
    );
}

#[test]
fn an_empty_command_is_refused() {
    let mut prompter = Scripted::new(&["github", "   "]);

    let err = complete_add(AddArgs::default(), &mut prompter)
        .unwrap_err()
        .to_string();

    assert!(err.to_lowercase().contains("command"), "{err}");
}
