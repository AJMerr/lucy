//! JSON-RPC 2.0 messages between the core and front ends, one JSON object per line.
//! See `docs/adr/0003-ipc-local-socket-and-optional-http.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentLevel, Mode, Role, ToolCall};

/// Bumped on any incompatible change; `hello` refuses mismatched clients.
/// `core.status` and `core.shutdown` keep their shape across versions, so a newer client can
/// still stop an older core (from v4 on).
pub const PROTOCOL_VERSION: u32 = 6;

pub mod method {
    pub const HELLO: &str = "hello";
    /// Version, PID and busy chats; allowed before `hello`, whatever the client's version.
    pub const CORE_STATUS: &str = "core.status";
    /// Stop the core (refused while replies are in progress unless forced); allowed before
    /// `hello`.
    pub const CORE_SHUTDOWN: &str = "core.shutdown";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const CHAT_SEND: &str = "chat.send";
    pub const CHAT_CANCEL: &str = "chat.cancel";
    /// `/clear`: she starts afresh from here; the chat stays as it was.
    pub const CHAT_CLEAR: &str = "chat.clear";
    /// `/compress`: summarise the chat so far now (`{summary}`).
    pub const CHAT_COMPRESS: &str = "chat.compress";
    /// A provider's models (for adding one), before any profile uses them.
    pub const MODELS_LIST: &str = "models.list";
    /// Send a model a test message (`{reply}`), before saving a profile for it.
    pub const MODELS_TRY: &str = "models.try";
    pub const SESSIONS_CREATE: &str = "sessions.create";
    pub const SESSIONS_LIST: &str = "sessions.list";
    pub const SESSIONS_LOAD: &str = "sessions.load";
    pub const SESSIONS_RENAME: &str = "sessions.rename";
    pub const SESSIONS_DELETE: &str = "sessions.delete";
    pub const SESSIONS_RESOLVE: &str = "sessions.resolve";
    pub const SESSION_SETTINGS: &str = "session.settings";
    pub const SESSION_SET_PROFILE: &str = "session.set_profile";
    pub const SESSION_SET_PLAIN: &str = "session.set_plain";
    pub const SESSION_SET_MODE: &str = "session.set_mode";
    pub const SESSION_SET_LEVEL: &str = "session.set_level";
    /// Show (and optionally restore) the files the last reply with changes touched.
    pub const SESSION_UNDO: &str = "session.undo";
    pub const PROFILES_LIST: &str = "profiles.list";
    pub const TOOLS_LIST: &str = "tools.list";
    pub const TOOL_APPROVE: &str = "tool.approve";
    /// The config file as JSON (with defaults filled in), where it lives, and whether
    /// home-manager owns it.
    pub const CONFIG_GET: &str = "config.get";
    /// Change config.toml with a JSON merge patch; refused if the result is invalid.
    pub const CONFIG_UPDATE: &str = "config.update";
    /// API keys the config refers to and whether each is set; never their values.
    pub const KEYS_STATUS: &str = "keys.status";
    /// Store an API key in the keyring of the core's machine.
    pub const KEYS_SET: &str = "keys.set";
    pub const KEYS_REMOVE: &str = "keys.remove";
    /// The persona in use (your override, or the built-in one), to edit.
    pub const PERSONA_GET: &str = "persona.get";
    /// Save your own version of the persona (personas/<name>.md).
    pub const PERSONA_SET: &str = "persona.set";
    /// Delete your version, back to the built-in persona.
    pub const PERSONA_RESET: &str = "persona.reset";
    /// Sign in to a remote MCP server: opens the browser on the core's machine and answers
    /// once signed in. Not for remote clients.
    pub const MCP_LOGIN: &str = "mcp.login";
    /// The model providers first-run setup offers (the same as `lucy config init`).
    pub const SETUP_PRESETS: &str = "setup.presets";
    /// A picture kept with a chat, by its id (`ImageRef::id`).
    pub const IMAGES_GET: &str = "images.get";
    /// Make a picture straight away (`/image`), as a turn: returns at once, then the picture
    /// arrives as a `generate_image` tool call's result.
    pub const IMAGES_GENERATE: &str = "images.generate";
    /// Lucy's long-term memory: memory.md's text and its facts.
    pub const MEMORY_GET: &str = "memory.get";
    /// Replace memory.md's text.
    pub const MEMORY_SET: &str = "memory.set";
    /// Take one fact back out of memory (`{fact}`), e.g. undoing one she just saved.
    pub const MEMORY_FORGET: &str = "memory.forget";
    /// Reminders and scheduled tasks, and whether the core runs at login.
    pub const SCHEDULES_LIST: &str = "schedules.list";
    /// Change a reminder or task (pause, reword, reschedule, level, model).
    pub const SCHEDULES_UPDATE: &str = "schedules.update";
    pub const SCHEDULES_DELETE: &str = "schedules.delete";
    /// Replace the token remote clients present (`{token}`), and disconnect every remote
    /// client. Only from the core's own machine.
    pub const REMOTE_ROTATE: &str = "remote.rotate";
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
    /// For remote clients: the folder on the core's machine where their chats' tools run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// `config.get`: what the settings screens show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigView {
    /// Where config.toml lives on the core's machine.
    pub path: String,
    /// home-manager owns the file, so it can't be changed from here.
    pub managed: bool,
    /// The config with defaults filled in; `None` before the first-run setup.
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigUpdateParams {
    /// A JSON merge patch (RFC 7396): objects merge, `null` removes, anything else replaces.
    pub patch: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigUpdateResult {
    /// The change only takes effect once the core restarts (remote access settings).
    pub restart_needed: bool,
}

/// One API key the config refers to (`keys.status`). Never carries the key itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyStatus {
    pub name: String,
    /// The profiles and MCP servers that use it.
    pub used_by: Vec<String>,
    /// Where it's set: `keyring`, or the environment variable's name; `None` when missing.
    pub source: Option<String>,
    /// The keyring couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySetParams {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyParams {
    pub name: String,
}

/// `persona.get`: the persona the config uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaView {
    pub name: String,
    pub text: String,
    /// Your own version is in use, not the built-in one.
    pub overridden: bool,
    /// Where your version lives (or would), on the core's machine.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaSetParams {
    pub text: String,
}

