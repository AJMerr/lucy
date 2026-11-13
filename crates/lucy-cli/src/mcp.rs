//! `lucy mcp`: add, list and remove external MCP servers, and serve the built-in tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use lucy_core::config::{Config, McpServer, validate_server_name};
use lucy_core::mcp::McpHost;
use lucy_core::oauth;
use lucy_core::secrets::{self, OsKeyring, SystemSecrets};
use lucy_protocol::rpc::{ToolsList, ToolsListParams, method};
use lucy_tools::PrivatePaths;
use lucy_tools::builtin::{self, BuiltinOptions};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, value};

use crate::core_process;
use crate::safe_text::visible;
use crate::wizard::Prompter;

#[derive(Debug, Default, Clone)]
pub struct AddArgs {
    pub name: Option<String>,
    /// The program and its arguments; empty (with no `url`) means ask.
    pub command: Vec<String>,
    /// `VAR=keyname`: fill `VAR` from the keyring entry `keyname`.
    pub keys: Vec<String>,
    /// `VAR=value`: plain environment variables.
    pub env: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// A remote server's address, instead of a command.
    pub url: Option<String>,
    /// Keyring entry sent as a bearer token.
    pub bearer_key: Option<String>,
    /// `HEADER=keyname`: send a header filled from the keyring.
    pub header_keys: Vec<String>,
    /// `HEADER=value`: plain headers.
    pub headers: Vec<String>,
}

/// Turn flags into a server entry, asking for the name and the command (or URL) if they're
/// missing. When it had to ask, it also asks where its secrets come from: environment
/// variables for a program, a token for a URL.
pub fn complete_add(args: AddArgs, prompter: &mut impl Prompter) -> Result<(String, McpServer)> {
    let mut asked = false;
    let name = match args.name {
        Some(name) => name,
        None => {
            asked = true;
            prompter.input("Name for this server (e.g. github)", None)?
        }
    };
    let name = name.trim().to_owned();
    validate_server_name(&name).map_err(anyhow::Error::msg)?;

    if !args.command.is_empty() && args.url.is_some() {
        bail!("give either a command or a --url, not both");
    }
    let (command, url) = match (args.command, args.url) {
        (command, None) if command.is_empty() => {
            asked = true;
            let line = prompter.input(
                "Command that starts it, or its URL (e.g. npx -y @modelcontextprotocol/server-github, \
                 or https://mcp.example.com/mcp)",
                None,
            )?;
            let line = line.trim();
            if line.starts_with("http://") || line.starts_with("https://") {
                (Vec::new(), Some(line.to_owned()))
            } else {
                let words = shell_words::split(line).context("couldn't split that command line")?;
                (words, None)
            }
        }
        (command, url) => (command, url),
    };

    let mut server = match url {
        Some(url) => {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                bail!("`{url}` isn't an http(s) URL");
            }
            let mut server = McpServer::remote(url);
            server.bearer_key = args.bearer_key;
            if asked && server.bearer_key.is_none() {
                let entry = prompter.input(
                    "Keyring entry holding an API token for it (leave empty if it uses sign-in \
                     or needs none)",
                    Some(""),
                )?;
                server.bearer_key = Some(entry.trim().to_owned()).filter(|e| !e.is_empty());
            }
            server
        }
        None => {
            let Some((program, rest)) = command.split_first().filter(|(p, _)| !p.trim().is_empty())
            else {
                bail!("the server needs a command to start it");
            };
            let mut keys = args.keys.clone();
            if asked {
                loop {
                    let entry = prompter.input(
                        "Environment variable to fill from the keyring, as VAR=keyname (leave \
                         empty when done)",
                        Some(""),
                    )?;
                    if entry.trim().is_empty() {
                        break;
                    }
                    keys.push(entry.trim().to_owned());
                }
            }
            let mut server = McpServer::program(program.clone(), rest.to_vec());
            server.env_keys = pairs(&keys, "VAR=keyname", is_env_name)?;
            server.cwd = args.cwd;
            server
        }
    };
    server.env = pairs(&args.env, "VAR=value", is_env_name)?;
    server.header_keys = pairs(&args.header_keys, "HEADER=keyname", is_header_name)?;
    server.headers = pairs(&args.headers, "HEADER=value", is_header_name)?;
    Ok((name, server))
}

fn pairs(
    items: &[String],
    shape: &str,
    valid_name: fn(&str) -> bool,
) -> Result<BTreeMap<String, String>> {
    items
        .iter()
        .map(|item| match item.split_once('=') {
            Some((name, val)) if valid_name(name) && !val.is_empty() => {
                Ok((name.to_owned(), val.to_owned()))
            }
            _ => bail!("`{item}` should look like {shape}"),
        })
        .collect()
}

fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Add `[mcp.servers.<name>]` to the config file, keeping its comments and layout.
pub fn add_to_config(path: &Path, name: &str, server: &McpServer) -> Result<()> {
    let mut doc = read_document(path)?;
    let servers = servers_table(&mut doc);
    if servers.contains_key(name) {
        bail!("there's already an MCP server named `{name}`; remove it first");
    }
    let mut table = Table::new();
    for (key, text) in [
        ("command", &server.command),
        ("url", &server.url),
        ("bearer_key", &server.bearer_key),
    ] {
        if let Some(text) = text {
            table[key] = value(text);
        }
    }
    if !server.args.is_empty() {
        table["args"] = value(server.args.iter().collect::<Array>());
    }
    for (key, map) in [
        ("env", &server.env),
        ("env_keys", &server.env_keys),
        ("headers", &server.headers),
        ("header_keys", &server.header_keys),
    ] {
        if !map.is_empty() {
            let mut inline = InlineTable::new();
            for (var, val) in map {
                inline.insert(var, val.as_str().into());
            }
            table[key] = value(inline);
        }
    }
    if let Some(cwd) = &server.cwd {
        table["cwd"] = value(cwd.display().to_string());
    }
    servers.insert(name, Item::Table(table));
    write_document(path, &doc)
}

