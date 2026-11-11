//! The core's local-socket server: JSON-RPC over newline-delimited JSON (ADR 0003).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use lucy_client::Endpoint;
use lucy_protocol::rpc::{
    self, ApproveParams, BusyChat, CancelResult, ChatSendParams, ChatSendResult,
    ConfigUpdateParams, ConfigUpdateResult, ConfigView, CoreStatus, Decision, Event, HelloParams,
    HelloResult, ImageGenerateParams, ImageParams, KeyParams, KeySetParams, KeyStatus,
    MemorySetParams, ModelsListParams, ModelsTryParams, Notification, PROTOCOL_VERSION,
    PersonaSetParams, PersonaView, ProfileInfo, RenameParams, Request, ResolveParams, Response,
    RpcError, ScheduleId, ServerProblem, SessionParams, SessionSettings, SetLevelParams,
    SetModeParams, SetPlainParams, SetProfileParams, ShutdownParams, ShutdownResult,
    SubscribeParams, ToolsList, ToolsListParams, TurnOutcome, TurnUsage, UndoFile, UndoParams,
    UndoResult, code, method,
};
use lucy_protocol::{AgentLevel, Message, Mode, Role, ToolCall};
use lucy_providers::{ChatRequest, Provider, ProviderError, StreamEvent, Usage};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::config::{Config, McpServer, ProviderKind, ToolSettings};
use crate::mcp::McpHost;
use crate::persona::{Personas, PromptContext, build_system_prompt, detect_machine};
use crate::provider::{build_provider, build_provider_with_key};

use crate::generate::Generator;
use crate::secrets::{KeySource, SecretStore, Secrets};
use crate::sessions::{
    ChatSettings, NewMessage, Origin, SessionId, SessionStore, StoreError, new_reply_id,
};
use crate::tools::{Access, GeneratorSlot, ToolBox, ToolOutput, Verdict, close_dangling_calls};

mod remote;
mod schedules;

/// The keyring entry holding the token remote clients must present (`lucy remote token`).
pub const REMOTE_TOKEN_KEY: &str = "remote-token";

/// How long a tool call waits for the user's approval before it's treated as declined.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub type DynSecrets =
    Secrets<Box<dyn SecretStore + Send + Sync>, Box<dyn Fn(&str) -> Option<String> + Send + Sync>>;

pub struct CoreOptions {
    pub endpoint: Endpoint,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    /// User persona folder; `None` uses only the built-in personas.
    pub personas_dir: Option<PathBuf>,
    pub secrets: DynSecrets,
    /// Exit after this long with no connections and no reply streaming. `None` runs until
    /// stopped with `core.shutdown` (the login service).
    pub idle_timeout: Option<Duration>,
    /// Where remote MCP servers' OAuth tokens live; `None` keeps them in memory (tests).
    pub token_store: Option<crate::oauth::TokenStore>,
    /// The clock reminders and tasks run by; `None` is the real one (tests use a fake).
    pub clock: Option<Arc<dyn crate::schedule::Clock>>,
    /// How the core tells you something when no app shows notifications; `None` sends a
    /// desktop notification.
    pub notifier: Option<Arc<dyn crate::schedule::Notifier>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("another Lucy core is already listening at {0}")]
    AlreadyRunning(String),
    #[error("could not listen at {endpoint}: {source}")]
    Listen {
        endpoint: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Counts what keeps the core awake.
#[derive(Default)]
struct Activity {
    connections: AtomicUsize,
    turns: AtomicUsize,
    /// Reminders and tasks waiting to run: an on-demand core stays up for them.
    schedules: AtomicUsize,
    changed: Notify,
}

impl Activity {
    fn is_idle(&self) -> bool {
        self.connections.load(Ordering::SeqCst) == 0
            && self.turns.load(Ordering::SeqCst) == 0
            && self.schedules.load(Ordering::SeqCst) == 0
    }

    fn set(&self, counter: &AtomicUsize, value: usize) {
        counter.store(value, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    fn add(&self, counter: &AtomicUsize, delta: isize) {
        if delta > 0 {
            counter.fetch_add(delta as usize, Ordering::SeqCst);
        } else {
            counter.fetch_sub(delta.unsigned_abs(), Ordering::SeqCst);
        }
        self.changed.notify_waiters();
    }
}

struct Core {
    options: CoreOptions,
    store: Mutex<SessionStore>,
    personas: Personas,
    machine: String,
    /// Per-session state that isn't saved (chat settings live in the session store).
    overrides: Mutex<HashMap<String, Overrides>>,
    /// Sessions with a reply in progress, and how to cancel it.
    turns: Mutex<HashMap<String, CancellationToken>>,
    next_turn: AtomicU64,
    events: broadcast::Sender<Event>,
    activity: Activity,
    /// Tool calls waiting for the user, by (session, call ID).
    approvals: Mutex<HashMap<(String, String), oneshot::Sender<Decision>>>,
    /// External MCP servers, shared by every chat.
    mcp: McpHost,
    /// Cancelled by `core.shutdown`.
    shutdown: CancellationToken,
    /// Her long-term memory (memory.md).
    memory: Arc<crate::memory::Memory>,
    clock: Arc<dyn crate::schedule::Clock>,
    notifier: Arc<dyn crate::schedule::Notifier>,
    /// Reminders or tasks were added, changed or removed.
    schedules_changed: Notify,
    /// Clients that show notifications themselves (the desktop app).
    notifying: AtomicUsize,
    /// Context sizes learned from providers, by provider, address and model.
    context_limits: Mutex<HashMap<String, Option<u32>>>,
}

#[derive(Default, Clone)]
struct Overrides {
    /// Tools approved with "always" in this session.
    allowed_tools: HashSet<String>,
}

/// Per-connection state shared by its reader and event forwarder.
#[derive(Default)]
struct ConnState {
    hello: bool,
    subscribed: bool,
    own_sessions: HashSet<String>,
    /// Incognito chats this client started: deleted when it leaves.
    incognito: HashSet<String>,
    /// It shows notifications itself (counted in `Core::notifying`).
    notifications: bool,
    /// Came in over WebSocket: its chats work in the core's folder, at a capped level.
    remote: bool,
}

/// How long a stopping core waits for cancelled replies and open connections to wind down.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Run the core until it has been idle for `idle_timeout`, or until `core.shutdown`.
pub async fn serve(options: CoreOptions) -> Result<(), CoreError> {
    let listener = bind(&options.endpoint).await?;
    let store = SessionStore::open(&options.db_path)?;
    // Incognito chats left by a core that stopped while one was open.
    for id in store.incognito_sessions()? {
        if let Ok(pictures) = store.delete_with_pictures(&id) {
            remove_pictures(&images_dir(&options.db_path), &pictures);
        }
    }
    let memory = Arc::new(crate::memory::Memory::new(
        options
            .db_path
            .parent()
            .map_or_else(|| PathBuf::from("memory.md"), |dir| dir.join("memory.md")),
    ));
    let (events, _) = broadcast::channel(1024);
    let idle_timeout = options.idle_timeout;
    let mcp = match &options.token_store {
        Some(store) => McpHost::with_token_store(Arc::clone(store)),
        None => McpHost::new(),
    };
    let clock: Arc<dyn crate::schedule::Clock> = options
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(crate::schedule::SystemClock));
    let notifier: Arc<dyn crate::schedule::Notifier> = options
        .notifier
        .clone()
        .unwrap_or_else(|| Arc::new(crate::schedule::DesktopNotifier));
    let core = Arc::new(Core {
        personas: Personas::new(options.personas_dir.clone()),
        options,
        store: Mutex::new(store),
        machine: detect_machine(),
        overrides: Mutex::default(),
        turns: Mutex::default(),
        next_turn: AtomicU64::new(1),
        events,
        activity: Activity::default(),
        approvals: Mutex::default(),
        mcp,
        shutdown: CancellationToken::new(),
        memory,
        clock,
        notifier,
        schedules_changed: Notify::new(),
        notifying: AtomicUsize::new(0),
        context_limits: Mutex::default(),
    });

    // Reminders and scheduled tasks.
    tokio::spawn(schedules::run(Arc::clone(&core)));

    // Accept in a task of its own: cancelling a pending accept would close the waiting pipe
    // instance on Windows, briefly leaving nothing to connect to.
    let acceptor = {
        let core = Arc::clone(&core);
        tokio::spawn(async move {
            loop {
                if let Ok(stream) = listener.accept().await {
                    core.activity.add(&core.activity.connections, 1);
                    let core = Arc::clone(&core);
                    tokio::spawn(async move {
                        socket_client(Arc::clone(&core), stream).await;
                        core.activity.add(&core.activity.connections, -1);
                    });
                }
            }
        })
    };

    let remote = remote::start(Arc::clone(&core)).await;

    loop {
        let changed = core.activity.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let idle = core.activity.is_idle();
        tokio::select! {
            _ = &mut changed => {}
            _ = core.shutdown.cancelled() => break,
            _ = tokio::time::sleep(idle_timeout.unwrap_or_default()),
                if idle && idle_timeout.is_some() => break,
        }
    }
    acceptor.abort();
    if let Some(remote) = remote {
        remote.abort();
    }
    // Dropping the listener (inside the aborted task) removes the socket file.
    let _ = acceptor.await;
    if core.shutdown.is_cancelled() {
        // Let cancelled replies save what they had, and clients read their last answers.
        let deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE;
        while !core.activity.is_idle() {
            let changed = core.activity.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if core.activity.is_idle() {
                break;
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
    }
    core.mcp.shutdown().await;
    Ok(())
}

async fn bind(endpoint: &Endpoint) -> Result<Listener, CoreError> {
    let listen_err = |source| CoreError::Listen {
        endpoint: endpoint.to_string(),
        source,
    };
    if let Endpoint::Path(path) = endpoint
        && let Some(dir) = path.parent()
    {
        std::fs::create_dir_all(dir).map_err(listen_err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(listen_err)?;
        }
    }
    let create = || {
        let options = ListenerOptions::new()
            .name(endpoint.name()?)
            .reclaim_name(true);
        // A named pipe's default security lets other accounts on the machine connect; allow
        // only this user (and SYSTEM), as the socket folder's 0700 does on Unix.
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            options.security_descriptor(windows_acl::only_this_user()?)
        };
        options.create_tokio()
    };
    match create() {
        Ok(listener) => Ok(listener),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            if Stream::connect(endpoint.name().map_err(listen_err)?)
                .await
                .is_ok()
            {
                return Err(CoreError::AlreadyRunning(endpoint.to_string()));
            }
            // A socket file left behind by a core that crashed.
            if let Endpoint::Path(path) = endpoint {
                let _ = std::fs::remove_file(path);
            }
            create().map_err(listen_err)
        }
        Err(err) => Err(listen_err(err)),
    }
}

/// A client on the local socket: one JSON message per line each way.
async fn socket_client(core: Arc<Core>, stream: Stream) {
    let stream = Arc::new(stream);
    let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = {
        let stream = Arc::clone(&stream);
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if (&*stream).write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        })
    };
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(&*stream).lines();
        while let Ok(Some(text)) = lines.next_line().await {
            if in_tx.send(text).is_err() {
                break;
            }
        }
    });
    serve_client(core, in_rx, out_tx, false).await;
    reader.abort();
    let _ = writer.await;
}

