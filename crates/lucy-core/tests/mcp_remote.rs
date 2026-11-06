//! Remote MCP servers over streamable HTTP: API keys from the keyring, and OAuth sign-in
//! against a fake authorization server. The MCP server itself is Lucy's built-in tool server,
//! served over HTTP the way a hosted service would serve its tools.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Form, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use lucy_core::config::{McpServer, ToolSettings};
use lucy_core::mcp::McpHost;
use lucy_core::oauth::{self, TokenStore};
use lucy_core::secrets::{MemoryStore, SecretStore, Secrets};
use lucy_core::tools::{ToolBox, Verdict};
use lucy_protocol::ToolCall;
use lucy_tools::PrivatePaths;
use lucy_tools::builtin::{Builtin, BuiltinOptions};
use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};

// --- A hosted MCP server with a fake sign-in service --------------------------------------

/// How the fake server checks requests to `/mcp`.
#[derive(Clone)]
enum Guard {
    /// Every listed header must be present with this exact value.
    Headers(Vec<(&'static str, &'static str)>),
    /// A bearer token issued by the fake authorization server.
    OAuth,
}

struct Remote {
    base: String,
    state: Arc<Idp>,
    _dir: tempfile::TempDir,
}

#[derive(Default)]
struct Idp {
    guard: Mutex<Option<Guard>>,
    /// Access tokens the MCP endpoint accepts.
    valid: Mutex<HashSet<String>>,
    /// Seconds each issued access token claims to last.
    lifetime: AtomicU32,
    refresh_allowed: AtomicBool,
    /// `grant_type` of every token request, in order.
    grants: Mutex<Vec<String>>,
    registrations: AtomicU32,
    /// `client_id` the authorize endpoint was called with.
    authorized_client: Mutex<Option<String>>,
    /// Send the user back with `error=access_denied` instead of a code.
    deny: AtomicBool,
    /// Authorization header of every MCP request.
    seen_auth: Mutex<Vec<String>>,
    issued: AtomicU32,
    base: Mutex<String>,
}

impl Remote {
    async fn start(guard: Guard) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "remote notes\n").unwrap();
        let state = Arc::new(Idp {
            guard: Mutex::new(Some(guard)),
            refresh_allowed: AtomicBool::new(true),
            ..Idp::default()
        });
        state.lifetime.store(3600, Ordering::SeqCst);

        let cwd = dir.path().to_owned();
        let mcp: StreamableHttpService<Builtin, LocalSessionManager> = StreamableHttpService::new(
            move || {
                Ok(Builtin::new(BuiltinOptions {
                    cwd: cwd.clone(),
                    private: PrivatePaths::defaults(None),
                }))
            },
            Default::default(),
            Default::default(),
        );
        let router = Router::new()
            .nest_service("/mcp", mcp)
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                guard_mcp,
            ))
            .route(
                "/.well-known/oauth-protected-resource",
                get(resource_metadata),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(resource_metadata),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(server_metadata),
            )
            .route("/register", post(register))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(Arc::clone(&state));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        *state.base.lock().unwrap() = base.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self {
            base,
            state,
            _dir: dir,
        }
    }

    fn url(&self) -> String {
        format!("{}/mcp", self.base)
    }
}

async fn guard_mcp(State(idp): State<Arc<Idp>>, request: Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/mcp") {
        return next.run(request).await;
    }
    let headers = request.headers();
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    idp.seen_auth.lock().unwrap().push(auth.clone());
    let guard = idp.guard.lock().unwrap().clone();
    let allowed = match guard {
        Some(Guard::Headers(required)) => required
            .iter()
            .all(|(name, value)| headers.get(*name).and_then(|v| v.to_str().ok()) == Some(*value)),
        Some(Guard::OAuth) => auth
            .strip_prefix("Bearer ")
            .is_some_and(|token| idp.valid.lock().unwrap().contains(token)),
        None => true,
    };
    if allowed {
        return next.run(request).await;
    }
    let base = idp.base.lock().unwrap().clone();
    let challenge =
        format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\"");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, challenge)],
    )
        .into_response()
}

async fn resource_metadata(State(idp): State<Arc<Idp>>) -> impl IntoResponse {
    let base = idp.base.lock().unwrap().clone();
    axum::Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
    }))
}

