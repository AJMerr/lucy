//! External MCP servers: started as child processes the first time a chat needs tools, then
//! shared by every chat until the core exits (ADR 0008). A server that dies, or whose config
//! changes, is restarted on next use.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lucy_protocol::rpc::ImageData;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::auth::{AuthClient, AuthorizationManager};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use crate::config::McpServer;
use crate::oauth::{self, KeyringCredentials, TokenStore};
use crate::secrets::{self, MemoryStore, SecretStore, Secrets};

type Client = RunningService<RoleClient, ()>;

/// How long a server gets to start and answer the MCP handshake (`npx` may download first).
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// One running server and the tools it offered when it started.
pub struct ExternalServer {
    name: String,
    /// Where a local server runs; `None` for remote servers.
    cwd: Option<PathBuf>,
    client: RunningService<RoleClient, ()>,
    tools: Vec<rmcp::model::Tool>,
}

impl ExternalServer {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tools(&self) -> &[rmcp::model::Tool] {
        &self.tools
    }

    /// The folder a local server was started in (its relative paths start here); `None` for a
    /// remote server, whose paths aren't on this machine.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn is_alive(&self) -> bool {
        !self.client.is_transport_closed()
    }

    /// Call one of its tools by the server's own name. `Err` means the call itself failed (e.g.
    /// the server died).
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<ToolOutput, String> {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Value::Object(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let result = self
            .client
            .call_tool(params)
            .await
            .map_err(|err| format!("the {} server failed: {err}", self.name))?;
        Ok(ToolOutput::from_result(result))
    }
}

/// What a tool call gave back: its text, whether it failed, and any pictures.
#[derive(Debug, Default)]
pub struct ToolOutput {
    pub text: String,
    pub is_error: bool,
    pub images: Vec<ImageData>,
    /// The pictures are ones it made, not ones it looked at.
    pub made: bool,
}

impl ToolOutput {
    /// A call that failed, or didn't run, with `text` saying why.
    pub fn failed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
            images: Vec::new(),
            made: false,
        }
    }

    /// An MCP result: its text parts joined and its pictures kept.
    pub fn from_result(result: CallToolResult) -> Self {
        let mut parts = Vec::new();
        let mut images = Vec::new();
        for content in &result.content {
            if let Some(text) = content.as_text() {
                parts.push(text.text.clone());
            } else if let Some(image) = content.as_image() {
                images.push(ImageData {
                    media_type: image.mime_type.clone(),
                    data: image.data.clone(),
                });
            } else {
                parts.push("[non-text content omitted]".into());
            }
        }
        if parts.is_empty()
            && images.is_empty()
            && let Some(structured) = &result.structured_content
        {
            parts.push(structured.to_string());
        }
        Self {
            text: parts.join("\n"),
            is_error: result.is_error == Some(true),
            images,
            made: false,
        }
    }
}

struct Running {
    config: McpServer,
    server: Arc<ExternalServer>,
}

type Slot = Arc<tokio::sync::Mutex<Option<Running>>>;

pub struct McpHost {
    /// One slot per server name, so slow starts don't block other servers.
    slots: Mutex<HashMap<String, Slot>>,
    /// Where OAuth tokens for remote servers live (the OS keyring in production).
    tokens: TokenStore,
}

impl Default for McpHost {
    fn default() -> Self {
        Self::new()
    }
}

impl McpHost {
    /// A host whose OAuth tokens live only in memory.
    pub fn new() -> Self {
        Self::with_token_store(Arc::new(MemoryStore::default()))
    }

    pub fn with_token_store(tokens: TokenStore) -> Self {
        Self {
            slots: Mutex::default(),
            tokens,
        }
    }

    /// The running server `name`, starting it (or restarting it, if it died or its config
    /// changed) as needed.
    pub async fn server<S, E>(
        &self,
        name: &str,
        config: &McpServer,
        secrets: &Secrets<S, E>,
    ) -> Result<Arc<ExternalServer>, String>
    where
        S: SecretStore,
        E: Fn(&str) -> Option<String>,
    {
        let slot = self
            .slots
            .lock()
            .unwrap()
            .entry(name.to_owned())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        if let Some(running) = slot.as_ref()
            && running.config == *config
            && running.server.is_alive()
        {
            return Ok(Arc::clone(&running.server));
        }
        if let Some(old) = slot.take() {
            stop(old.server).await;
        }
        let server = Arc::new(start(name, config, secrets, &self.tokens).await?);
        *slot = Some(Running {
            config: config.clone(),
            server: Arc::clone(&server),
        });
        Ok(server)
    }

