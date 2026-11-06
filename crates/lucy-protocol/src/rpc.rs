//! JSON-RPC 2.0 messages between the core and front ends, one JSON object per line.
//! See `docs/adr/0003-ipc-local-socket-and-optional-http.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Role, ToolCall};

/// Bumped on any incompatible change; `hello` refuses mismatched clients.
pub const PROTOCOL_VERSION: u32 = 2;

pub mod method {
    pub const HELLO: &str = "hello";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const CHAT_SEND: &str = "chat.send";
    pub const CHAT_CANCEL: &str = "chat.cancel";
    pub const SESSIONS_CREATE: &str = "sessions.create";
    pub const SESSIONS_LIST: &str = "sessions.list";
    pub const SESSIONS_LOAD: &str = "sessions.load";
    pub const SESSIONS_RENAME: &str = "sessions.rename";
    pub const SESSIONS_DELETE: &str = "sessions.delete";
    pub const SESSIONS_RESOLVE: &str = "sessions.resolve";
    pub const SESSION_SETTINGS: &str = "session.settings";
    pub const SESSION_SET_PROFILE: &str = "session.set_profile";
    pub const SESSION_SET_PLAIN: &str = "session.set_plain";
    pub const PROFILES_LIST: &str = "profiles.list";
    pub const TOOLS_LIST: &str = "tools.list";
    pub const TOOL_APPROVE: &str = "tool.approve";
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
    /// The user's working folder: relative paths and commands start here. Defaults to home.
    #[serde(default)]
    pub cwd: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolsListParams {
    /// Show this session's rules, including tools approved with "always".
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolsList {
    pub tools: Vec<ToolInfo>,
    /// Configured MCP servers that couldn't start, and why.
    #[serde(default)]
    pub problems: Vec<ServerProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerProblem {
    pub server: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    /// Which server provides it: `builtin`, or an MCP server's name from config.
    pub server: String,
    pub description: String,
    /// What happens when Lucy calls it in Assist mode.
    pub rule: ToolRuleInfo,
}

/// How a call is handled: run straight away, ask first, or refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuleInfo {
    Auto,
    /// Read-only, but asks for private files or anything outside home.
    AutoInHome,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveParams {
    pub session_id: String,
    pub call_id: String,
    pub decision: Decision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Run this call.
    Once,
    /// Run this call, and later calls of the same tool in this session without asking.
    Always,
    Deny,
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
    /// A tool call needs the user's go-ahead; answer with `tool.approve`. Any connected
    /// client may answer; the first answer counts.
    ToolApproval {
        session_id: String,
        turn_id: u64,
        call: ToolCall,
        /// Why it needs approval, e.g. "can change files".
        reason: String,
        /// What it would change, e.g. a diff for a file edit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    ToolStarted {
        session_id: String,
        turn_id: u64,
        call: ToolCall,
    },
    ToolFinished {
        session_id: String,
        turn_id: u64,
        call_id: String,
        /// Exactly what the model was given.
        output: String,
        is_error: bool,
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
            | Event::ToolApproval { session_id, .. }
            | Event::ToolStarted { session_id, .. }
            | Event::ToolFinished { session_id, .. }
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
