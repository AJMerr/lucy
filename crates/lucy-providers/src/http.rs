//! HTTP status and error-body handling shared by the adapters.

use serde_json::Value;

use crate::ProviderError;

/// Pass a successful response through; turn anything else into a classified error that
/// carries the server's own message.
pub(crate) async fn check_status(
    response: reqwest::Response,
) -> Result<reqwest::Response, ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str(&body)
        .ok()
        .and_then(|value: Value| error_message(&value))
        .unwrap_or_else(|| body.trim().chars().take(500).collect());
    let message = format!("{status}: {message}");

    Err(match status.as_u16() {
        401 | 403 => ProviderError::Auth(message),
        408 | 429 | 500.. => ProviderError::Unavailable(message),
        _ => ProviderError::Request(message),
    })
}

/// The human-readable message in an error body. Handles `{"error": {"message": ..}}`
/// (OpenAI, Anthropic) and `{"error": ".."}` (Ollama).
pub(crate) fn error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_owned)
}

/// Describe a request that never got a response, including the underlying cause.
pub(crate) fn send_error(url: &str, err: reqwest::Error) -> ProviderError {
    let mut cause = String::new();
    let mut source = std::error::Error::source(&err);
    while let Some(inner) = source {
        cause = inner.to_string();
        source = inner.source();
    }
    let action = if err.is_connect() {
        "could not connect to"
    } else if err.is_timeout() {
        "timed out talking to"
    } else {
        "request failed for"
    };
    ProviderError::Unavailable(if cause.is_empty() {
        format!("{action} {url}")
    } else {
        format!("{action} {url}: {cause}")
    })
}