/// One client, whatever the transport: requests arrive on `incoming` until it closes, and
/// replies and events leave on `outgoing`.
async fn serve_client(
    core: Arc<Core>,
    mut incoming: mpsc::UnboundedReceiver<String>,
    out_tx: mpsc::UnboundedSender<String>,
    remote: bool,
) {
    let state = Arc::new(Mutex::new(ConnState {
        remote,
        ..ConnState::default()
    }));
    let forwarder = {
        let mut events = core.events.subscribe();
        let state = Arc::clone(&state);
        let out_tx = out_tx.clone();
        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let wanted = {
                    let state = state.lock().unwrap();
                    state.subscribed || state.own_sessions.contains(event.session_id())
                };
                if wanted && out_tx.send(line(&Notification::event(&event))).is_err() {
                    break;
                }
            }
        })
    };

    while let Some(text) = incoming.recv().await {
        let response = match serde_json::from_str::<Request>(&text) {
            Ok(request) => {
                let id = request.id;
                match handle(&core, &state, request).await {
                    Ok(result) => Response::ok(id, result),
                    Err(error) => Response::err(id, error),
                }
            }
            Err(err) => Response::err(0, RpcError::new(code::PARSE_ERROR, err.to_string())),
        };
        if out_tx.send(line(&response)).is_err() {
            break;
        }
    }

    if std::mem::take(&mut state.lock().unwrap().notifications) {
        core.notifying.fetch_sub(1, Ordering::SeqCst);
    }
    // Incognito chats end with the client that started them.
    let incognito = std::mem::take(&mut state.lock().unwrap().incognito);
    for key in incognito {
        core.forget_incognito(&key);
    }
    // Dropping the last sender lets the transport's writer flush and finish.
    forwarder.abort();
}

fn line(message: &impl Serialize) -> String {
    let mut text = serde_json::to_string(message).expect("protocol messages always serialize");
    text.push('\n');
    text
}

fn params<P: DeserializeOwned>(value: Value) -> Result<P, RpcError> {
    serde_json::from_value(value)
        .map_err(|err| RpcError::new(code::INVALID_PARAMS, err.to_string()))
}

fn result(value: impl Serialize) -> Result<Value, RpcError> {
    Ok(serde_json::to_value(value).expect("results always serialize"))
}