/// Remove `[mcp.servers.<name>]` from the config file.
pub fn remove_from_config(path: &Path, name: &str) -> Result<()> {
    let mut doc = read_document(path)?;
    if servers_table(&mut doc).remove(name).is_none() {
        bail!("no MCP server named `{name}` in {}", path.display());
    }
    write_document(path, &doc)
}

fn is_env_name(var: &str) -> bool {
    !var.is_empty() && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn read_document(path: &Path) -> Result<DocumentMut> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "couldn't read {}; run `lucy config init` first",
            path.display()
        )
    })?;
    text.parse()
        .with_context(|| format!("{} isn't valid TOML", path.display()))
}

fn servers_table(doc: &mut DocumentMut) -> &mut Table {
    let mcp = doc
        .entry("mcp")
        .or_insert_with(|| Item::Table(implicit_table()));
    let servers = mcp
        .as_table_mut()
        .expect("[mcp] is a table")
        .entry("servers")
        .or_insert_with(|| Item::Table(implicit_table()));
    servers.as_table_mut().expect("[mcp.servers] is a table")
}

/// A table that only appears as a prefix (`[mcp.servers.x]`), never as its own header.
fn implicit_table() -> Table {
    let mut table = Table::new();
    table.set_implicit(true);
    table
}

/// Write the edited config, refusing anything Lucy couldn't load.
fn write_document(path: &Path, doc: &DocumentMut) -> Result<()> {
    let text = doc.to_string();
    lucy_core::config::check_writable(path)?;
    Config::from_toml_str(&text).context("the edited config would be invalid")?;
    std::fs::write(path, text).with_context(|| format!("couldn't write {}", path.display()))
}

/// How a freshly added server's first start went.
pub enum Checked {
    Ok,
    /// A remote server that wants `lucy mcp login`.
    NeedsLogin,
    Failed,
}

/// Start a newly added server once and report its tools (or what went wrong).
pub async fn check(name: &str, server: &McpServer) -> Result<Checked> {
    let secrets = SystemSecrets::system();
    let key_names = server
        .env_keys
        .values()
        .chain(server.header_keys.values())
        .chain(&server.bearer_key);
    for key_name in key_names {
        if secrets.resolve(key_name)?.is_none() {
            println!(
                "  It needs keyring entry `{key_name}`: run `lucy auth set {key_name}` (or set {})",
                secrets::env_var_name(key_name)
            );
            return Ok(Checked::Failed);
        }
    }
    println!("Connecting to `{name}` to check it…");
    let host = McpHost::with_token_store(Arc::new(OsKeyring));
    let checked = match host.server(name, server, &secrets).await {
        Ok(running) => {
            let names: Vec<String> = running
                .tools()
                .iter()
                .map(|t| format!("{name}__{}", t.name))
                .collect();
            println!("✓ {} tools: {}", names.len(), names.join(", "));
            Checked::Ok
        }
        Err(err) if err.contains("lucy mcp login") => {
            println!("  `{name}` needs you to sign in.");
            Checked::NeedsLogin
        }
        Err(err) => {
            println!(
                "✗ {}\n  The entry is saved; fix it and run `lucy mcp list`.",
                visible(&err.to_string())
            );
            Checked::Failed
        }
    };
    host.shutdown().await;
    Ok(checked)
}

/// `lucy mcp login`: sign in through the browser and keep the tokens in the keyring.
pub async fn login(name: &str, server: &McpServer) -> Result<()> {
    oauth::login(name, server, Arc::new(OsKeyring), |url| {
        println!(
            "Opening your browser to sign in to `{name}`. If it doesn't open, visit:\n  {url}"
        );
        oauth::open_browser(url);
    })
    .await
    .map_err(anyhow::Error::msg)?;
    println!("✓ Signed in to `{name}`. New chats can use its tools.");
    check(name, server).await.map(|_| ())
}

/// `lucy mcp logout`: forget a server's tokens.
pub fn logout(name: &str) -> Result<()> {
    let store: oauth::TokenStore = Arc::new(OsKeyring);
    if oauth::logout(&store, name).map_err(anyhow::Error::msg)? {
        println!("Signed out of `{name}`.");
    } else {
        println!("`{name}` had no stored sign-in.");
    }
    Ok(())
}

/// Configured servers, with their tool counts or problems as the core sees them.
pub async fn list(config: &Config) -> Result<()> {
    if config.mcp.servers.is_empty() {
        println!("No MCP servers yet. Add one with `lucy mcp add`.");
        return Ok(());
    }
    let client = core_process::connect_local().await?;
    let status: ToolsList = client
        .call(method::TOOLS_LIST, ToolsListParams { session_id: None })
        .await?;
    for (name, server) in &config.mcp.servers {
        let state = if !server.enabled {
            "disabled".to_owned()
        } else if let Some(problem) = status.problems.iter().find(|p| p.server == *name) {
            format!("✗ {}", problem.error)
        } else {
            let count = status.tools.iter().filter(|t| t.server == *name).count();
            format!("✓ {count} tools")
        };
        println!("{name}  {state}\n    {}", server.describe());
    }
    Ok(())
}

/// `lucy mcp serve`: the built-in tools over stdin/stdout, working in the current folder.
pub async fn serve() -> Result<()> {
    let cwd = std::env::current_dir()?;
    builtin::serve_stdio(BuiltinOptions {
        cwd,
        private: PrivatePaths::defaults(dirs_home().as_deref()),
    })
    .await
    .map_err(anyhow::Error::msg)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
