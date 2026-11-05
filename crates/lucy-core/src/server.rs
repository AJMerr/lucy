//! The core's local-socket server: JSON-RPC over newline-delimited JSON (ADR 0003).

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use lucy_client::Endpoint;
use lucy_protocol::rpc::{
    self, CancelResult, ChatSendParams, ChatSendResult, Event, HelloParams, HelloResult,
    Notification, PROTOCOL_VERSION, ProfileInfo, RenameParams, Request, ResolveParams, Response,
    RpcError, SessionParams, SessionSettings, SetPlainParams, SetProfileParams, TurnOutcome, code,
    method,
};
use lucy_protocol::{Message, Role};
use lucy_providers::{ChatRequest, Provider, ProviderError, StreamEvent};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::{Config, ProviderKind};
use crate::persona::{Personas, PromptContext, build_system_prompt, detect_machine};
use crate::provider::build_provider;

use crate::secrets::{SecretStore, Secrets};
use crate::sessions::{NewMessage, Origin, SessionId, SessionStore, StoreError};

pub type DynSecrets =
    Secrets<Box<dyn SecretStore + Send + Sync>, Box<dyn Fn(&str) -> Option<String> + Send + Sync>>;

pub struct CoreOptions {
    pub endpoint: Endpoint,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    /// User persona folder; `None` uses only the built-in personas.
    pub personas_dir: Option<PathBuf>,
    pub secrets: DynSecrets,
    /// Exit after this long with no connections and no reply streaming.
    pub idle_timeout: Duration,
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
    changed: Notify,
}

impl Activity {
    fn is_idle(&self) -> bool {
        self.connections.load(Ordering::SeqCst) == 0 && self.turns.load(Ordering::SeqCst) == 0
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
    /// Per-session `/model` and `/plain` choices (not persisted; defaults come from config).
    overrides: Mutex<HashMap<String, Overrides>>,
    /// Sessions with a reply in progress, and how to cancel it.
    turns: Mutex<HashMap<String, CancellationToken>>,
    next_turn: AtomicU64,
    events: broadcast::Sender<Event>,
    activity: Activity,
}

#[derive(Default, Clone)]
struct Overrides {
    profile: Option<String>,
    plain: Option<bool>,
}

/// Per-connection state shared by its reader and event forwarder.
#[derive(Default)]
struct ConnState {
    hello: bool,
    subscribed: bool,
    own_sessions: HashSet<String>,
}

/// Run the core until it has been idle for `idle_timeout`.
pub async fn serve(options: CoreOptions) -> Result<(), CoreError> {
    let listener = bind(&options.endpoint).await?;
    let store = SessionStore::open(&options.db_path)?;
    let (events, _) = broadcast::channel(1024);
    let idle_timeout = options.idle_timeout;
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
    });

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
                        connection(Arc::clone(&core), stream).await;
                        core.activity.add(&core.activity.connections, -1);
                    });
                }
            }
        })
    };

    loop {
        let changed = core.activity.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let idle = core.activity.is_idle();
        tokio::select! {
            _ = &mut changed => {}
            _ = tokio::time::sleep(idle_timeout), if idle => break,
        }
    }
    acceptor.abort();
    // Dropping the listener (inside the aborted task) removes the socket file.
    let _ = acceptor.await;
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
        ListenerOptions::new()
            .name(endpoint.name()?)
            .reclaim_name(true)
            .create_tokio()
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

