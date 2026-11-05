//! JSON-RPC 2.0 messages between the core and front ends, one JSON object per line.
//! See `docs/adr/0003-ipc-local-socket-and-optional-http.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Role;

/// Bumped on any incompatible change; `hello` refuses mismatched clients.
pub const PROTOCOL_VERSION: u32 = 1;

pub mod method {
    pub const HELLO: &str = "hello";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const CHAT_SEND: &str = "chat.send";
    pub const CHAT_CANCEL: &str = "chat.cancel";
    pub const SESSIONS_LIST: &str = "sessions.list";
    pub const SESSIONS_LOAD: &str = "sessions.load";
    pub const SESSIONS_RENAME: &str = "sessions.rename";
    pub const SESSIONS_DELETE: &str = "sessions.delete";
    pub const SESSIONS_RESOLVE: &str = "sessions.resolve";
    pub const SESSION_SETTINGS: &str = "session.settings";
    pub const SESSION_SET_PROFILE: &str = "session.set_profile";
    pub const SESSION_SET_PLAIN: &str = "session.set_plain";
    pub const PROFILES_LIST: &str = "profiles.list";
    /// Notification carrying an [`Event`](super::Event).
    pub const EVENT: &str = "event";
}

pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Anything Lucy-specific: unknown session, missing key, busy session, ...
    pub const APP_ERROR: i64 = -32000;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn app(message: impl Into<String>) -> Self {
        Self::new(code::APP_ERROR, message)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn event(event: &Event) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method::EVENT.into(),
            params: serde_json::to_value(event).expect("events always serialize"),
        }
    }
}

// --- Method params and results ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol_version: u32,
    /// e.g. "lucy-cli 0.1.0".
    pub client: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol_version: u32,
    pub core_version: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSendParams {
    /// Continue this session (full ID or unique prefix); `None` starts a new one.
    #[serde(default)]
    pub session_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSendResult {
    pub session_id: String,
    pub turn_id: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionParams {
    /// Full ID or unique prefix.
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelResult {
    /// Whether a reply was in progress.
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenameParams {
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveParams {
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetProfileParams {
    pub session_id: String,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlainParams {
    pub session_id: String,
    pub plain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSettings {
    pub profile: String,
    pub plain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredMessage {
    pub role: Role,
    pub content: String,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub incomplete: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub summary: SessionSummary,
    pub messages: Vec<StoredMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub is_default: bool,
}

// --- Events -------------------------------------------------------------------------------

/// Live session activity, broadcast as `event` notifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    UserMessage {
        session_id: String,
        text: String,
    },
    TurnStarted {
        session_id: String,
        turn_id: u64,
        profile: String,
        model: String,
    },
    Delta {
        session_id: String,
        turn_id: u64,
        text: String,
    },
    TurnFinished {
        session_id: String,
        turn_id: u64,
        outcome: TurnOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl Event {
    pub fn session_id(&self) -> &str {
        match self {
            Event::UserMessage { session_id, .. }
            | Event::TurnStarted { session_id, .. }
            | Event::Delta { session_id, .. }
            | Event::TurnFinished { session_id, .. } => session_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Complete,
    Cancelled,
    Failed,
}