/// A model provider first-run setup offers (`setup.presets`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderPreset {
    /// e.g. "Anthropic (Claude)".
    pub label: String,
    /// The profile it becomes, e.g. "claude".
    pub profile_name: String,
    /// `openai_compat` or `anthropic`.
    pub provider: String,
    /// `None` for Anthropic (built in), and for a custom endpoint, where you give one.
    pub base_url: Option<String>,
    /// The suggested model; `None` for a custom endpoint.
    pub model: Option<String>,
    /// The keyring name its API key goes under; `None` when it needs no key.
    pub key_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreStatus {
    pub protocol_version: u32,
    pub core_version: String,
    pub pid: u32,
    /// Started by the login service (runs until stopped) rather than on demand.
    pub service: bool,
    /// Chats with a reply in progress.
    pub busy: Vec<BusyChat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusyChat {
    pub session_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShutdownParams {
    /// Cancel replies in progress (keeping what was said) instead of refusing.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShutdownResult {
    /// False when replies are in progress and `force` wasn't set; the core keeps running.
    pub stopping: bool,
    pub busy: Vec<BusyChat>,
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
    /// Pictures to go with the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageData>,
    /// Start a new chat as incognito (ignored when continuing one).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
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
    /// Also make it the default for new chats (and the sprite).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub make_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlainParams {
    pub session_id: String,
    pub plain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetModeParams {
    pub session_id: String,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetLevelParams {
    pub session_id: String,
    pub level: AgentLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndoParams {
    pub session_id: String,
    /// Restore the files; `false` only lists them.
    pub apply: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndoResult {
    /// Empty when there's nothing left to undo.
    pub files: Vec<UndoFile>,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndoFile {
    pub path: String,
    /// The reply created it, so undo deletes it.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSettings {
    pub profile: String,
    pub plain: bool,
    pub mode: Mode,
    pub level: AgentLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u32,
    /// An incognito chat: no memory, deleted when the client that started it leaves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
}

/// A picture kept with a chat (`StoredMessage::images`), in the core's data folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// Its name in the core's image store (a content hash plus extension).
    pub id: String,
    /// e.g. `image/png`.
    pub media_type: String,
    /// She made it (with `generate_image` or `/image`), rather than being shown it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub made: bool,
}

/// `images.generate`: make a picture from `prompt` in a chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageGenerateParams {
    /// Continue this session (full ID or unique prefix); `None` starts a new one.
    #[serde(default)]
    pub session_id: Option<String>,
    pub prompt: String,
}

/// A picture sent to or from the core: `chat.send`'s attachments, and `images.get`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    pub media_type: String,
    /// The file's bytes, base64-encoded.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageParams {
    pub id: String,
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
    /// Pictures you attached (fetch one with `images.get`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    /// Not a message but a marker: `cleared` (/clear) or `summary` (older turns summarised;
    /// `content` is the summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
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
    /// Read-only, but asks for private files and anything outside the folders it may read
    /// (the chat's folder, the workspace, `[tools] readable`). Named before reads became an
    /// allowlist; kept for compatibility.
    AutoInHome,
    /// Agent mode: changes files inside the working folder (or workspace); asks elsewhere.
    AutoInFolder,
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
        /// Pictures sent with it, kept with the chat (`images.get`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageRef>,
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
        /// Only a client on the machine running Lucy may approve (a reply driven from another
        /// machine, with `[remote] approvals = "local"`); a remote client can only decline.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        local_only: bool,
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
        /// Pictures the call gave back, kept with the chat (`images.get`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageRef>,
    },
    /// Something to tell the user now: a reminder, or a scheduled task finishing. Clients
    /// that subscribed with `notifications` show it; otherwise the core sends an OS
    /// notification. `session_id` is the Tasks chat.
    Notification {
        session_id: String,
        title: String,
        body: String,
    },
    TurnFinished {
        session_id: String,
        turn_id: u64,
        outcome: TurnOutcome,
        /// Why it failed, or which cap stopped it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Model steps and tokens the reply used (agent mode shows it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TurnUsage>,
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
            | Event::TurnFinished { session_id, .. }
            | Event::Notification { session_id, .. } => session_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Complete,
    /// Agent mode hit its step, cost or token cap; the reply ends with a summary.
    Capped,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnUsage {
    /// Model responses in this reply (one per tool round, plus the answer).
    pub steps: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// When the profile's prices are known (local models are free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// `memory.get`: Lucy's long-term memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryView {
    /// Where memory.md lives on the core's machine.
    pub path: String,
    /// The whole file.
    pub text: String,
    /// Its facts, in order.
    pub facts: Vec<MemoryFact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryFact {
    pub heading: String,
    pub text: String,
}

/// `memory.forget`: the fact to take out (as saved, or enough of it to pick one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryForgetParams {
    pub fact: String,
}

/// `memory.set`: replace memory.md's text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySetParams {
    pub text: String,
}

/// `subscribe`: every chat's events, not just the client's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscribeParams {
    /// This client shows notifications itself (the desktop app), so the core doesn't.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notifications: bool,
}

/// A reminder or scheduled task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleInfo {
    pub id: String,
    /// `reminder` (shows `what`) or `task` (runs `what` as a prompt).
    pub kind: String,
    pub title: String,
    pub what: String,
    /// When it runs, in words: "every day at 08:00".
    pub when: String,
    /// When it runs, as `schedule` takes it: `{"daily": "08:00"}`.
    pub when_spec: Value,
    /// Tasks: the agent level (0-3) it runs at.
    pub level: u8,
    /// Tasks: the profile that runs it.
    pub profile: String,
    pub paused: bool,
    /// Unix ms; `None` once a one-off has run.
    pub next_run_ms: Option<i64>,
    pub last_run_ms: Option<i64>,
}

/// `schedules.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulesList {
    pub schedules: Vec<ScheduleInfo>,
    /// The core runs at login, so tasks run even if nothing else started it.
    pub always_on: bool,
}

/// `schedules.update`: fields left out stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScheduleUpdate {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// `schedules.delete`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleId {
    pub id: String,
}

/// `models.list`: a provider's models, with a key from the keyring (`key_name`) or given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsListParams {
    /// `openai_compat` or `anthropic`.
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
    /// A key not saved yet (it isn't kept).
    #[serde(default)]
    pub key: Option<String>,
}

/// `models.try`: like `models.list`, for one model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsTryParams {
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub key_name: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}