async fn server_metadata(State(idp): State<Arc<Idp>>) -> impl IntoResponse {
    let base = idp.base.lock().unwrap().clone();
    axum::Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

async fn register(State(idp): State<Arc<Idp>>, axum::Json(body): axum::Json<Value>) -> Response {
    idp.registrations.fetch_add(1, Ordering::SeqCst);
    let mut reply = body.clone();
    reply["client_id"] = json!("registered-client");
    reply["token_endpoint_auth_method"] = json!("none");
    (StatusCode::CREATED, axum::Json(reply)).into_response()
}

async fn authorize(
    State(idp): State<Arc<Idp>>,
    Query(query): Query<HashMap<String, String>>,
) -> Redirect {
    *idp.authorized_client.lock().unwrap() = query.get("client_id").cloned();
    let redirect = &query["redirect_uri"];
    let state = &query["state"];
    if idp.deny.load(Ordering::SeqCst) {
        return Redirect::to(&format!("{redirect}?error=access_denied&state={state}"));
    }
    Redirect::to(&format!("{redirect}?code=the-code&state={state}"))
}

async fn token(State(idp): State<Arc<Idp>>, Form(form): Form<HashMap<String, String>>) -> Response {
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    idp.grants.lock().unwrap().push(grant.clone());
    let ok = match grant.as_str() {
        "authorization_code" => form.get("code").map(String::as_str) == Some("the-code"),
        "refresh_token" => idp.refresh_allowed.load(Ordering::SeqCst),
        _ => false,
    };
    if !ok {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    }
    let n = idp.issued.fetch_add(1, Ordering::SeqCst) + 1;
    let access = format!("access-{n}");
    idp.valid.lock().unwrap().insert(access.clone());
    axum::Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": idp.lifetime.load(Ordering::SeqCst),
        "refresh_token": format!("refresh-{n}"),
    }))
    .into_response()
}

// --- Helpers ------------------------------------------------------------------------------

type TestSecrets = Secrets<MemoryStore, fn(&str) -> Option<String>>;

fn no_env(_: &str) -> Option<String> {
    None
}

fn secrets(entries: &[(&str, &str)]) -> TestSecrets {
    let store = MemoryStore::default();
    for (name, value) in entries {
        store.set(name, value).unwrap();
    }
    Secrets::new(store, no_env as fn(&str) -> Option<String>)
}

/// The "browser": follows the sign-in page's redirect back to Lucy's localhost listener.
fn browser(url: &str) {
    let url = url.to_owned();
    tokio::spawn(async move {
        let _ = reqwest::get(url).await;
    });
}

async fn read_notes(
    host: &McpHost,
    name: &str,
    server: &McpServer,
    secrets: &TestSecrets,
) -> String {
    let remote = host.server(name, server, secrets).await.unwrap();
    let (output, is_error) = remote
        .call("read_file", json!({"path": "notes.txt"}))
        .await
        .unwrap();
    assert!(!is_error, "{output}");
    output
}

// --- API keys -----------------------------------------------------------------------------

#[tokio::test]
async fn a_bearer_key_from_the_keyring_is_sent() {
    let remote = Remote::start(Guard::Headers(vec![("authorization", "Bearer s3cret")])).await;
    let mut server = McpServer::remote(remote.url());
    server.bearer_key = Some("acme".into());
    let host = McpHost::new();

    let notes = read_notes(&host, "acme", &server, &secrets(&[("acme", "s3cret")])).await;

    assert_eq!(notes, "remote notes\n");
    host.shutdown().await;
}

#[tokio::test]
async fn header_keys_and_plain_headers_are_sent() {
    let remote = Remote::start(Guard::Headers(vec![
        ("x-api-key", "k-123"),
        ("x-region", "eu"),
    ]))
    .await;
    let mut server = McpServer::remote(remote.url());
    server
        .header_keys
        .insert("X-Api-Key".into(), "acme-key".into());
    server.headers.insert("X-Region".into(), "eu".into());
    let host = McpHost::new();

    let notes = read_notes(&host, "acme", &server, &secrets(&[("acme-key", "k-123")])).await;

    assert_eq!(notes, "remote notes\n");
    host.shutdown().await;
}

#[tokio::test]
async fn a_rejected_key_says_which_keyring_entry_to_fix() {
    let remote = Remote::start(Guard::Headers(vec![("authorization", "Bearer right")])).await;
    let mut server = McpServer::remote(remote.url());
    server.bearer_key = Some("acme".into());
    let host = McpHost::new();

    let err = host
        .server("acme", &server, &secrets(&[("acme", "wrong")]))
        .await
        .err()
        .expect("the server should refuse the key");

    assert!(err.contains("lucy auth set acme"), "{err}");
}

#[tokio::test]
async fn a_missing_key_says_how_to_add_it() {
    let remote = Remote::start(Guard::Headers(vec![("authorization", "Bearer right")])).await;
    let mut server = McpServer::remote(remote.url());
    server.bearer_key = Some("acme".into());
    let host = McpHost::new();

    let err = host
        .server("acme", &server, &secrets(&[]))
        .await
        .err()
        .expect("no key, no connection");

    assert!(err.contains("lucy auth set acme"), "{err}");
}

#[tokio::test]
async fn remote_read_only_tools_run_without_local_path_checks() {
    let remote = Remote::start(Guard::Headers(vec![])).await;
    let host = McpHost::new();
    let server = host
        .server("hosted", &McpServer::remote(remote.url()), &secrets(&[]))
        .await
        .unwrap();
    let settings = ToolSettings {
        builtin: false,
        ..ToolSettings::default()
    };
    let toolbox = ToolBox::open(&settings, std::env::temp_dir(), vec![server])
        .await
        .unwrap();
    let call = |name: &str, arguments: Value| ToolCall {
        id: "call_1".into(),
        name: name.into(),
        arguments,
    };

    // A path that would be private on this machine means nothing on the remote one.
    let read = toolbox.verdict(
        &call(
            "hosted__read_file",
            json!({"path": "/home/someone/.ssh/id_rsa"}),
        ),
        &HashSet::new(),
    );
    let write = toolbox.verdict(
        &call("hosted__write_file", json!({"path": "x", "content": "y"})),
        &HashSet::new(),
    );

    assert_eq!(read, Verdict::Run);
    assert!(matches!(write, Verdict::Ask { .. }), "{write:?}");
    toolbox.close().await;
    host.shutdown().await;
}