async fn handle(
    core: &Arc<Core>,
    state: &Arc<Mutex<ConnState>>,
    request: Request,
) -> Result<Value, RpcError> {
    if request.method == method::HELLO {
        let hello: HelloParams = params(request.params)?;
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(RpcError::app(format!(
                "client speaks protocol v{} but this core speaks protocol v{PROTOCOL_VERSION}",
                hello.protocol_version
            )));
        }
        let remote = {
            let mut state = state.lock().unwrap();
            state.hello = true;
            state.remote
        };
        let folder = match remote {
            true => Some(remote_folder(&core.config()?).display().to_string()),
            false => None,
        };
        return result(HelloResult {
            protocol_version: PROTOCOL_VERSION,
            core_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            folder,
        });
    }
    if request.method == method::CORE_SHUTDOWN && state.lock().unwrap().remote {
        return Err(RpcError::app(
            "a remote client can't stop the core (it couldn't start it again); stop it on its \
             own machine",
        ));
    }
    if request.method == method::MCP_LOGIN && state.lock().unwrap().remote {
        return Err(RpcError::app(
            "signing in opens a browser on the machine that runs Lucy, so do it there",
        ));
    }
    let changes_settings = matches!(
        request.method.as_str(),
        method::CONFIG_UPDATE
            | method::KEYS_SET
            | method::KEYS_REMOVE
            | method::PERSONA_SET
            | method::PERSONA_RESET
            | method::MEMORY_SET
    );
    if changes_settings
        && state.lock().unwrap().remote
        && !core.config().is_ok_and(|config| config.remote.allow_config)
    {
        return Err(RpcError::app(
            "remote clients can't change settings, keys or the persona; set `[remote] \
             allow_config = true` on the machine that runs Lucy to allow it",
        ));
    }
    // These two work before `hello`, so any client version can see and stop the core.
    if request.method == method::CORE_STATUS {
        return result(CoreStatus {
            protocol_version: PROTOCOL_VERSION,
            core_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            service: core.options.idle_timeout.is_none(),
            busy: core.busy(),
        });
    }
    if request.method == method::CORE_SHUTDOWN {
        let p: ShutdownParams = params(request.params)?;
        let busy = core.busy();
        if !busy.is_empty() && !p.force {
            return result(ShutdownResult {
                stopping: false,
                busy,
            });
        }
        for turn in core.turns.lock().unwrap().values() {
            turn.cancel();
        }
        core.shutdown.cancel();
        return result(ShutdownResult {
            stopping: true,
            busy,
        });
    }
    if !state.lock().unwrap().hello {
        return Err(RpcError::app("call hello first"));
    }
    match request.method.as_str() {
        method::CHAT_SEND => chat_send(core, state, params(request.params)?),
        method::IMAGES_GENERATE => images_generate(core, state, params(request.params)?),
        method::CHAT_CLEAR => {
            let p: SessionParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            core.not_busy(&key)?;
            let id: SessionId = key.parse().map_err(app)?;
            core.store
                .lock()
                .unwrap()
                .append(&id, NewMessage::cleared())
                .map_err(app)?;
            result(Value::Null)
        }
        method::CHAT_COMPRESS => {
            let p: SessionParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            core.not_busy(&key)?;
            result(json!({"summary": compress_now(core, &key).await?}))
        }
        method::MODELS_LIST => {
            let p: ModelsListParams = params(request.params)?;
            let (kind, base_url, key) =
                provider_access(core, &p.provider, p.base_url, p.key_name, p.key)?;
            result(
                lucy_providers::models::list_models(kind, &base_url, key.as_deref())
                    .await
                    .map_err(app)?,
            )
        }
        method::MODELS_TRY => {
            let p: ModelsTryParams = params(request.params)?;
            let (_, base_url, key) =
                provider_access(core, &p.provider, p.base_url, p.key_name, p.key)?;
            let profile = crate::config::Profile {
                provider: if p.provider == "anthropic" {
                    crate::config::ProviderKind::Anthropic
                } else {
                    crate::config::ProviderKind::OpenAiCompat
                },
                base_url: (!base_url.is_empty()).then_some(base_url),
                model: p.model,
                key_name: None,
                price_in: None,
                price_out: None,
                tools: false,
                vision: false,
                context_tokens: None,
            };
            let provider = build_provider_with_key("try", &profile, key).map_err(app)?;
            let request = lucy_providers::ChatRequest {
                messages: vec![Message::new(Role::User, "Reply with just the word OK.")],
                tools: Vec::new(),
            };
            let mut stream = provider.stream(request).await.map_err(app)?;
            let mut reply = String::new();
            while let Some(event) = stream.next().await {
                if let StreamEvent::TextDelta(delta) = event.map_err(app)? {
                    reply.push_str(&delta);
                }
            }
            result(json!({"reply": reply.trim()}))
        }
        method::CHAT_CANCEL => {
            let p: SessionParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            let turn = core.turns.lock().unwrap().get(&key).cloned();
            if let Some(turn) = &turn {
                turn.cancel();
            }
            result(CancelResult {
                cancelled: turn.is_some(),
            })
        }
        method::SUBSCRIBE => {
            let p: SubscribeParams = if request.params.is_null() {
                SubscribeParams::default()
            } else {
                params(request.params)?
            };
            let mut state = state.lock().unwrap();
            state.subscribed = true;
            if p.notifications && !state.notifications {
                state.notifications = true;
                core.notifying.fetch_add(1, Ordering::SeqCst);
            }
            result(Value::Null)
        }
        method::SCHEDULES_LIST => result(schedules::list(core)?),
        method::SCHEDULES_UPDATE => result(schedules::update(core, params(request.params)?)?),
        method::SCHEDULES_DELETE => {
            let p: ScheduleId = params(request.params)?;
            schedules::delete(core, &p.id)?;
            result(Value::Null)
        }
        method::SESSIONS_LOAD => {
            let p: SessionParams = params(request.params)?;
            let store = core.store.lock().unwrap();
            let id = store.resolve(&p.session_id).map_err(app)?;
            result(session_dto(store.load(&id).map_err(app)?))
        }
        method::SESSIONS_CREATE => {
            let id = core.store.lock().unwrap().create().map_err(app)?;
            result(id.to_string())
        }
        method::SESSIONS_LIST => {
            let sessions = core.store.lock().unwrap().list().map_err(app)?;
            result(sessions.into_iter().map(summary_dto).collect::<Vec<_>>())
        }
        method::SESSIONS_RESOLVE => {
            let p: ResolveParams = params(request.params)?;
            let id = core.store.lock().unwrap().resolve(&p.prefix).map_err(app)?;
            result(id.to_string())
        }
        method::SESSIONS_RENAME => {
            let p: RenameParams = params(request.params)?;
            let store = core.store.lock().unwrap();
            let id = store.resolve(&p.session_id).map_err(app)?;
            store.rename(&id, &p.title).map_err(app)?;
            result(Value::Null)
        }
        method::SESSIONS_DELETE => {
            let p: SessionParams = params(request.params)?;
            let id = core
                .store
                .lock()
                .unwrap()
                .resolve(&p.session_id)
                .map_err(app)?;
            let key = id.to_string();
            if let Some(turn) = core.turns.lock().unwrap().get(&key) {
                turn.cancel();
            }
            core.store.lock().unwrap().delete(&id).map_err(app)?;
            core.overrides.lock().unwrap().remove(&key);
            result(Value::Null)
        }
        method::SESSION_SETTINGS => {
            let p: SessionParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            result(core.settings(&key, &core.config()?)?)
        }
        method::SESSION_SET_PROFILE => {
            let p: SetProfileParams = params(request.params)?;
            let config = core.config()?;
            if !config.profiles.contains_key(&p.profile) {
                return Err(RpcError::app(format!(
                    "no profile named `{}` (available: {})",
                    p.profile,
                    config
                        .profiles
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            if p.make_default {
                crate::config::update_file(
                    &core.options.config_path,
                    &json!({"defaults": {"profile": p.profile}}),
                )
                .map_err(app)?;
            }
            core.update_settings(&p.session_id, &config, |s| s.profile = Some(p.profile))
        }
        method::SESSION_SET_PLAIN => {
            let p: SetPlainParams = params(request.params)?;
            core.update_settings(&p.session_id, &core.config()?, |s| s.plain = Some(p.plain))
        }
        method::SESSION_SET_MODE => {
            let p: SetModeParams = params(request.params)?;
            core.update_settings(&p.session_id, &core.config()?, |s| s.mode = Some(p.mode))
        }
        method::SESSION_SET_LEVEL => {
            let p: SetLevelParams = params(request.params)?;
            core.update_settings(&p.session_id, &core.config()?, |s| s.level = Some(p.level))
        }
        method::SESSION_UNDO => {
            let p: UndoParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            if core.turns.lock().unwrap().contains_key(&key) {
                return Err(RpcError::app(
                    "Lucy is still replying in this session; wait or cancel first",
                ));
            }
            result(core.undo(&key, p.apply).map_err(app)?)
        }
        method::CONFIG_GET => {
            let path = &core.options.config_path;
            let config = Config::load(path).map_err(app)?;
            result(ConfigView {
                path: path.display().to_string(),
                managed: matches!(
                    crate::config::check_writable(path),
                    Err(crate::config::ConfigError::Managed { .. })
                ),
                config: config
                    .map(|config| serde_json::to_value(config).map_err(app))
                    .transpose()?,
            })
        }
        method::CONFIG_UPDATE => {
            let p: ConfigUpdateParams = params(request.params)?;
            let path = &core.options.config_path;
            let before = Config::load(path).map_err(app)?;
            crate::config::update_file(path, &p.patch).map_err(app)?;
            let after = Config::load(path).map_err(app)?;
            // The listener for remote access is set up when the core starts.
            let remote = |config: &Option<Config>| config.as_ref().map(|c| c.remote.clone());
            result(ConfigUpdateResult {
                restart_needed: remote(&before) != remote(&after),
            })
        }
        method::KEYS_STATUS => {
            let config = core.config()?;
            result(
                config
                    .key_names()
                    .into_iter()
                    .map(|(name, used_by)| {
                        let (source, error) = match core.options.secrets.resolve(&name) {
                            Ok(Some(key)) => (
                                Some(match key.source {
                                    KeySource::Keyring => "keyring".to_owned(),
                                    KeySource::Env(var) => var,
                                }),
                                None,
                            ),
                            Ok(None) => (None, None),
                            Err(err) => (None, Some(err.to_string())),
                        };
                        KeyStatus {
                            name,
                            used_by,
                            source,
                            error,
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        }
        method::KEYS_SET => {
            let p: KeySetParams = params(request.params)?;
            if p.name.trim().is_empty() || p.value.is_empty() {
                return Err(RpcError::new(
                    code::INVALID_PARAMS,
                    "a key needs a name and a value",
                ));
            }
            core.options.secrets.set(&p.name, &p.value).map_err(app)?;
            result(Value::Null)
        }
        method::KEYS_REMOVE => {
            let p: KeyParams = params(request.params)?;
            result(core.options.secrets.remove(&p.name).map_err(app)?)
        }
        method::PERSONA_GET => {
            let name = core.config()?.defaults.persona;
            let persona = core.personas.load(&name).map_err(app)?;
            result(PersonaView {
                overridden: matches!(persona.source, crate::persona::PersonaSource::File(_)),
                path: core
                    .persona_file(&name)
                    .map(|path| path.display().to_string()),
                name: persona.name,
                text: persona.text,
            })
        }
        method::PERSONA_SET => {
            let p: PersonaSetParams = params(request.params)?;
            let name = core.config()?.defaults.persona;
            let path = core
                .persona_file(&name)
                .ok_or_else(|| RpcError::app("this core has no folder for personas"))?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(app)?;
            }
            std::fs::write(&path, p.text).map_err(app)?;
            result(Value::Null)
        }
        method::PERSONA_RESET => {
            let name = core.config()?.defaults.persona;
            if let Some(path) = core.persona_file(&name) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(app(err)),
                }
            }
            result(Value::Null)
        }
        method::SETUP_PRESETS => result(crate::setup::presets()),
        method::MEMORY_GET => result(core.memory.view()),
        method::MEMORY_SET => {
            let p: MemorySetParams = params(request.params)?;
            core.memory.set_text(&p.text).map_err(RpcError::app)?;
            result(core.memory.view())
        }
        method::IMAGES_GET => {
            let p: ImageParams = params(request.params)?;
            result(crate::pictures::load(&core.images_dir(), &p.id).map_err(RpcError::app)?)
        }
        method::MCP_LOGIN => {
            let p: KeyParams = params(request.params)?;
            let config = core.config()?;
            let server = config
                .mcp
                .servers
                .get(&p.name)
                .ok_or_else(|| RpcError::app(format!("no MCP server named `{}`", p.name)))?;
            let store = core
                .options
                .token_store
                .clone()
                .ok_or_else(|| RpcError::app("this core keeps no sign-ins"))?;
            crate::oauth::login(&p.name, server, store, crate::oauth::open_browser)
                .await
                .map_err(RpcError::app)?;
            result(Value::Null)
        }
        method::PROFILES_LIST => {
            let config = core.config()?;
            result(
                config
                    .profiles
                    .iter()
                    .map(|(name, profile)| ProfileInfo {
                        name: name.clone(),
                        provider: match profile.provider {
                            ProviderKind::OpenAiCompat => "openai_compat".into(),
                            ProviderKind::Anthropic => "anthropic".into(),
                        },
                        model: profile.model.clone(),
                        is_default: *name == config.defaults.profile,
                    })
                    .collect::<Vec<_>>(),
            )
        }
        method::TOOL_APPROVE => {
            let p: ApproveParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            let waiting = core
                .approvals
                .lock()
                .unwrap()
                .remove(&(key, p.call_id.clone()));
            match waiting {
                Some(tx) => {
                    let _ = tx.send(p.decision);
                    result(Value::Null)
                }
                None => Err(RpcError::app(format!(
                    "no tool call `{}` is waiting for approval (already answered?)",
                    p.call_id
                ))),
            }
        }
        method::TOOLS_LIST => {
            let p: ToolsListParams = params(request.params)?;
            let config = core.config()?;
            // With a session: its "always" approvals, mode and level.
            let (allowed, access) = match &p.session_id {
                Some(prefix) => {
                    let key = core.resolve_key(prefix)?;
                    let mut settings = core.settings(&key, &config)?;
                    cap_remote_level(&mut settings, &config, state.lock().unwrap().remote);
                    let allowed = core
                        .overrides
                        .lock()
                        .unwrap()
                        .get(&key)
                        .map(|o| o.allowed_tools.clone())
                        .unwrap_or_default();
                    let access = Access {
                        mode: settings.mode,
                        level: settings.level,
                        workspace: config.agent.workspace.as_deref().map(expand_home),
                    };
                    (allowed, access)
                }
                None => (HashSet::new(), Access::default()),
            };
            let cwd = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
            let (external, failed) = core
                .mcp
                .servers(&config.mcp.servers, &core.options.secrets)
                .await;
            let toolbox = ToolBox::open(&config.tools, cwd, external)
                .await
                .map_err(app)?
                .with_access(access)
                .with_generator(core.generator(&config));
            let tools = toolbox.infos(&allowed);
            toolbox.close().await;
            result(ToolsList {
                tools,
                problems: failed
                    .into_iter()
                    .map(|(server, error)| ServerProblem { server, error })
                    .collect(),
            })
        }
        other => Err(RpcError::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method `{other}`"),
        )),
    }
}

fn app(err: impl std::fmt::Display) -> RpcError {
    RpcError::app(err.to_string())
}

impl Core {
    fn config(&self) -> Result<Config, RpcError> {
        Config::load(&self.options.config_path)
            .map_err(app)?
            .ok_or_else(|| {
                RpcError::app(format!(
                    "no config at {}; run `lucy config init`",
                    self.options.config_path.display()
                ))
            })
    }

    /// Where pictures you attach are kept: `images/` next to the chats database.
    fn images_dir(&self) -> PathBuf {
        images_dir(&self.options.db_path)
    }

    /// Refuse while she's replying in this chat.
    fn not_busy(&self, key: &str) -> Result<(), RpcError> {
        if self.turns.lock().unwrap().contains_key(key) {
            return Err(RpcError::app(
                "Lucy is still replying in this chat; wait or stop her first",
            ));
        }
        Ok(())
    }

    /// Delete an incognito chat, its pictures with it (unless another chat has them too).
    fn forget_incognito(&self, key: &str) {
        if let Some(token) = self.turns.lock().unwrap().get(key) {
            token.cancel();
        }
        let Ok(id) = key.parse::<SessionId>() else {
            return;
        };
        let deleted = self.store.lock().unwrap().delete_with_pictures(&id);
        if let Ok(pictures) = deleted {
            remove_pictures(&self.images_dir(), &pictures);
        }
    }

    /// The picture maker `config` sets up, for `generate_image`: ready, or why not.
    fn generator(&self, config: &Config) -> Option<GeneratorSlot> {
        match Generator::from_config(config, &self.options.secrets) {
            Ok(None) => None,
            Ok(Some(generator)) => Some(Ok(Arc::new(generator))),
            Err(err) => Some(Err(err)),
        }
    }

    /// Where your own version of persona `name` lives (whether or not it exists yet).
    fn persona_file(&self, name: &str) -> Option<PathBuf> {
        self.options
            .personas_dir
            .as_ref()
            .map(|dir| dir.join(format!("{name}.md")))
    }

    /// Chats with a reply in progress, with their titles.
    fn busy(&self) -> Vec<BusyChat> {
        let keys: HashSet<String> = self.turns.lock().unwrap().keys().cloned().collect();
        if keys.is_empty() {
            return Vec::new();
        }
        let titles: HashMap<String, Option<String>> = match self.store.lock().unwrap().list() {
            Ok(sessions) => sessions
                .into_iter()
                .map(|s| (s.id.to_string(), s.title))
                .collect(),
            Err(_) => HashMap::new(),
        };
        let mut busy: Vec<BusyChat> = keys
            .into_iter()
            .map(|key| BusyChat {
                title: titles.get(&key).cloned().flatten(),
                session_id: key,
            })
            .collect();
        busy.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        busy
    }

    /// The full ID of the session matching `prefix`.
    fn resolve_key(&self, prefix: &str) -> Result<String, RpcError> {
        let store = self.store.lock().unwrap();
        Ok(store.resolve(prefix).map_err(app)?.to_string())
    }

    /// The profile and plain flag in effect for a session.
    /// The settings in effect for a session: its own choices, else the config's defaults.
    /// Agent mode uses `agent_profile` unless the chat picked a profile itself.
    fn settings(&self, session: &str, config: &Config) -> Result<SessionSettings, RpcError> {
        let id: SessionId = session.parse().map_err(app)?;
        let own = self.store.lock().unwrap().settings(&id).map_err(app)?;
        let mode = own.mode.unwrap_or(config.defaults.mode);
        let default_profile = match mode {
            Mode::Agent => config
                .defaults
                .agent_profile
                .clone()
                .unwrap_or_else(|| config.defaults.profile.clone()),
            Mode::Assist => config.defaults.profile.clone(),
        };
        Ok(SessionSettings {
            profile: own
                .profile
                .filter(|p| config.profiles.contains_key(p))
                .unwrap_or(default_profile),
            plain: own.plain.unwrap_or(false),
            mode,
            level: own.level.unwrap_or(config.defaults.agent_level),
        })
    }

    fn update_settings(
        &self,
        prefix: &str,
        config: &Config,
        change: impl FnOnce(&mut ChatSettings),
    ) -> Result<Value, RpcError> {
        let key = self.resolve_key(prefix)?;
        let id: SessionId = key.parse().map_err(app)?;
        {
            let store = self.store.lock().unwrap();
            let mut own = store.settings(&id).map_err(app)?;
            change(&mut own);
            store.set_settings(&id, &own).map_err(app)?;
        }
        result(self.settings(&key, config)?)
    }

    /// List, and with `apply` restore, the files the most recent reply with changes touched.
    fn undo(&self, session: &str, apply: bool) -> Result<UndoResult, String> {
        let id: SessionId = session.parse().map_err(|err| format!("{err}"))?;
        let store = self.store.lock().unwrap();
        let backups = store.last_backups(&id).map_err(|err| err.to_string())?;
        let files = backups
            .iter()
            .map(|b| UndoFile {
                path: b.path.display().to_string(),
                created: b.before.is_none(),
            })
            .collect();
        if !apply || backups.is_empty() {
            return Ok(UndoResult {
                files,
                applied: false,
            });
        }
        let mut problems = Vec::new();
        for backup in &backups {
            let restored = match &backup.before {
                Some(content) => std::fs::write(&backup.path, content),
                None => match std::fs::remove_file(&backup.path) {
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            if let Err(err) = restored {
                problems.push(format!("{}: {err}", backup.path.display()));
            }
        }
        store
            .drop_backups(&id, &backups[0].reply)
            .map_err(|err| err.to_string())?;
        if problems.is_empty() {
            Ok(UndoResult {
                files,
                applied: true,
            })
        } else {
            Err(format!("couldn't restore {}", problems.join("; ")))
        }
    }
}

fn chat_send(
    core: &Arc<Core>,
    state: &Arc<Mutex<ConnState>>,
    p: ChatSendParams,
) -> Result<Value, RpcError> {
    let config = core.config()?;
    let (session_id, incognito) = {
        let store = core.store.lock().unwrap();
        let id = match &p.session_id {
            Some(prefix) => store.resolve(prefix),
            None if p.incognito => store.create_incognito(),
            None => store.create(),
        }
        .map_err(app)?;
        let incognito = store.is_incognito(&id).map_err(app)?;
        (id, incognito)
    };
    let key = session_id.to_string();
    if incognito && p.session_id.is_none() {
        state.lock().unwrap().incognito.insert(key.clone());
    }
    let remote = state.lock().unwrap().remote;
    // A remote client's folder is on another machine; its chats work in the core's.
    let cwd = match remote {
        true => remote_folder(&config),
        false => working_folder(p.cwd.as_deref()),
    };
    let turn_id = start_turn(
        core,
        TurnRequest {
            session_id,
            text: p.text,
            images: p.images,
            cwd,
            remote,
            incognito,
            task: None,
        },
    )?;
    state.lock().unwrap().own_sessions.insert(key.clone());
    result(ChatSendResult {
        session_id: key,
        turn_id,
    })
}

/// What starts a reply: a message in a chat, from a client or a scheduled task.
struct TurnRequest {
    session_id: SessionId,
    text: String,
    images: Vec<rpc::ImageData>,
    /// The working folder its tools start in.
    cwd: PathBuf,
    /// From another machine: the agent level is capped.
    remote: bool,
    incognito: bool,
    /// A scheduled task, running at its own level and model with nobody there to ask.
    task: Option<TaskRun>,
}

/// A scheduled task's turn.
#[derive(Clone)]
struct TaskRun {
    title: String,
    profile: String,
    level: AgentLevel,
}

/// Save the message, then reply to it in the background. Returns the turn's ID.
fn start_turn(core: &Arc<Core>, request: TurnRequest) -> Result<u64, RpcError> {
    let TurnRequest {
        session_id,
        text,
        images,
        cwd,
        remote,
        incognito,
        task,
    } = request;
    let config = core.config()?;
    let key = session_id.to_string();
    // Memory: on unless switched off, never in an incognito chat.
    let memory = (config.memory.enabled && !incognito).then(|| Arc::clone(&core.memory));
    let mut settings = core.settings(&key, &config)?;
    cap_remote_level(&mut settings, &config, remote);
    if let Some(task) = &task {
        if !config.profiles.contains_key(&task.profile) {
            return Err(RpcError::app(format!(
                "the task's profile `{}` no longer exists",
                task.profile
            )));
        }
        settings.profile = task.profile.clone();
        settings.mode = Mode::Agent;
        settings.level = task.level;
    }
    let profile = &config.profiles[&settings.profile];
    let provider =
        build_provider(&settings.profile, profile, &core.options.secrets).map_err(app)?;
    let persona = core.personas.load(&config.defaults.persona).map_err(app)?;
    let origin = Origin {
        profile: settings.profile.clone(),
        model: profile.model.clone(),
    };

    let cancel = CancellationToken::new();
    {
        let mut turns = core.turns.lock().unwrap();
        if core.shutdown.is_cancelled() {
            return Err(RpcError::app("Lucy's core is shutting down; try again"));
        }
        if turns.contains_key(&key) {
            return Err(RpcError::app(
                "Lucy is still replying in this session; wait or cancel first",
            ));
        }
        turns.insert(key.clone(), cancel.clone());
    }

    let images_dir = core.images_dir();
    let pictures: Result<Vec<_>, _> = images
        .iter()
        .map(|picture| crate::pictures::save(&images_dir, picture))
        .collect();
    let pictures = match pictures {
        Ok(pictures) => pictures,
        Err(err) => {
            core.turns.lock().unwrap().remove(&key);
            return Err(RpcError::app(err));
        }
    };
    let history = {
        let store = core.store.lock().unwrap();
        store
            .append(
                &session_id,
                NewMessage::user_with_images(text.clone(), pictures.clone()),
            )
            .and_then(|()| store.load(&session_id))
    };
    let history = match history {
        Ok(session) => session.messages,
        Err(err) => {
            core.turns.lock().unwrap().remove(&key);
            return Err(app(err));
        }
    };
    // What the chat is about, for picking memories: your last few messages.
    let memory_plan = memory.clone().map(|memory| {
        let recent: Vec<&str> = history
            .iter()
            .rev()
            .filter(|m| m.message.role == Role::User)
            .take(3)
            .map(|m| m.message.content.as_str())
            .collect();
        let embedder = config.memory.embedding_profile.as_ref().and_then(|name| {
            crate::provider::build_embedder(name, &config.profiles[name], &core.options.secrets)
                .map_err(|err| eprintln!("memory: no embeddings: {err}"))
                .ok()
        });
        MemoryPlan {
            memory,
            query: recent.join("\n"),
            embedder,
        }
    });
    let _ = core.events.send(Event::UserMessage {
        session_id: key.clone(),
        text,
        images: pictures,
    });

    let agent = settings.mode == Mode::Agent;
    let budget = if agent {
        Budget {
            agent: true,
            max_steps: config.agent.max_steps.max(1),
            max_cost: Some(config.agent.max_cost_usd),
            max_tokens: Some(config.agent.max_tokens),
            prices: profile.prices(),
        }
    } else {
        Budget {
            agent: false,
            max_steps: config.tools.max_rounds,
            max_cost: None,
            max_tokens: None,
            prices: profile.prices(),
        }
    };
    let agent_note = agent.then(|| agent_note(&settings, &config, &budget));
    let system = build_system_prompt(
        &persona,
        &PromptContext {
            plain: settings.plain,
            now: chrono::Local::now().fixed_offset(),
            machine: Some(core.machine.clone()),
            user_name: config.user.name.clone(),
            origin: Some(origin.clone()),
            cwd: Some(cwd.display().to_string()),
            agent: agent_note,
        },
    );
    // Pictures: sent as they are to a model that can see; otherwise described by the vision
    // helper (if there is one), the first time during the reply.
    let helper = match (&profile.vision, &config.images.vision_profile) {
        (false, Some(name)) => {
            let helper_profile = &config.profiles[name];
            Some(build_provider(name, helper_profile, &core.options.secrets).map_err(app)?)
        }
        _ => None,
    };
    let sight = match (profile.vision, helper.is_some()) {
        (true, _) => crate::pictures::Sight::Sees,
        (false, true) => crate::pictures::Sight::Helped,
        (false, false) => crate::pictures::Sight::Blind,
    };
    // What the model gets: after the last /clear, with a summary for what it covers.
    let visible = crate::context::visible(history);
    let mut pending = Vec::new();
    let history = visible
        .messages
        .into_iter()
        .map(|m| crate::pictures::attach(&images_dir, m.message, &m.images, &sight, &mut pending))
        .collect();
    let summary = visible
        .summary
        .as_deref()
        .map(crate::context::summary_message);
    let messages: Vec<Message> = std::iter::once(Message::new(Role::System, system))
        .chain(summary.clone())
        .chain(close_dangling_calls(history))
        .collect();
    let context = ContextPlan {
        limit: profile.context_tokens,
        model: (
            profile.provider,
            profile.base_url.clone().unwrap_or_default(),
            crate::provider::profile_key(&settings.profile, profile, &core.options.secrets)
                .ok()
                .flatten(),
            profile.model.clone(),
        ),
        first: visible.first,
        history_at: 1 + usize::from(summary.is_some()),
        summary: visible.summary,
    };

    let turn_id = core.next_turn.fetch_add(1, Ordering::Relaxed);
    core.activity.add(&core.activity.turns, 1);
    let turn = Turn {
        core: Arc::clone(core),
        session_id,
        key: key.clone(),
        id: turn_id,
        origin,
        reply: new_reply_id(),
        task,
    };
    let access = Access {
        mode: settings.mode,
        level: settings.level,
        workspace: config.agent.workspace.as_deref().map(expand_home),
    };
    tokio::spawn(turn.run(Plan {
        provider,
        messages,
        tools: profile.tools.then(|| ToolPlan {
            settings: config.tools.clone(),
            servers: config.mcp.servers.clone(),
            cwd,
            access,
            generator: core.generator(&config),
            memory,
            schedules: Some(Arc::new(schedules::Book {
                core: Arc::clone(core),
                profile: settings.profile.clone(),
            }) as Arc<dyn crate::tools::ScheduleBook>),
        }),
        budget,
        cancel,
        memory: memory_plan,
        context,
        pictures: Pictures {
            dir: images_dir,
            sight,
            helper,
            pending,
        },
    }));
    Ok(turn_id)
}

/// `/image`: make a picture straight away, without asking (you typed it) and without a model
/// turn. Returns at once; the picture arrives as a `generate_image` result.
fn images_generate(
    core: &Arc<Core>,
    state: &Arc<Mutex<ConnState>>,
    p: ImageGenerateParams,
) -> Result<Value, RpcError> {
    let prompt = p.prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err(RpcError::app(
            "say what to draw, like /image a lighthouse at dusk",
        ));
    }
    let config = core.config()?;
    let generator = Generator::from_config(&config, &core.options.secrets)
        .map_err(RpcError::app)?
        .ok_or_else(|| {
            RpcError::app(
                "nothing is set up to make pictures yet: choose an image generator in Settings \
                 ([images] generator: an image model profile, or \"comfyui\")",
            )
        })?;
    let session_id = {
        let store = core.store.lock().unwrap();
        match &p.session_id {
            Some(prefix) => store.resolve(prefix),
            None => store.create(),
        }
        .map_err(app)?
    };
    let key = session_id.to_string();
    let settings = core.settings(&key, &config)?;
    let origin = Origin {
        profile: settings.profile.clone(),
        model: config.profiles[&settings.profile].model.clone(),
    };

    let cancel = CancellationToken::new();
    {
        let mut turns = core.turns.lock().unwrap();
        if core.shutdown.is_cancelled() {
            return Err(RpcError::app("Lucy's core is shutting down; try again"));
        }
        if turns.contains_key(&key) {
            return Err(RpcError::app(
                "Lucy is still replying in this session; wait or cancel first",
            ));
        }
        turns.insert(key.clone(), cancel.clone());
    }
    let text = format!("/image {prompt}");
    let saved = core
        .store
        .lock()
        .unwrap()
        .append(&session_id, NewMessage::user(text.clone()));
    if let Err(err) = saved {
        core.turns.lock().unwrap().remove(&key);
        return Err(app(err));
    }
    let _ = core.events.send(Event::UserMessage {
        session_id: key.clone(),
        text,
        images: Vec::new(),
    });

    let turn_id = core.next_turn.fetch_add(1, Ordering::Relaxed);
    state.lock().unwrap().own_sessions.insert(key.clone());
    core.activity.add(&core.activity.turns, 1);
    let turn = Turn {
        core: Arc::clone(core),
        session_id,
        key: key.clone(),
        id: turn_id,
        origin,
        reply: new_reply_id(),
        task: None,
    };
    tokio::spawn(turn.image(generator, prompt, cancel));
    result(ChatSendResult {
        session_id: key,
        turn_id,
    })
}

/// What the model is told about agent mode: how far it may go, and its limits.
fn agent_note(settings: &SessionSettings, config: &Config, budget: &Budget) -> String {
    let level = match settings.level {
        AgentLevel::Supervised => "0, Supervised: the user approves every tool call",
        AgentLevel::Guarded => {
            "1, Guarded: reads and file changes in the working folder run without asking; \
             commands, going online and anything outside the folder need the user's approval"
        }
        AgentLevel::Workspace => {
            "2, Workspace: anything local inside the workspace runs without asking; going \
             online and anything outside it need approval"
        }
        AgentLevel::Unrestricted => {
            "3, Unrestricted: tools run without asking, except on private files"
        }
    };
    let workspace = match (&config.agent.workspace, settings.level) {
        (Some(workspace), AgentLevel::Workspace) => format!(" Workspace: {}.", workspace.display()),
        _ => String::new(),
    };
    let money = match budget.prices {
        Some((i, o)) if i > 0.0 || o > 0.0 => {
            format!(" and about ${:.2}", budget.max_cost.unwrap_or_default())
        }
        Some(_) => String::new(),
        None => format!(" and {} tokens", budget.max_tokens.unwrap_or_default()),
    };
    format!(
        "Agent mode (level {level}).{workspace} Work through the task yourself with tools, step \
         by step, without asking the user for permission in chat; when it's done, report what \
         you did. You have {} steps{money} for this task.",
        budget.max_steps
    )
}

/// `~/x` as an absolute path.
pub(crate) fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), dirs::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_owned(),
    }
}

/// The folder a client says it's working in, if it exists; otherwise home.
/// Where remote clients' chats use tools: `[agent] workspace` if it exists, else home.
fn remote_folder(config: &Config) -> PathBuf {
    config
        .agent
        .workspace
        .as_deref()
        .map(expand_home)
        .filter(|dir| dir.is_dir())
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// Chats driven from a remote client run at most at `[remote] max_level`.
fn cap_remote_level(settings: &mut SessionSettings, config: &Config, remote: bool) {
    if remote {
        settings.level = settings.level.min(config.remote.max_level);
    }
}

fn working_folder(cwd: Option<&str>) -> PathBuf {
    cwd.map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// One reply in progress: model steps, with tool calls in between.
struct Turn {
    core: Arc<Core>,
    session_id: SessionId,
    key: String,
    id: u64,
    origin: Origin,
    /// Groups this reply's file backups for `/undo`.
    reply: String,
    /// A scheduled task: nobody is there to approve calls, and its result is announced.
    task: Option<TaskRun>,
}

struct Plan {
    provider: Box<dyn Provider>,
    messages: Vec<Message>,
    /// Tool settings and working folder, when this profile uses tools.
    tools: Option<ToolPlan>,
    budget: Budget,
    cancel: CancellationToken,
    pictures: Pictures,
    /// Her memory, to add to the system prompt before the first step.
    memory: Option<MemoryPlan>,
    /// Keeping the chat within the model's context window.
    context: ContextPlan,
}

/// How big a reply's prompt may get, and what to summarise if it's too big.
struct ContextPlan {
    /// `context_tokens`, or `None` to learn it from the provider.
    limit: Option<u32>,
    /// Where to ask: provider, base URL, key and model.
    model: (crate::config::ProviderKind, String, Option<String>, String),
    /// The chat's index of the first message after the system prompt (and summary).
    first: usize,
    /// Where the chat's messages start in the prompt.
    history_at: usize,
    summary: Option<String>,
}

/// Which memories go into this reply's prompt.
struct MemoryPlan {
    memory: Arc<crate::memory::Memory>,
    /// Recent messages, to pick relevant facts by.
    query: String,
    embedder: Option<lucy_providers::embed::Embedder>,
}

/// How this reply handles pictures, from you or from tools.
struct Pictures {
    dir: PathBuf,
    sight: crate::pictures::Sight,
    /// The vision helper, for a model that can't see.
    helper: Option<Box<dyn Provider>>,
    /// Pictures in the history the helper still has to describe.
    pending: Vec<rpc::ImageRef>,
}

struct ToolPlan {
    settings: ToolSettings,
    servers: BTreeMap<String, McpServer>,
    /// The user's working folder.
    cwd: PathBuf,
    access: Access,
    generator: Option<GeneratorSlot>,
    memory: Option<Arc<crate::memory::Memory>>,
    schedules: Option<Arc<dyn crate::tools::ScheduleBook>>,
}

/// How far one reply may go.
struct Budget {
    agent: bool,
    /// Tool rounds (Assist) or model steps (Agent).
    max_steps: u32,
    max_cost: Option<f64>,
    /// Used instead of the cost cap when the model's prices are unknown.
    max_tokens: Option<u64>,
    /// USD per million tokens (in, out), if known.
    prices: Option<(f64, f64)>,
}

impl Budget {
    fn cost(&self, usage: &Usage) -> Option<f64> {
        self.prices.map(|(i, o)| usage.cost_usd(i, o))
    }

    /// Which agent cap, if any, `progress` has reached.
    fn reached(&self, progress: &Progress) -> Option<String> {
        if !self.agent {
            return None;
        }
        if progress.steps >= self.max_steps {
            return Some(format!("step cap ({} steps)", self.max_steps));
        }
        match (self.cost(&progress.usage), self.max_cost, self.max_tokens) {
            (Some(cost), Some(max), _) if cost >= max => {
                Some(format!("cost cap (${cost:.2} of ${max:.2})"))
            }
            (None, _, Some(max)) if progress.usage.total() >= max => Some(format!(
                "token cap ({} of {max} tokens)",
                progress.usage.total()
            )),
            _ => None,
        }
    }
}

/// What a reply has done so far.
#[derive(Default)]
struct Progress {
    /// The text of the model step in progress; finished steps are saved as they end.
    text: String,
    steps: u32,
    usage: Usage,
}

/// How a reply ended, short of failing.
enum Ending {
    Complete,
    /// Hit an agent cap (and summarised).
    Capped(String),
}

impl Turn {
    fn emit(&self, event: Event) {
        let _ = self.core.events.send(event);
    }

    /// The session may have been deleted mid-reply; there's nothing to save to then.
    fn save(&self, message: NewMessage) {
        let _ = self
            .core
            .store
            .lock()
            .unwrap()
            .append(&self.session_id, message);
    }

    async fn run(self, plan: Plan) {
        self.emit(Event::TurnStarted {
            session_id: self.key.clone(),
            turn_id: self.id,
            profile: self.origin.profile.clone(),
            model: self.origin.model.clone(),
        });

        let toolbox = match plan.tools {
            Some(tools) => match self.open_tools(&tools).await {
                Ok(toolbox) => Some(
                    toolbox
                        .with_access(tools.access)
                        .with_generator(tools.generator)
                        .with_memory(tools.memory)
                        .with_schedules(tools.schedules),
                ),
                Err(err) => {
                    eprintln!("tools unavailable for this reply: {err}");
                    None
                }
            },
            None => None,
        };

        let mut messages = plan.messages;
        if let Some(memory) = &plan.memory
            && let Some(section) = memory
                .memory
                .for_prompt(&memory.query, memory.embedder.as_ref())
                .await
            && let Some(system) = messages.first_mut()
            && system.role == Role::System
        {
            system.content = format!("{}\n\n{section}", system.content);
        }
        // Too big for the model: summarise older turns first. Small prompts fit any model, so
        // there's no need to learn its context size for them.
        let size = crate::context::estimate(&messages);
        let worth_checking = plan.context.limit.is_some() || size > crate::context::FITS_ANY;
        if worth_checking
            && let Some(limit) = self.context_limit(&plan.context).await
            && size > (limit as f32 * crate::context::SUMMARISE_AT) as usize
        {
            messages = self
                .compress(&*plan.provider, messages, &plan.context, limit)
                .await;
        }
        let pictures = &plan.pictures;
        if let Some(helper) = &pictures.helper {
            crate::pictures::describe_pending(
                &pictures.dir,
                &**helper,
                &pictures.pending,
                &mut messages,
            )
            .await;
        }

        let mut progress = Progress::default();
        let steps = self.steps(
            &*plan.provider,
            messages,
            toolbox.as_ref(),
            &plan.budget,
            pictures,
            &mut progress,
        );
        let (outcome, error) = tokio::select! {
            _ = plan.cancel.cancelled() => (TurnOutcome::Cancelled, None),
            steps = steps => match steps {
                Ok(Ending::Complete) => (TurnOutcome::Complete, None),
                Ok(Ending::Capped(cap)) => (TurnOutcome::Capped, Some(format!("stopped at the {cap}"))),
                Err(err) => (TurnOutcome::Failed, Some(err)),
            },
        };
        // Keep whatever arrived; a cut-off reply is saved and marked incomplete.
        if outcome == TurnOutcome::Cancelled || outcome == TurnOutcome::Failed {
            let text = std::mem::take(&mut progress.text);
            if !text.is_empty() {
                self.save(NewMessage::assistant(text, self.origin.clone()).incomplete());
            }
        }

        self.core
            .approvals
            .lock()
            .unwrap()
            .retain(|(session, _), _| *session != self.key);
        if let Some(toolbox) = toolbox {
            toolbox.close().await;
        }
        let usage = TurnUsage {
            steps: progress.steps,
            input_tokens: progress.usage.input_tokens,
            output_tokens: progress.usage.output_tokens,
            cost_usd: plan.budget.cost(&progress.usage),
        };
        // Free the session before announcing, so a client can send its next message right away.
        self.core.turns.lock().unwrap().remove(&self.key);
        // A scheduled task tells you how it went: its last words, or why it stopped.
        if let Some(task) = &self.task {
            let said = self
                .core
                .store
                .lock()
                .unwrap()
                .load(&self.session_id)
                .ok()
                .and_then(|session| {
                    session
                        .messages
                        .into_iter()
                        .rev()
                        .find(|m| {
                            m.message.role == Role::Assistant && !m.message.content.is_empty()
                        })
                        .map(|m| m.message.content)
                });
            let body = match (&error, said) {
                (Some(error), _) => format!("Stopped: {error}"),
                (None, Some(said)) => schedules::task_summary(&said),
                (None, None) => "Done.".into(),
            };
            self.core.notify(&self.key, &task.title, &body);
        }
        self.emit(Event::TurnFinished {
            session_id: self.key.clone(),
            turn_id: self.id,
            outcome,
            error,
            usage: Some(usage),
        });
        self.core.activity.add(&self.core.activity.turns, -1);
    }

    /// The built-ins plus every configured MCP server that starts; failures are logged.
    async fn open_tools(&self, plan: &ToolPlan) -> Result<ToolBox, String> {
        let (external, failed) = self
            .core
            .mcp
            .servers(&plan.servers, &self.core.options.secrets)
            .await;
        for (server, err) in failed {
            eprintln!("MCP server `{server}` unavailable: {err}");
        }
        ToolBox::open(&plan.settings, plan.cwd.clone(), external).await
    }

    /// Ask the model, run any tools it calls, and repeat until it answers without tools, or
    /// (in agent mode) until a cap, when it gets one last turn to sum up.
    async fn steps(
        &self,
        provider: &dyn Provider,
        mut messages: Vec<Message>,
        toolbox: Option<&ToolBox>,
        budget: &Budget,
        pictures: &Pictures,
        progress: &mut Progress,
    ) -> Result<Ending, String> {
        let specs = toolbox.map(ToolBox::specs).unwrap_or_default();
        let max_rounds = budget.max_steps;
        let mut round = 0;
        loop {
            let calls = self
                .step(provider, &messages, &specs, progress)
                .await
                .map_err(|err| err.to_string())?;
            let said = std::mem::take(&mut progress.text);
            let Some(toolbox) = toolbox.filter(|_| !calls.is_empty()) else {
                self.save(NewMessage::assistant(said, self.origin.clone()));
                return Ok(Ending::Complete);
            };

            round += 1;
            self.save(NewMessage::assistant_with_tools(
                said.clone(),
                calls.clone(),
                self.origin.clone(),
            ));
            messages.push(Message::assistant_with_tools(said, calls.clone()));
            if !budget.agent && round > max_rounds + 1 {
                return Err(format!(
                    "Lucy kept calling tools after the limit of {max_rounds} rounds"
                ));
            }
            for call in &calls {
                let output = if !budget.agent && round > max_rounds {
                    let refusal = format!(
                        "Tool limit reached ({max_rounds} rounds for one message). Answer with \
                         what you have, and say what's left to do."
                    );
                    self.announce(call, refusal)
                } else {
                    self.run_call(toolbox, call).await
                };
                let message = self.tool_result(call, output, pictures).await;
                messages.push(message);
            }

            if let Some(cap) = budget.reached(progress) {
                self.wrap_up(provider, messages, &specs, &cap, pictures, progress)
                    .await?;
                return Ok(Ending::Capped(cap));
            }
        }
    }

    /// At a cap: one more step, asking for a summary instead of more tool calls. Tools stay
    /// offered (some providers require them once tool calls are in the history); any calls
    /// it makes anyway are refused.
    async fn wrap_up(
        &self,
        provider: &dyn Provider,
        mut messages: Vec<Message>,
        specs: &[lucy_providers::ToolSpec],
        cap: &str,
        pictures: &Pictures,
        progress: &mut Progress,
    ) -> Result<(), String> {
        messages.push(Message::new(
            Role::User,
            format!(
                "[Lucy's core: this task reached its {cap}. Don't call any more tools. \
                 Summarise what you've done and what's left, so the user can decide whether to \
                 continue.]"
            ),
        ));
        let calls = self
            .step(provider, &messages, specs, progress)
            .await
            .map_err(|err| err.to_string())?;
        let said = std::mem::take(&mut progress.text);
        if calls.is_empty() {
            self.save(NewMessage::assistant(said, self.origin.clone()));
            return Ok(());
        }
        self.save(NewMessage::assistant_with_tools(
            said,
            calls.clone(),
            self.origin.clone(),
        ));
        for call in &calls {
            let refusal = format!("Not run: the task reached its {cap}.");
            let output = self.announce(call, refusal);
            self.tool_result(call, output, pictures).await;
        }
        Ok(())
    }

    /// Save a call's result with any pictures it gave, announce it, and return it as the
    /// model gets it: with the pictures for a model that can see, described by the helper
    /// otherwise.
    async fn tool_result(
        &self,
        call: &ToolCall,
        mut output: ToolOutput,
        pictures: &Pictures,
    ) -> Message {
        let mut kept = Vec::new();
        for image in &output.images {
            match crate::pictures::save(&pictures.dir, image) {
                Ok(image) => kept.push(rpc::ImageRef {
                    made: output.made,
                    ..image
                }),
                Err(err) => output.text += &format!("\n[A picture that couldn't be kept: {err}]"),
            }
        }
        self.save(
            NewMessage::tool_result(&call.id, &output.text, output.is_error)
                .with_images(kept.clone()),
        );
        self.emit(Event::ToolFinished {
            session_id: self.key.clone(),
            turn_id: self.id,
            call_id: call.id.clone(),
            output: output.text.clone(),
            is_error: output.is_error,
            images: kept.clone(),
        });
        let mut pending = Vec::new();
        let message = crate::pictures::attach(
            &pictures.dir,
            Message::tool_result(&call.id, output.text, output.is_error),
            &kept,
            &pictures.sight,
            &mut pending,
        );
        let mut messages = [message];
        if let Some(helper) = &pictures.helper {
            crate::pictures::describe_pending(&pictures.dir, &**helper, &pending, &mut messages)
                .await;
        }
        let [message] = messages;
        message
    }

    /// One model response: streams its text, counts it, and returns the tool calls it made.
    async fn step(
        &self,
        provider: &dyn Provider,
        messages: &[Message],
        specs: &[lucy_providers::ToolSpec],
        progress: &mut Progress,
    ) -> Result<Vec<ToolCall>, ProviderError> {
        progress.text.clear();
        progress.steps += 1;
        let request = ChatRequest {
            messages: messages.to_vec(),
            tools: specs.to_vec(),
        };
        let mut calls = Vec::new();
        let mut stream = provider.stream(request).await?;
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(delta) => {
                    progress.text.push_str(&delta);
                    self.emit(Event::Delta {
                        session_id: self.key.clone(),
                        turn_id: self.id,
                        text: delta,
                    });
                }
                StreamEvent::ToolCall(call) => calls.push(call),
                StreamEvent::Usage(usage) => progress.usage += usage,
                StreamEvent::Done => return Ok(calls),
            }
        }
        Err(ProviderError::Protocol(
            "stream ended without finishing".into(),
        ))
    }

    /// Check permissions (asking the user if needed), then run the call.
    async fn run_call(&self, toolbox: &ToolBox, call: &ToolCall) -> ToolOutput {
        let allowed = self
            .core
            .overrides
            .lock()
            .unwrap()
            .get(&self.key)
            .map(|o| o.allowed_tools.clone())
            .unwrap_or_default();
        let go_ahead = match toolbox.verdict(call, &allowed) {
            Verdict::Run => Ok(()),
            Verdict::Refuse(reason) => Err(reason),
            // A scheduled task runs with nobody there: anything that would ask is refused.
            Verdict::Ask { reason, .. } if self.task.is_some() => Err(format!(
                "Not run: this needs the user's approval ({reason}), and this task ran while \
                 they were away. Carry on without it, and say what you couldn't do; they can \
                 raise the task's level to allow it."
            )),
            Verdict::Ask { reason, preview } => match self.approval(call, reason, preview).await {
                Some(Decision::Once) => Ok(()),
                Some(Decision::Always) => {
                    self.core
                        .overrides
                        .lock()
                        .unwrap()
                        .entry(self.key.clone())
                        .or_default()
                        .allowed_tools
                        .insert(call.name.clone());
                    Ok(())
                }
                Some(Decision::Deny) => Err(
                    "The user declined this tool call. Don't try it again unless they ask.".into(),
                ),
                None => Err(format!(
                    "Nobody answered the approval prompt within {} minutes, so this didn't run.",
                    APPROVAL_TIMEOUT.as_secs() / 60
                )),
            },
        };
        match go_ahead {
            Ok(()) => {
                self.emit(Event::ToolStarted {
                    session_id: self.key.clone(),
                    turn_id: self.id,
                    call: call.clone(),
                });
                self.back_up(toolbox, call);
                toolbox.call(call).await
            }
            Err(reason) => self.announce(call, reason),
        }
    }

    /// Keep what files looked like before a built-in write or edit, for `/undo`.
    fn back_up(&self, toolbox: &ToolBox, call: &ToolCall) {
        const MAX_BACKUP: u64 = 10 * 1024 * 1024;
        for path in toolbox.changed_files(call) {
            let before = match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > MAX_BACKUP => continue,
                Ok(_) => match std::fs::read(&path) {
                    Ok(bytes) => Some(bytes),
                    Err(_) => continue,
                },
                Err(_) => None,
            };
            let _ = self.core.store.lock().unwrap().record_backup(
                &self.session_id,
                &self.reply,
                &path,
                before.as_deref(),
            );
        }
    }

    /// Wait for a client to approve or decline; `None` if nobody answers in time.
    async fn approval(
        &self,
        call: &ToolCall,
        reason: String,
        preview: Option<String>,
    ) -> Option<Decision> {
        let (tx, rx) = oneshot::channel();
        self.core
            .approvals
            .lock()
            .unwrap()
            .insert((self.key.clone(), call.id.clone()), tx);
        self.emit(Event::ToolApproval {
            session_id: self.key.clone(),
            turn_id: self.id,
            call: call.clone(),
            reason,
            preview,
        });
        let decision = tokio::time::timeout(APPROVAL_TIMEOUT, rx).await;
        self.core
            .approvals
            .lock()
            .unwrap()
            .remove(&(self.key.clone(), call.id.clone()));
        decision.ok().and_then(Result::ok)
    }

    /// The model's context size: configured, learned before, or asked of the provider now.
    async fn context_limit(&self, context: &ContextPlan) -> Option<u32> {
        if let Some(limit) = context.limit {
            return Some(limit);
        }
        let (kind, base_url, key, model) = &context.model;
        let cache_key = format!("{kind:?} {base_url} {model}");
        if let Some(known) = self.core.context_limits.lock().unwrap().get(&cache_key) {
            return *known;
        }
        let kind = match kind {
            crate::config::ProviderKind::Anthropic => lucy_providers::models::Kind::Anthropic,
            crate::config::ProviderKind::OpenAiCompat => lucy_providers::models::Kind::OpenAiCompat,
        };
        let asked = tokio::time::timeout(
            Duration::from_secs(5),
            lucy_providers::models::describe_model(kind, base_url, key.as_deref(), model),
        )
        .await;
        let limit = match asked {
            Ok(Ok(info)) => info.context_tokens,
            _ => None,
        };
        self.core
            .context_limits
            .lock()
            .unwrap()
            .insert(cache_key, limit);
        limit
    }

    /// Replace older turns with a summary (saved with the chat), keeping recent ones word for
    /// word. Returns the prompt unchanged if there's nothing older to summarise.
    async fn compress(
        &self,
        provider: &dyn Provider,
        messages: Vec<Message>,
        context: &ContextPlan,
        limit: u32,
    ) -> Vec<Message> {
        let at = context.history_at.min(messages.len());
        let history = &messages[at..];
        let keep = (limit as f32 * crate::context::KEEP) as usize;
        let cut = crate::context::cut(history, keep);
        if cut == 0 {
            return messages;
        }
        match crate::context::summarise(provider, context.summary.as_deref(), &history[..cut]).await
        {
            Ok(summary) => {
                self.save(NewMessage::summary(
                    summary.clone(),
                    (context.first + cut) as u32,
                ));
                std::iter::once(messages[0].clone())
                    .chain(std::iter::once(crate::context::summary_message(&summary)))
                    .chain(history[cut..].iter().cloned())
                    .collect()
            }
            Err(err) => {
                eprintln!("couldn't summarise the chat: {err}");
                messages
            }
        }
    }

    /// Report a call that didn't run, as if it ran and failed with `reason` (its result is
    /// announced with the others').
    fn announce(&self, call: &ToolCall, reason: String) -> ToolOutput {
        self.emit(Event::ToolStarted {
            session_id: self.key.clone(),
            turn_id: self.id,
            call: call.clone(),
        });
        ToolOutput::failed(reason)
    }

    /// `/image`: make a picture with no model in between, recorded as if she'd called
    /// `generate_image`, so it's kept and shown like one.
    async fn image(self, generator: Generator, prompt: String, cancel: CancellationToken) {
        self.emit(Event::TurnStarted {
            session_id: self.key.clone(),
            turn_id: self.id,
            profile: self.origin.profile.clone(),
            model: self.origin.model.clone(),
        });
        let call = ToolCall {
            id: format!("image_{}", self.id),
            name: crate::generate::TOOL.into(),
            arguments: serde_json::json!({"prompt": prompt}),
        };
        self.save(NewMessage::assistant_with_tools(
            "",
            vec![call.clone()],
            self.origin.clone(),
        ));
        self.emit(Event::ToolStarted {
            session_id: self.key.clone(),
            turn_id: self.id,
            call: call.clone(),
        });
        let request = lucy_providers::generate::ImageRequest {
            prompt,
            negative: None,
            width: crate::generate::DEFAULT_SIZE,
            height: crate::generate::DEFAULT_SIZE,
        };
        let output = tokio::select! {
            _ = cancel.cancelled() => ToolOutput::failed("Stopped before the picture was made."),
            output = generator.run(&request) => output,
        };
        let (outcome, error) = match (cancel.is_cancelled(), output.is_error) {
            (true, _) => (TurnOutcome::Cancelled, None),
            (false, true) => (TurnOutcome::Failed, Some(output.text.clone())),
            (false, false) => (TurnOutcome::Complete, None),
        };
        let pictures = Pictures {
            dir: self.core.images_dir(),
            sight: crate::pictures::Sight::Sees,
            helper: None,
            pending: Vec::new(),
        };
        self.tool_result(&call, output, &pictures).await;
        self.core.turns.lock().unwrap().remove(&self.key);
        self.emit(Event::TurnFinished {
            session_id: self.key.clone(),
            turn_id: self.id,
            outcome,
            error,
            usage: None,
        });
        self.core.activity.add(&self.core.activity.turns, -1);
    }
}

