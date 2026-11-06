//! `lucy mcp`: add, list and remove external MCP servers, and serve the built-in tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lucy_core::config::{Config, McpServer, validate_server_name};
use lucy_core::mcp::McpHost;
use lucy_core::secrets::{self, SystemSecrets};
use lucy_protocol::rpc::{ToolsList, ToolsListParams, method};
use lucy_tools::PrivatePaths;
use lucy_tools::builtin::{self, BuiltinOptions};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, value};

use crate::core_process;
use crate::wizard::Prompter;

#[derive(Debug, Default, Clone)]
pub struct AddArgs {
    pub name: Option<String>,
    /// The program and its arguments; empty means ask.
    pub command: Vec<String>,
    /// `VAR=keyname`: fill `VAR` from the keyring entry `keyname`.
    pub keys: Vec<String>,
    /// `VAR=value`: plain environment variables.
    pub env: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Turn flags into a server entry, asking for the name and command if they're missing. When
/// it had to ask, it also asks which variables come from the keyring.
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

    let command = if args.command.is_empty() {
        asked = true;
        let line = prompter.input(
            "Command that starts it (e.g. npx -y @modelcontextprotocol/server-github)",
            None,
        )?;
        shell_words::split(&line).context("couldn't split that command line")?
    } else {
        args.command
    };
    let Some((program, rest)) = command.split_first().filter(|(p, _)| !p.trim().is_empty()) else {
        bail!("the server needs a command to start it");
    };

    let mut keys = args.keys;
    if asked {
        loop {
            let entry = prompter.input(
                "Environment variable to fill from the keyring, as VAR=keyname (leave empty \
                 when done)",
                Some(""),
            )?;
            if entry.trim().is_empty() {
                break;
            }
            keys.push(entry.trim().to_owned());
        }
    }

    Ok((
        name,
        McpServer {
            command: program.clone(),
            args: rest.to_vec(),
            env: pairs(&args.env, "VAR=value")?,
            env_keys: pairs(&keys, "VAR=keyname")?,
            cwd: args.cwd,
            enabled: true,
        },
    ))
}

fn pairs(items: &[String], shape: &str) -> Result<BTreeMap<String, String>> {
    items
        .iter()
        .map(|item| match item.split_once('=') {
            Some((var, val)) if is_env_name(var) && !val.is_empty() => {
                Ok((var.to_owned(), val.to_owned()))
            }
            _ => bail!("`{item}` should look like {shape}"),
        })
        .collect()
}

fn is_env_name(var: &str) -> bool {
    !var.is_empty() && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Add `[mcp.servers.<name>]` to the config file, keeping its comments and layout.
pub fn add_to_config(path: &Path, name: &str, server: &McpServer) -> Result<()> {
    let mut doc = read_document(path)?;
    let servers = servers_table(&mut doc);
    if servers.contains_key(name) {
        bail!("there's already an MCP server named `{name}`; remove it first");
    }
    let mut table = Table::new();
    table["command"] = value(&server.command);
    if !server.args.is_empty() {
        table["args"] = value(server.args.iter().collect::<Array>());
    }
    for (key, map) in [("env", &server.env), ("env_keys", &server.env_keys)] {
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
    Config::from_toml_str(&text).context("the edited config would be invalid")?;
    std::fs::write(path, text).with_context(|| format!("couldn't write {}", path.display()))
}

/// Start a newly added server once and report its tools (or what went wrong).
pub async fn check(name: &str, server: &McpServer) -> Result<()> {
    for (var, key_name) in &server.env_keys {
        if SystemSecrets::system().resolve(key_name)?.is_none() {
            println!(
                "  {var} needs keyring entry `{key_name}`: run `lucy auth set {key_name}` (or set \
                 {})",
                secrets::env_var_name(key_name)
            );
            return Ok(());
        }
    }
    println!("Starting `{name}` to check it…");
    let host = McpHost::new();
    let result = host.server(name, server, &SystemSecrets::system()).await;
    match &result {
        Ok(running) => {
            let names: Vec<String> = running
                .tools()
                .iter()
                .map(|t| format!("{name}__{}", t.name))
                .collect();
            println!("✓ {} tools: {}", names.len(), names.join(", "));
        }
        Err(err) => println!("✗ {err}\n  The entry is saved; fix it and run `lucy mcp list`."),
    }
    host.shutdown().await;
    Ok(())
}

/// Configured servers, with their tool counts or problems as the core sees them.
pub async fn list(config: &Config) -> Result<()> {
    if config.mcp.servers.is_empty() {
        println!("No MCP servers yet. Add one with `lucy mcp add`.");
        return Ok(());
    }
    let client = core_process::connect().await?;
    let status: ToolsList = client
        .call(method::TOOLS_LIST, ToolsListParams { session_id: None })
        .await?;
    for (name, server) in &config.mcp.servers {
        let command = std::iter::once(&server.command)
            .chain(&server.args)
            .map(|word| shell_words::quote(word).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let state = if !server.enabled {
            "disabled".to_owned()
        } else if let Some(problem) = status.problems.iter().find(|p| p.server == *name) {
            format!("✗ {}", problem.error)
        } else {
            let count = status.tools.iter().filter(|t| t.server == *name).count();
            format!("✓ {count} tools")
        };
        println!("{name}  {state}\n    {command}");
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