// --- OAuth --------------------------------------------------------------------------------

fn token_store() -> TokenStore {
    Arc::new(MemoryStore::default())
}

#[tokio::test]
async fn without_signing_in_the_error_says_to_log_in() {
    let remote = Remote::start(Guard::OAuth).await;
    let host = McpHost::new();

    let err = host
        .server("linear", &McpServer::remote(remote.url()), &secrets(&[]))
        .await
        .err()
        .expect("the server wants a sign-in");

    assert!(err.contains("lucy mcp login linear"), "{err}");
}

#[tokio::test]
async fn signing_in_stores_tokens_that_later_connections_use() {
    let remote = Remote::start(Guard::OAuth).await;
    let server = McpServer::remote(remote.url());
    let store = token_store();

    oauth::login("linear", &server, Arc::clone(&store), browser)
        .await
        .unwrap();

    assert!(oauth::has_login(&store, "linear"));
    assert_eq!(remote.state.registrations.load(Ordering::SeqCst), 1);
    let host = McpHost::with_token_store(Arc::clone(&store));
    let notes = read_notes(&host, "linear", &server, &secrets(&[])).await;
    assert_eq!(notes, "remote notes\n");
    assert!(
        remote
            .state
            .seen_auth
            .lock()
            .unwrap()
            .contains(&"Bearer access-1".to_owned())
    );
    host.shutdown().await;
}

#[tokio::test]
async fn an_expired_token_is_refreshed() {
    let remote = Remote::start(Guard::OAuth).await;
    remote.state.lifetime.store(1, Ordering::SeqCst);
    let server = McpServer::remote(remote.url());
    let store = token_store();
    oauth::login("linear", &server, Arc::clone(&store), browser)
        .await
        .unwrap();
    // The first token expires, and the server stops accepting it.
    tokio::time::sleep(Duration::from_secs(2)).await;
    remote.state.valid.lock().unwrap().remove("access-1");
    remote.state.lifetime.store(3600, Ordering::SeqCst);

    let host = McpHost::with_token_store(Arc::clone(&store));
    let notes = read_notes(&host, "linear", &server, &secrets(&[])).await;

    assert_eq!(notes, "remote notes\n");
    assert!(
        remote
            .state
            .grants
            .lock()
            .unwrap()
            .contains(&"refresh_token".to_owned()),
        "no refresh happened"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn a_refused_refresh_says_to_log_in_again() {
    let remote = Remote::start(Guard::OAuth).await;
    remote.state.lifetime.store(1, Ordering::SeqCst);
    let server = McpServer::remote(remote.url());
    let store = token_store();
    oauth::login("linear", &server, Arc::clone(&store), browser)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    remote.state.valid.lock().unwrap().clear();
    remote.state.refresh_allowed.store(false, Ordering::SeqCst);

    let host = McpHost::with_token_store(Arc::clone(&store));
    let err = host
        .server("linear", &server, &secrets(&[]))
        .await
        .err()
        .expect("the refresh is refused");

    assert!(err.contains("lucy mcp login linear"), "{err}");
}

#[tokio::test]
async fn a_configured_client_id_skips_registration() {
    let remote = Remote::start(Guard::OAuth).await;
    let mut server = McpServer::remote(remote.url());
    server.oauth_client_id = Some("my-registered-app".into());

    oauth::login("linear", &server, token_store(), browser)
        .await
        .unwrap();

    assert_eq!(remote.state.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(
        remote.state.authorized_client.lock().unwrap().as_deref(),
        Some("my-registered-app")
    );
}

#[tokio::test]
async fn a_declined_sign_in_is_reported_and_stores_nothing() {
    let remote = Remote::start(Guard::OAuth).await;
    remote.state.deny.store(true, Ordering::SeqCst);
    let store = token_store();

    let err = oauth::login(
        "linear",
        &McpServer::remote(remote.url()),
        Arc::clone(&store),
        browser,
    )
    .await
    .unwrap_err();

    assert!(err.contains("access_denied"), "{err}");
    assert!(!oauth::has_login(&store, "linear"));
}

#[tokio::test]
async fn logging_out_forgets_the_tokens() {
    let remote = Remote::start(Guard::OAuth).await;
    let server = McpServer::remote(remote.url());
    let store = token_store();
    oauth::login("linear", &server, Arc::clone(&store), browser)
        .await
        .unwrap();

    assert!(oauth::logout(&store, "linear").unwrap());

    assert!(!oauth::has_login(&store, "linear"));
    let err = McpHost::with_token_store(store)
        .server("linear", &server, &secrets(&[]))
        .await
        .err()
        .expect("signed out");
    assert!(err.contains("lucy mcp login linear"), "{err}");
}