fn session_dto(session: crate::sessions::Session) -> rpc::Session {
    rpc::Session {
        summary: summary_dto(session.summary),
        messages: session
            .messages
            .into_iter()
            .map(|m| rpc::StoredMessage {
                role: m.message.role,
                content: m.message.content,
                tool_calls: m.message.tool_calls,
                tool_call_id: m.message.tool_call_id,
                is_error: m.message.is_error,
                profile: m.origin.as_ref().map(|o| o.profile.clone()),
                model: m.origin.map(|o| o.model),
                incomplete: m.incomplete,
                created_at_ms: millis(m.created_at),
                images: m.images,
                marker: m.marker,
            })
            .collect(),
    }
}

fn summary_dto(summary: crate::sessions::SessionSummary) -> rpc::SessionSummary {
    rpc::SessionSummary {
        id: summary.id.to_string(),
        title: summary.title,
        created_at_ms: millis(summary.created_at),
        updated_at_ms: millis(summary.updated_at),
        message_count: summary.message_count,
        incognito: summary.incognito,
    }
}

fn millis(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(windows)]
mod windows_acl {
    use std::io;

    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Full access for the user running the core and for SYSTEM; nobody else.
    pub fn only_this_user() -> io::Result<SecurityDescriptor> {
        let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", current_user_sid()?);
        let wide = widestring::U16CString::from_str(&sddl).map_err(io::Error::other)?;
        SecurityDescriptor::deserialize(&wide)
    }