async fn connection(core: Arc<Core>, stream: Stream) {
    let stream = Arc::new(stream);
    let state = Arc::new(Mutex::new(ConnState::default()));
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

    let mut lines = BufReader::new(&*stream).lines();
    while let Ok(Some(text)) = lines.next_line().await {
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

    forwarder.abort();
    drop(out_tx);
    let _ = writer.await;
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
        state.lock().unwrap().hello = true;
        return result(HelloResult {
            protocol_version: PROTOCOL_VERSION,
            core_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
        });
    }
    if !state.lock().unwrap().hello {
        return Err(RpcError::app("call hello first"));
    }
    match request.method.as_str() {
        method::CHAT_SEND => chat_send(core, state, params(request.params)?),
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
            state.lock().unwrap().subscribed = true;
            result(Value::Null)
        }
        method::SESSIONS_LOAD => {
            let p: SessionParams = params(request.params)?;
            let store = core.store.lock().unwrap();
            let id = store.resolve(&p.session_id).map_err(app)?;
            result(session_dto(store.load(&id).map_err(app)?))
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
            result(core.settings(&key, &core.config()?))
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
            let key = core.resolve_key(&p.session_id)?;
            core.overrides
                .lock()
                .unwrap()
                .entry(key.clone())
                .or_default()
                .profile = Some(p.profile);
            result(core.settings(&key, &config))
        }
        method::SESSION_SET_PLAIN => {
            let p: SetPlainParams = params(request.params)?;
            let key = core.resolve_key(&p.session_id)?;
            core.overrides
                .lock()
                .unwrap()
                .entry(key.clone())
                .or_default()
                .plain = Some(p.plain);
            result(core.settings(&key, &core.config()?))
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

    /// The full ID of the session matching `prefix`.
    fn resolve_key(&self, prefix: &str) -> Result<String, RpcError> {
        let store = self.store.lock().unwrap();
        Ok(store.resolve(prefix).map_err(app)?.to_string())
    }

    /// The profile and plain flag in effect for a session.
    fn settings(&self, session: &str, config: &Config) -> SessionSettings {
        let overrides = self
            .overrides
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default();
        SessionSettings {
            profile: overrides
                .profile
                .filter(|p| config.profiles.contains_key(p))
                .unwrap_or_else(|| config.defaults.profile.clone()),
            plain: overrides.plain.unwrap_or(false),
        }
    }
}

fn chat_send(
    core: &Arc<Core>,
    state: &Arc<Mutex<ConnState>>,
    p: ChatSendParams,
) -> Result<Value, RpcError> {
    let config = core.config()?;
    let session_id = {
        let store = core.store.lock().unwrap();
        match &p.session_id {
            Some(prefix) => store.resolve(prefix),
            None => store.create(),
        }
        .map_err(app)?
    };
    let key = session_id.to_string();
    let settings = core.settings(&key, &config);
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
        if turns.contains_key(&key) {
            return Err(RpcError::app(
                "Lucy is still replying in this session; wait or cancel first",
            ));
        }
        turns.insert(key.clone(), cancel.clone());
    }

    let history = {
        let store = core.store.lock().unwrap();
        store
            .append(&session_id, NewMessage::user(p.text.clone()))
            .and_then(|()| store.load(&session_id))
    };
    let history = match history {
        Ok(session) => session.messages,
        Err(err) => {
            core.turns.lock().unwrap().remove(&key);
            return Err(app(err));
        }
    };
    let _ = core.events.send(Event::UserMessage {
        session_id: key.clone(),
        text: p.text,
    });

    let system = build_system_prompt(
        &persona,
        &PromptContext {
            plain: settings.plain,
            now: chrono::Local::now().fixed_offset(),
            machine: Some(core.machine.clone()),
            user_name: config.user.name.clone(),
            origin: Some(origin.clone()),
        },
    );
    let messages = std::iter::once(Message::new(Role::System, system))
        .chain(history.into_iter().map(|m| m.message))
        .collect();

    let turn_id = core.next_turn.fetch_add(1, Ordering::Relaxed);
    state.lock().unwrap().own_sessions.insert(key.clone());
    core.activity.add(&core.activity.turns, 1);
    tokio::spawn(run_turn(
        Arc::clone(core),
        session_id,
        turn_id,
        provider,
        ChatRequest { messages },
        origin,
        cancel,
    ));
    result(ChatSendResult {
        session_id: key,
        turn_id,
    })
}

async fn run_turn(
    core: Arc<Core>,
    session_id: SessionId,
    turn_id: u64,
    provider: Box<dyn Provider>,
    request: ChatRequest,
    origin: Origin,
    cancel: CancellationToken,
) {
    let key = session_id.to_string();
    let _ = core.events.send(Event::TurnStarted {
        session_id: key.clone(),
        turn_id,
        profile: origin.profile.clone(),
        model: origin.model.clone(),
    });

    let mut text = String::new();
    let streamed = async {
        let mut stream = provider.stream(request).await?;
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    let _ = core.events.send(Event::Delta {
                        session_id: key.clone(),
                        turn_id,
                        text: delta,
                    });
                }
                StreamEvent::Done => return Ok(()),
            }
        }
        Err(ProviderError::Protocol(
            "stream ended without finishing".into(),
        ))
    };
    let (outcome, error) = tokio::select! {
        _ = cancel.cancelled() => (TurnOutcome::Cancelled, None),
        streamed = streamed => match streamed {
            Ok(()) => (TurnOutcome::Complete, None),
            Err(err) => (TurnOutcome::Failed, Some(err.to_string())),
        },
    };

    // Keep whatever arrived; a cut-off reply is saved and marked incomplete.
    let reply = match outcome {
        TurnOutcome::Complete => Some(NewMessage::assistant(text, origin)),
        _ if !text.is_empty() => Some(NewMessage::assistant(text, origin).incomplete()),
        _ => None,
    };
    if let Some(reply) = reply {
        // The session may have been deleted mid-reply; nothing to save it to then.
        let _ = core.store.lock().unwrap().append(&session_id, reply);
    }
    // Free the session before announcing, so a client can send its next message right away.
    core.turns.lock().unwrap().remove(&key);
    let _ = core.events.send(Event::TurnFinished {
        session_id: key,
        turn_id,
        outcome,
        error,
    });
    core.activity.add(&core.activity.turns, -1);
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
                profile: m.origin.as_ref().map(|o| o.profile.clone()),
                model: m.origin.map(|o| o.model),
                incomplete: m.incomplete,
                created_at_ms: millis(m.created_at),
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
    }
}

fn millis(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