    /// Every enabled server in `servers`, started in parallel, plus the ones that failed and
    /// why. Servers no longer configured are stopped.
    pub async fn servers<S, E>(
        &self,
        servers: &BTreeMap<String, McpServer>,
        secrets: &Secrets<S, E>,
    ) -> (Vec<Arc<ExternalServer>>, Vec<(String, String)>)
    where
        S: SecretStore,
        E: Fn(&str) -> Option<String>,
    {
        let removed: Vec<Slot> = {
            let mut slots = self.slots.lock().unwrap();
            let gone: Vec<String> = slots
                .keys()
                .filter(|name| !servers.get(*name).is_some_and(|s| s.enabled))
                .cloned()
                .collect();
            gone.iter().filter_map(|name| slots.remove(name)).collect()
        };
        for slot in removed {
            if let Some(old) = slot.lock().await.take() {
                stop(old.server).await;
            }
        }

        let starts =
            servers
                .iter()
                .filter(|(_, config)| config.enabled)
                .map(|(name, config)| async move {
                    (name.clone(), self.server(name, config, secrets).await)
                });
        let mut running = Vec::new();
        let mut failed = Vec::new();
        for (name, result) in futures::future::join_all(starts).await {
            match result {
                Ok(server) => running.push(server),
                Err(err) => failed.push((name, err)),
            }
        }
        (running, failed)
    }

    /// Stop every server (when the core exits).
    pub async fn shutdown(&self) {
        let slots: Vec<Slot> = self.slots.lock().unwrap().drain().map(|(_, s)| s).collect();
        for slot in slots {
            if let Some(old) = slot.lock().await.take() {
                stop(old.server).await;
            }
        }
    }
}

async fn start<S, E>(
    name: &str,
    config: &McpServer,
    secrets: &Secrets<S, E>,
    tokens: &TokenStore,
) -> Result<ExternalServer, String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    let (client, cwd) = match (&config.command, &config.url) {
        (Some(command), _) => start_program(name, command, config, secrets).await?,
        (None, Some(url)) => (
            start_remote(name, url, config, secrets, tokens).await?,
            None,
        ),
        (None, None) => return Err(format!("MCP server `{name}` has no command or url")),
    };
    let tools = tokio::time::timeout(START_TIMEOUT, client.list_all_tools())
        .await
        .map_err(|_| format!("MCP server `{name}` didn't list its tools in time"))?
        .map_err(|err| format!("MCP server `{name}`: couldn't list its tools: {err}"))?;
    Ok(ExternalServer {
        name: name.to_owned(),
        cwd,
        client,
        tools,
    })
}

/// A key from the keyring (or its `LUCY_KEY_*` override), or how to provide it.
fn key<S, E>(
    secrets: &Secrets<S, E>,
    server: &str,
    key_name: &str,
    purpose: &str,
) -> Result<String, String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    secrets::not_reserved(key_name).map_err(|err| format!("MCP server `{server}`: {err}"))?;
    Ok(secrets
        .resolve(key_name)
        .map_err(|err| format!("MCP server `{server}`: {err}"))?
        .ok_or_else(|| {
            format!(
                "MCP server `{server}` needs keyring entry `{key_name}` for {purpose}: run `lucy \
                 auth set {key_name}` or set {}",
                secrets::env_var_name(key_name)
            )
        })?
        .value)
}

async fn start_program<S, E>(
    name: &str,
    program: &str,
    config: &McpServer,
    secrets: &Secrets<S, E>,
) -> Result<(Client, Option<PathBuf>), String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    // Resolve through PATH the way a shell would (`npx` is `npx.cmd` on Windows).
    let mut command = rmcp::transport::which_command(program)
        .map_err(|err| format!("MCP server `{name}`: can't find `{program}` ({err})"))?;
    command.args(&config.args).envs(&config.env);
    for (var, key_name) in &config.env_keys {
        command.env(var, key(secrets, name, key_name, var)?);
    }
    let cwd = config
        .cwd
        .clone()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);
    command.current_dir(&cwd);

    // The server's own log output goes to the core's log.
    let (transport, _) = TokioChildProcess::builder(command)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("MCP server `{name}`: couldn't start `{program}`: {err}"))?;
    let client = tokio::time::timeout(START_TIMEOUT, ().serve(transport))
        .await
        .map_err(|_| {
            format!(
                "MCP server `{name}` didn't finish starting within {}s",
                START_TIMEOUT.as_secs()
            )
        })?
        .map_err(|err| format!("MCP server `{name}`: {err}"))?;
    Ok((client, Some(cwd)))
}

