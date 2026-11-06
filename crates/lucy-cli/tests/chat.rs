//! The interactive chat loop against a real core, with scripted input and fake model servers.

use std::collections::VecDeque;
use std::time::Duration;

use lucy_cli::chat::{ChatOptions, LineSource, ReadLine, Start, run_chat};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::json;
use tokio::sync::mpsc;

fn sse(parts: &[&str], finish: bool) -> FakeResponse {
    let mut body = String::new();
    for part in parts {
        body += &format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": part}, "finish_reason": null}]})
        );
    }
    if finish {
        body +=
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        FakeResponse::sse(body)
    } else {
        FakeResponse::sse(body).stall()
    }
}

fn reply(text: &str) -> FakeResponse {
    sse(&[text], true)
}

struct Harness {
    _dir: tempfile::TempDir,
    endpoint: Endpoint,
    main: FakeServer,
    other: FakeServer,
}

impl Harness {
    async fn start(main: Vec<FakeResponse>, other: Vec<FakeResponse>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let main = FakeServer::start(main).await;
        let other = FakeServer::start(other).await;
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[defaults]\nprofile = \"fake\"\n\n\
                 [profiles.fake]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"fake-model\"\n\n\
                 [profiles.other]\nprovider = \"openai_compat\"\nbase_url = \"{}/v1\"\nmodel = \"other-model\"\n",
                main.url(),
                other.url()
            ),
        )
        .unwrap();
        let endpoint = if cfg!(windows) {
            let unique = dir.path().file_name().unwrap().to_string_lossy();
            Endpoint::Pipe(format!("lucy-chat-test-{unique}"))
        } else {
            Endpoint::Path(dir.path().join("core.sock"))
        };
        let options = CoreOptions {
            endpoint: endpoint.clone(),
            config_path,
            db_path: dir.path().join("lucy.sqlite"),
            personas_dir: None,
            secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
            idle_timeout: Duration::from_secs(3600),
            token_store: None,
        };
        tokio::spawn(async move { serve(options).await.unwrap() });
        for _ in 0..200 {
            if Client::connect(&endpoint, "probe").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            _dir: dir,
            endpoint,
            main,
            other,
        }
    }

    /// Run a chat session over `lines`; returns the transcript (no colour).
    async fn chat(&self, start: Start, lines: &[&str]) -> String {
        self.chat_interrupting_at(start, lines, None).await
    }

    /// Like `chat`, pressing Ctrl+C the moment the transcript contains `trigger`.
    async fn chat_interrupting_at(
        &self,
        start: Start,
        lines: &[&str],
        trigger: Option<&str>,
    ) -> String {
        let mut client = Client::connect(&self.endpoint, "test").await.unwrap();
        let mut input = Scripted(lines.iter().map(|l| l.to_string()).collect());
        let (tx, mut interrupts) = mpsc::channel(1);
        let mut out = Tripwire {
            transcript: Vec::new(),
            trigger: trigger.map(|t| (t.to_owned(), tx)),
        };
        run_chat(
            &mut client,
            &mut input,
            &mut out,
            &mut interrupts,
            ChatOptions {
                color: false,
                width: 80,
                start,
            },
        )
        .await
        .unwrap();
        String::from_utf8(out.transcript).unwrap()
    }
}

/// Collects output and sends an interrupt once the trigger text has been written.
struct Tripwire {
    transcript: Vec<u8>,
    trigger: Option<(String, mpsc::Sender<()>)>,
}

impl std::io::Write for Tripwire {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.transcript.extend_from_slice(data);
        if let Some((text, _)) = &self.trigger
            && String::from_utf8_lossy(&self.transcript).contains(text.as_str())
            && let Some((_, tx)) = self.trigger.take()
        {
            let _ = tx.try_send(());
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Scripted(VecDeque<String>);

impl LineSource for Scripted {
    async fn read_line(&mut self, _prompt: &str) -> ReadLine {
        match self.0.pop_front() {
            Some(line) => ReadLine::Text(line),
            None => ReadLine::Eof,
        }
    }
}

fn messages_sent(server: &FakeServer, request: usize) -> Vec<(String, String)> {
    server.requests()[request].body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["role"].as_str().unwrap().into(),
                m["content"].as_str().unwrap().into(),
            )
        })
        .collect()
}

