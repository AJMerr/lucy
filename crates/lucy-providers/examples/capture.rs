//! Record a real streamed response as a test fixture.
//!
//! ```sh
//! cargo run -p lucy-providers --example capture -- \
//!     openai_compat http://localhost:11434/v1 qwen3:14b \
//!     crates/lucy-providers/tests/fixtures/openai_compat/captured_ollama.sse
//!
//! LUCY_CAPTURE_KEY=sk-ant-... cargo run -p lucy-providers --example capture -- \
//!     anthropic https://api.anthropic.com claude-sonnet-5-5 \
//!     crates/lucy-providers/tests/fixtures/anthropic/captured.sse
//! ```
//!
//! Add `tools` after the output path to offer a `read_file` tool and ask a question that needs
//! it, recording a tool call instead of plain text.
//!
//! The API key, if needed, comes from `LUCY_CAPTURE_KEY`. Response bytes are written exactly as
//! received; request headers (including the key) are never written.

use std::process::ExitCode;

use futures::StreamExt;
use serde_json::{Value, json};

const SYSTEM: &str = "You are Lucy, a terse assistant.";
const PROMPT: &str = "Say hello in exactly five words.";
const TOOL_PROMPT: &str = "What does notes.txt say? Read it.";

fn read_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"path": {"type": "string", "description": "File to read"}},
        "required": ["path"],
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (kind, base_url, model, out, tools) = match args.as_slice() {
        [kind, base_url, model, out] => (kind, base_url, model, out, false),
        [kind, base_url, model, out, flag] if flag == "tools" => (kind, base_url, model, out, true),
        _ => {
            eprintln!(
                "usage: capture <openai_compat|anthropic> <base_url> <model> <out.sse> [tools]"
            );
            return ExitCode::FAILURE;
        }
    };
    let prompt = if tools { TOOL_PROMPT } else { PROMPT };
    let key = std::env::var("LUCY_CAPTURE_KEY").ok();
    let base_url = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let request = match kind.as_str() {
        "openai_compat" => {
            let mut body = json!({
                "model": model,
                "stream": true,
                "messages": [
                    {"role": "system", "content": SYSTEM},
                    {"role": "user", "content": prompt},
                ],
            });
            if tools {
                body["tools"] = json!([{"type": "function", "function": {
                    "name": "read_file",
                    "description": "Read a text file",
                    "parameters": read_file_schema(),
                }}]);
            }
            let mut request = client
                .post(format!("{base_url}/chat/completions"))
                .json(&body);
            if let Some(key) = &key {
                request = request.bearer_auth(key);
            }
            request
        }
        "anthropic" => {
            let Some(key) = &key else {
                eprintln!("set LUCY_CAPTURE_KEY to your Anthropic API key");
                return ExitCode::FAILURE;
            };
            let mut body = json!({
                "model": model,
                "max_tokens": 1024,
                "stream": true,
                "system": SYSTEM,
                "messages": [{"role": "user", "content": prompt}],
            });
            if tools {
                body["tools"] = json!([{
                    "name": "read_file",
                    "description": "Read a text file",
                    "input_schema": read_file_schema(),
                }]);
            }
            client
                .post(format!("{base_url}/v1/messages"))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
        }
        other => {
            eprintln!("unknown provider kind `{other}`");
            return ExitCode::FAILURE;
        }
    };

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            eprintln!("request failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    let status = response.status();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => body.extend_from_slice(&bytes),
            Err(err) => {
                eprintln!("stream failed after {} bytes: {err}", body.len());
                return ExitCode::FAILURE;
            }
        }
    }
    if !status.is_success() {
        eprintln!("{status}: {}", String::from_utf8_lossy(&body));
        return ExitCode::FAILURE;
    }
    if let Err(err) = std::fs::write(out, &body) {
        eprintln!("could not write {out}: {err}");
        return ExitCode::FAILURE;
    }
    println!("wrote {} bytes to {out}", body.len());
    ExitCode::SUCCESS
}
