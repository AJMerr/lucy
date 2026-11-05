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
//! The API key, if needed, comes from `LUCY_CAPTURE_KEY`. Response bytes are written exactly as
//! received; request headers (including the key) are never written.

use std::process::ExitCode;

use futures::StreamExt;
use serde_json::json;

const SYSTEM: &str = "You are Lucy, a terse assistant.";
const PROMPT: &str = "Say hello in exactly five words.";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [kind, base_url, model, out] = args.as_slice() else {
        eprintln!("usage: capture <openai_compat|anthropic> <base_url> <model> <out.sse>");
        return ExitCode::FAILURE;
    };
    let key = std::env::var("LUCY_CAPTURE_KEY").ok();
    let base_url = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let request = match kind.as_str() {
        "openai_compat" => {
            let mut request = client
                .post(format!("{base_url}/chat/completions"))
                .json(&json!({
                    "model": model,
                    "stream": true,
                    "messages": [
                        {"role": "system", "content": SYSTEM},
                        {"role": "user", "content": PROMPT},
                    ],
                }));
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
            client
                .post(format!("{base_url}/v1/messages"))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "max_tokens": 1024,
                    "stream": true,
                    "system": SYSTEM,
                    "messages": [{"role": "user", "content": PROMPT}],
                }))
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
