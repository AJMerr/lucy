//! Signing in to remote MCP servers with OAuth (ADR 0008): the browser flow behind
//! `lucy mcp login`, and keyring storage for the tokens it produces. Discovery, client
//! registration, PKCE, token exchange and refresh are rmcp's.

use std::sync::Arc;
use std::time::Duration;

use rmcp::transport::auth::{
    AuthError, AuthorizationCallback, AuthorizationManager, AuthorizationRequest, CredentialStore,
    OAuthState, StoredCredentials,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::McpServer;
use crate::secrets::SecretStore;

pub type TokenStore = Arc<dyn SecretStore + Send + Sync>;

/// How long `lucy mcp login` waits for the browser to come back.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The keyring entry holding a server's OAuth tokens and client registration.
pub fn token_entry(server: &str) -> String {
    format!("mcp-oauth-{server}")
}

/// rmcp's credential store, backed by one keyring entry (JSON) per server.
pub struct KeyringCredentials {
    store: TokenStore,
    entry: String,
}

impl KeyringCredentials {
    pub fn new(store: TokenStore, server: &str) -> Self {
        Self {
            store,
            entry: token_entry(server),
        }
    }
}

fn store_error(err: impl std::fmt::Display) -> AuthError {
    AuthError::CredentialStoreError(err.to_string())
}

#[async_trait::async_trait]
impl CredentialStore for KeyringCredentials {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.store.get(&self.entry).map_err(store_error)? {
            Some(json) => serde_json::from_str(&json).map(Some).map_err(store_error),
            None => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string(&credentials).map_err(store_error)?;
        self.store.set(&self.entry, &json).map_err(store_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.store.delete(&self.entry).map_err(store_error)?;
        Ok(())
    }
}

/// Whether `server` has stored OAuth tokens.
pub fn has_login(store: &TokenStore, server: &str) -> bool {
    matches!(store.get(&token_entry(server)), Ok(Some(_)))
}

/// Forget `server`'s tokens. Returns whether there were any.
pub fn logout(store: &TokenStore, server: &str) -> Result<bool, String> {
    store
        .delete(&token_entry(server))
        .map_err(|err| err.to_string())
}

/// Sign in to a remote server: register Lucy (unless a client ID is configured), send the
/// user to the sign-in page via `open`, catch the redirect on localhost, and store the
/// tokens. `open` receives the URL to visit and must not block.
pub async fn login(
    name: &str,
    server: &McpServer,
    store: TokenStore,
    open: impl FnOnce(&str),
) -> Result<(), String> {
    let url = server
        .url
        .as_deref()
        .ok_or_else(|| format!("`{name}` isn't a remote server; only URL servers sign in"))?;
    let fail = |err: AuthError| format!("signing in to `{name}` failed: {err}");

    let listener = TcpListener::bind(("127.0.0.1", server.oauth_redirect_port.unwrap_or(0)))
        .await
        .map_err(|err| format!("couldn't listen for the sign-in redirect: {err}"))?;
    let port = listener.local_addr().map_err(|err| err.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let mut manager = AuthorizationManager::new(url).await.map_err(fail)?;
    manager.set_credential_store(KeyringCredentials::new(store, name));
    let mut state = OAuthState::Unauthorized(manager);
    let mut request = AuthorizationRequest::new(&redirect_uri).with_client_name("Lucy");
    if let Some(client_id) = &server.oauth_client_id {
        request = request.with_preregistered_client(client_id);
    }
    state.start_authorization(request).await.map_err(fail)?;
    let auth_url = state.get_authorization_url().await.map_err(fail)?;

    open(&auth_url);
    let callback = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(&listener))
        .await
        .map_err(|_| "no sign-in within 5 minutes; run `lucy mcp login` again".to_owned())??;
    state
        .handle_callback_with_issuer(
            &callback.code,
            &callback.csrf_token,
            callback.issuer.as_deref(),
        )
        .await
        .map_err(fail)
}

/// Answer requests on the redirect port until one carries the authorization code.
async fn wait_for_callback(listener: &TcpListener) -> Result<AuthorizationCallback, String> {
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|err| format!("sign-in redirect failed: {err}"))?;
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 64 * 1024 {
            match socket.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&buf[..n]),
            }
        }
        let head = String::from_utf8_lossy(&request);
        let target = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        if !target.starts_with("/callback") {
            let _ = respond(&mut socket, 404, "Not found").await;
            continue;
        }
        let url = format!("http://127.0.0.1{target}");
        if let Some(error) = query_value(&url, "error") {
            let _ = respond(
                &mut socket,
                400,
                "Sign-in was cancelled. You can close this tab.",
            )
            .await;
            return Err(format!("the server refused sign-in: {error}"));
        }
        return match AuthorizationCallback::from_redirect_url(&url) {
            Ok(callback) => {
                let _ = respond(
                    &mut socket,
                    200,
                    "Signed in. You can close this tab and go back to Lucy.",
                )
                .await;
                Ok(callback)
            }
            Err(err) => {
                let _ = respond(&mut socket, 400, "Sign-in failed. You can close this tab.").await;
                Err(format!("bad sign-in redirect: {err}"))
            }
        };
    }
}

fn query_value(url: &str, key: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

async fn respond(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    text: &str,
) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Lucy</title>\
         <body style=\"font-family:sans-serif;margin:3em\"><p>{text}</p></body>"
    );
    let reply = format!(
        "HTTP/1.1 {status} Lucy\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(reply.as_bytes()).await?;
    socket.shutdown().await
}

/// Open `url` in the browser on this machine. Best effort: if none opens, a printed URL still
/// works.
pub fn open_browser(url: &str) {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]).arg(url);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    let _ = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