    /// The current user's SID, e.g. `S-1-5-21-…-1001`.
    fn current_user_sid() -> io::Result<String> {
        // SAFETY: every pointer passed below points into a live buffer of the size given, the
        // token handle is closed once, and the string from ConvertSidToStringSidW is read up to
        // its terminator and then freed with LocalFree, as its documentation requires.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            // u64s keep the buffer aligned for TOKEN_USER.
            let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
            let ok =
                GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
                return Err(io::Error::last_os_error());
            }
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            Ok(sid)
        }
    }
}

/// `<data dir>/images`, next to the chats database.
fn images_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map_or_else(|| PathBuf::from("images"), |dir| dir.join("images"))
}

/// Delete picture files (and their cached descriptions).
fn remove_pictures(dir: &Path, ids: &[String]) {
    for id in ids {
        let path = dir.join(id);
        let _ = std::fs::remove_file(path.with_extension("description.txt"));
        let _ = std::fs::remove_file(path);
    }
}

/// `/compress`: summarise everything the model would get of the chat, now.
async fn compress_now(core: &Arc<Core>, key: &str) -> Result<String, RpcError> {
    let config = core.config()?;
    let settings = core.settings(key, &config)?;
    let profile = &config.profiles[&settings.profile];
    let provider =
        build_provider(&settings.profile, profile, &core.options.secrets).map_err(app)?;
    let id: SessionId = key.parse().map_err(app)?;
    let history = core.store.lock().unwrap().load(&id).map_err(app)?.messages;
    let total = history.len();
    let visible = crate::context::visible(history);
    let messages: Vec<Message> = visible.messages.into_iter().map(|m| m.message).collect();
    if messages.is_empty() {
        return Err(RpcError::app("there's nothing new to compress"));
    }
    let summary = crate::context::summarise(&*provider, visible.summary.as_deref(), &messages)
        .await
        .map_err(RpcError::app)?;
    core.store
        .lock()
        .unwrap()
        .append(&id, NewMessage::summary(summary.clone(), total as u32))
        .map_err(app)?;
    Ok(summary)
}

/// Where to reach a provider, for `models.*`: its kind, address and key.
fn provider_access(
    core: &Core,
    provider: &str,
    base_url: Option<String>,
    key_name: Option<String>,
    key: Option<String>,
) -> Result<(lucy_providers::models::Kind, String, Option<String>), RpcError> {
    let kind = match provider {
        "anthropic" => lucy_providers::models::Kind::Anthropic,
        "openai_compat" => lucy_providers::models::Kind::OpenAiCompat,
        other => return Err(RpcError::app(format!("no provider `{other}`"))),
    };
    let base_url = base_url.unwrap_or_default();
    if kind == lucy_providers::models::Kind::OpenAiCompat && base_url.is_empty() {
        return Err(RpcError::app(
            "an OpenAI-compatible provider needs its address",
        ));
    }
    let key = match (key.filter(|k| !k.is_empty()), key_name) {
        (Some(key), _) => Some(key),
        (None, Some(name)) => core
            .options
            .secrets
            .resolve(&name)
            .map_err(app)?
            .map(|resolved| resolved.value),
        (None, None) => None,
    };
    Ok((kind, base_url, key))
}