#[tokio::test]
async fn replies_print_and_follow_ups_stay_in_the_same_chat() {
    let h = Harness::start(vec![reply("Hey, choom."), reply("Still here.")], vec![]).await;

    let transcript = h.chat(Start::New, &["hi", "you there?"]).await;

    assert!(transcript.contains("Hey, choom.\n"), "{transcript}");
    assert!(transcript.contains("Still here.\n"), "{transcript}");
    let second = messages_sent(&h.main, 1);
    assert_eq!(
        second[1..].to_vec(),
        vec![
            ("user".into(), "hi".into()),
            ("assistant".into(), "Hey, choom.".into()),
            ("user".into(), "you there?".into()),
        ]
    );
}

#[tokio::test]
async fn model_lists_profiles_and_switches_this_chat() {
    let h = Harness::start(vec![], vec![reply("Other model here.")]).await;

    let transcript = h
        .chat(Start::New, &["/model", "/model other", "hello"])
        .await;

    assert!(
        transcript.contains("fake") && transcript.contains("other-model"),
        "{transcript}"
    );
    assert!(
        transcript.contains("Switched this chat to other (other-model)."),
        "{transcript}"
    );
    assert!(transcript.contains("Other model here."), "{transcript}");
    assert_eq!(h.main.requests().len(), 0);
    assert_eq!(h.other.requests().len(), 1);
}

#[tokio::test]
async fn plain_toggles_before_the_first_message() {
    let h = Harness::start(vec![reply("Answer.")], vec![]).await;

    let transcript = h.chat(Start::New, &["/plain", "hi"]).await;

    assert!(transcript.contains("Plain mode on."), "{transcript}");
    let system = &messages_sent(&h.main, 0)[0].1;
    assert!(
        !system.contains("## Who you are"),
        "voice still in the prompt"
    );
}

#[tokio::test]
async fn new_chats_are_listed_and_continue_picks_up_the_latest() {
    let h = Harness::start(
        vec![reply("One."), reply("Two."), reply("Continued.")],
        vec![],
    )
    .await;

    let transcript = h
        .chat(
            Start::New,
            &["first chat", "/new", "second chat", "/sessions"],
        )
        .await;
    assert!(transcript.contains("New chat."), "{transcript}");
    let listing = transcript.split("/sessions").last().unwrap_or(&transcript);
    assert!(
        listing.contains("second chat") && listing.contains("first chat"),
        "{transcript}"
    );

    let resumed = h.chat(Start::Continue, &["and more"]).await;
    assert!(
        resumed.contains("second chat"),
        "resume banner should name the chat: {resumed}"
    );
    let sent = messages_sent(&h.main, 2);
    assert!(sent.contains(&("user".into(), "second chat".into())));
    assert!(!sent.contains(&("user".into(), "first chat".into())));
}

#[tokio::test]
async fn ctrl_c_stops_the_reply_and_the_chat_goes_on() {
    let h = Harness::start(
        vec![sse(&["Once upon", " a time"], false), reply("Okay.")],
        vec![],
    )
    .await;
    let transcript = h
        .chat_interrupting_at(
            Start::New,
            &["story?", "never mind"],
            Some("Once upon a time"),
        )
        .await;

    assert!(transcript.contains("Once upon a time"), "{transcript}");
    assert!(transcript.contains("(stopped)"), "{transcript}");
    assert!(transcript.contains("Okay."), "{transcript}");
}

#[tokio::test]
async fn mistakes_are_explained_and_double_slash_sends_text() {
    let h = Harness::start(vec![reply("Paths are fine.")], vec![]).await;

    let transcript = h
        .chat(Start::New, &["/frobnicate", "/resume zzzz", "//etc/hosts?"])
        .await;

    assert!(
        transcript.contains("unknown command /frobnicate"),
        "{transcript}"
    );
    assert!(transcript.contains("no session matching"), "{transcript}");
    assert_eq!(messages_sent(&h.main, 0).last().unwrap().1, "/etc/hosts?");
}
