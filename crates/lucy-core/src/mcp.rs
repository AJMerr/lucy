//! External MCP servers: started as child processes the first time a chat needs tools, then
//! shared by every chat until the core exits (ADR 0008). A server that dies, or whose config
//! changes, is restarted on next use.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use crate::config::McpServer;
use crate::secrets::{self, SecretStore, Secrets};

/// How long a server gets to start and answer the MCP handshake (`npx` may download first).
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// One running server and the tools it offered when it started.
pub struct ExternalServer {
    name: String,
    cwd: PathBuf,
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

    /// The folder it was started in; its relative paths start here.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn is_alive(&self) -> bool {
        !self.client.is_transport_closed()
    }

    /// Call one of its tools by the server's own name. Returns the text output and whether
    /// the tool reported an error; `Err` means the call itself failed (e.g. the server died).
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<(String, bool), String> {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Value::Object(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let result = self
            .client
            .call_tool(params)
            .await
            .map_err(|err| format!("the {} server failed: {err}", self.name))?;
        let mut parts: Vec<String> = result
            .content
            .iter()
            .map(|c| match c.as_text() {
                Some(text) => text.text.clone(),
                None => "[non-text content omitted]".into(),
            })
            .collect();
        if parts.is_empty()
            && let Some(structured) = &result.structured_content
        {
            parts.push(structured.to_string());
        }
        Ok((parts.join("\n"), result.is_error == Some(true)))
    }
}

struct Running {
    config: McpServer,
    server: Arc<ExternalServer>,
}

type Slot = Arc<tokio::sync::Mutex<Option<Running>>>;

#[derive(Default)]
pub struct McpHost {
    /// One slot per server name, so slow starts don't block other servers.
    slots: Mutex<HashMap<String, Slot>>,
}

impl McpHost {
    pub fn new() -> Self {
        Self::default()
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
        let server = Arc::new(start(name, config, secrets).await?);
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
) -> Result<ExternalServer, String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    // Resolve through PATH the way a shell would (`npx` is `npx.cmd` on Windows).
    let mut command = rmcp::transport::which_command(&config.command).map_err(|err| {
        format!(
            "MCP server `{name}`: can't find `{}` ({err})",
            config.command
        )
    })?;
    command.args(&config.args).envs(&config.env);
    for (var, key_name) in &config.env_keys {
        let key = secrets
            .resolve(key_name)
            .map_err(|err| format!("MCP server `{name}`: {err}"))?
            .ok_or_else(|| {
                format!(
                    "MCP server `{name}` needs keyring entry `{key_name}` for {var}: run `lucy \
                     auth set {key_name}` or set {}",
                    secrets::env_var_name(key_name)
                )
            })?;
        command.env(var, key.value);
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
        .map_err(|err| {
            format!(
                "MCP server `{name}`: couldn't start `{}`: {err}",
                config.command
            )
        })?;
    let handshake = async {
        let client = ().serve(transport).await.map_err(|err| err.to_string())?;
        let tools = client
            .list_all_tools()
            .await
            .map_err(|err| format!("couldn't list its tools: {err}"))?;
        Ok::<_, String>((client, tools))
    };
    let (client, tools) = tokio::time::timeout(START_TIMEOUT, handshake)
        .await
        .map_err(|_| {
            format!(
                "MCP server `{name}` didn't finish starting within {}s",
                START_TIMEOUT.as_secs()
            )
        })?
        .map_err(|err| format!("MCP server `{name}`: {err}"))?;
    Ok(ExternalServer {
        name: name.to_owned(),
        cwd,
        client,
        tools,
    })
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