async fn start_remote<S, E>(
    name: &str,
    url: &str,
    config: &McpServer,
    secrets: &Secrets<S, E>,
    tokens: &TokenStore,
) -> Result<Client, String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    let mut headers = HashMap::new();
    let plain = config
        .headers
        .iter()
        .map(|(h, v)| Ok::<_, String>((h, v.clone())));
    let keyed = config
        .header_keys
        .iter()
        .map(|(h, key_name)| Ok((h, key(secrets, name, key_name, &format!("header {h}"))?)));
    for pair in plain.chain(keyed) {
        let (header, value): (&String, String) = pair?;
        let header = HeaderName::try_from(header.as_str())
            .map_err(|err| format!("MCP server `{name}`: bad header name `{header}`: {err}"))?;
        let value = HeaderValue::try_from(value)
            .map_err(|err| format!("MCP server `{name}`: bad value for header {header}: {err}"))?;
        headers.insert(header, value);
    }
    let mut transport_config =
        StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);

    let connected = if let Some(key_name) = &config.bearer_key {
        transport_config = transport_config.auth_header(key(secrets, name, key_name, "its token")?);
        let transport =
            StreamableHttpClientTransport::with_client(reqwest::Client::new(), transport_config);
        tokio::time::timeout(START_TIMEOUT, ().serve(transport)).await
    } else if oauth::has_login(tokens, name) {
        let mut manager = AuthorizationManager::new(url)
            .await
            .map_err(|err| format!("MCP server `{name}`: {err}"))?;
        manager.set_credential_store(KeyringCredentials::new(Arc::clone(tokens), name));
        manager
            .initialize_from_store()
            .await
            .map_err(|err| signed_out(name, &err.to_string()))?;
        let client = AuthClient::new(reqwest::Client::new(), manager);
        let transport = StreamableHttpClientTransport::with_client(client, transport_config);
        tokio::time::timeout(START_TIMEOUT, ().serve(transport)).await
    } else {
        let transport =
            StreamableHttpClientTransport::with_client(reqwest::Client::new(), transport_config);
        tokio::time::timeout(START_TIMEOUT, ().serve(transport)).await
    };
    connected
        .map_err(|_| format!("MCP server `{name}` at {url} didn't answer in time"))?
        .map_err(|err| {
            let detail = format!("{err:?}");
            let keys: Vec<&String> = config
                .bearer_key
                .iter()
                .chain(config.header_keys.values())
                .collect();
            if needs_login(&detail) && !keys.is_empty() {
                let fixes: Vec<String> = keys
                    .iter()
                    .map(|k| format!("`lucy auth set {k}`"))
                    .collect();
                format!(
                    "MCP server `{name}` refused its key; check it with {}",
                    fixes.join(" or ")
                )
            } else if needs_login(&detail) {
                signed_out(name, &err.to_string())
            } else {
                format!(
                    "couldn't reach MCP server `{name}` at {url}: {}",
                    root_cause(&err)
                )
            }
        })
}

/// Whether a connection error means the server wants (new) credentials.
fn needs_login(detail: &str) -> bool {
    [
        "AuthRequired",
        "AuthorizationRequired",
        "TokenRefresh",
        "401",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

fn signed_out(name: &str, err: &str) -> String {
    format!("MCP server `{name}` needs you to sign in: run `lucy mcp login {name}` ({err})")
}

/// Close the connection; the child process is killed when its transport drops.
async fn stop(server: Arc<ExternalServer>) {
    match Arc::try_unwrap(server) {
        Ok(server) => {
            let _ = server.client.cancel().await;
        }
        // Still in use by a turn; it goes away when that turn drops it.
        Err(server) => server.client.cancellation_token().cancel(),
    }
}

/// A readable reason: rmcp's transport errors embed Rust type names in brackets, and bury
/// "couldn't connect" inside them.
fn root_cause(err: &(dyn std::error::Error + 'static)) -> String {
    let mut deepest = err;
    while let Some(source) = deepest.source() {
        deepest = source;
    }
    let text = deepest.to_string();
    if text.contains("error sending request") {
        return "no connection (check the URL, and that the server is up)".into();
    }
    let mut readable = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            c if depth == 0 => readable.push(c),
            _ => {}
        }
    }
    readable.split_whitespace().collect::<Vec<_>>().join(" ")
}
