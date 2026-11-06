use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Input, Password, Select};
use lucy_core::config::{self, Config};
use lucy_core::secrets::{self, KeySource, OsKeyring, SecretStore, Secrets, SecretsError};

use lucy_cli::chat::Start;
use lucy_cli::{ask, core_process, mcp, terminal, wizard};

/// Lucy: a model-agnostic personal assistant.
#[derive(Parser)]
#[command(name = "lucy", version, about)]
struct Cli {
    /// Continue your most recent chat.
    #[arg(short = 'c', long = "continue", conflicts_with = "resume")]
    continue_latest: bool,
    /// Resume a saved chat by ID (any unique prefix).
    #[arg(short, long, value_name = "ID")]
    resume: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Ask a single question and print the answer (pipe-friendly).
    Ask {
        /// The prompt. Reads stdin when omitted.
        prompt: Option<String>,
        /// Approve every tool call without asking (for scripts you trust).
        #[arg(short, long)]
        yes: bool,
    },
    /// Give Lucy a task to work through on her own (agent mode), in the current folder.
    ///
    /// Exits with code 2 if the task stopped at its step or cost cap.
    Agent {
        /// The task. Reads stdin when omitted.
        task: Option<String>,
        /// How free she is: 0 supervised, 1 guarded (default), 2 workspace, 3 unrestricted.
        #[arg(short, long, value_parser = parse_level)]
        level: Option<lucy_protocol::AgentLevel>,
        /// Approve every tool call that would ask (only for tasks you trust).
        #[arg(short, long)]
        yes: bool,
    },
    /// Set up or inspect Lucy's config file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Manage provider API keys in the OS keyring.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Add, list or remove external MCP tool servers.
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Install or remove the core as a login service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Lucy's background core (started automatically when needed).
    #[command(hide = true)]
    Core {
        #[command(subcommand)]
        action: CoreAction,
    },
}

#[derive(Subcommand)]
enum CoreAction {
    /// Run the core in the foreground until it has been idle for a while.
    Run,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Run the setup wizard and write a new config.
    Init,
    /// Print where the config file lives.
    Path,
}

#[derive(Subcommand)]
enum AuthAction {
    /// Store an API key in the OS keyring. Reads stdin when piped.
    Set {
        /// The `key_name` a profile uses, e.g. `anthropic`.
        key_name: String,
    },
    /// Remove a stored API key from the OS keyring.
    Remove { key_name: String },
    /// Show where each profile's API key comes from (never prints keys).
    Status,
}

#[derive(Subcommand)]
enum McpAction {
    /// Add a server Lucy starts as a program. Asks for anything not given.
    ///
    /// Example: lucy mcp add github --key GITHUB_TOKEN=github -- npx -y @modelcontextprotocol/server-github
    Add {
        /// Short name; its tools appear as `<name>__<tool>`.
        name: Option<String>,
        /// Fill an environment variable from the keyring: VAR=keyname (see `lucy auth set`).
        #[arg(long = "key", value_name = "VAR=KEYNAME")]
        keys: Vec<String>,
        /// A plain environment variable: VAR=value.
        #[arg(long, value_name = "VAR=VALUE")]
        env: Vec<String>,
        /// Folder to start it in (default: home).
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// A remote server's address, instead of a command.
        #[arg(long)]
        url: Option<String>,
        /// Keyring entry to send as `Authorization: Bearer <key>` (remote servers).
        #[arg(long, value_name = "KEYNAME")]
        bearer_key: Option<String>,
        /// Send a header filled from the keyring: HEADER=keyname (remote servers).
        #[arg(long = "header-key", value_name = "HEADER=KEYNAME")]
        header_keys: Vec<String>,
        /// Send a plain header: HEADER=value (remote servers).
        #[arg(long = "header", value_name = "HEADER=VALUE")]
        headers: Vec<String>,
        /// The command that starts the server, after `--`.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Show configured servers and whether they start.
    List,
    /// Remove a server from the config.
    Remove { name: String },
    /// Sign in to a remote server in your browser (OAuth); tokens go in the keyring.
    Login { name: String },
    /// Forget a remote server's sign-in.
    Logout { name: String },
    /// Serve Lucy's built-in tools over stdio, for other MCP clients.
    #[command(hide = true)]
    Serve,
}

#[derive(Subcommand)]
enum ServiceAction {
    Install,
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => {
            let path = config_path()?;
            if Config::load(&path)?.is_none() {
                if !std::io::stdin().is_terminal() {
                    bail!(no_config_message(&path));
                }
                init_config(&path)?;
            }
            let start = match (cli.continue_latest, cli.resume) {
                (true, _) => Start::Continue,
                (_, Some(id)) => Start::Resume(id),
                _ => Start::New,
            };
            terminal::run(start)
        }
        Some(Command::Ask { prompt, yes }) => {
            let path = config_path()?;
            if Config::load(&path)?.is_none() {
                bail!(no_config_message(&path));
            }
            ask::run(
                prompt,
                ask::AskOptions {
                    yes,
                    ..Default::default()
                },
            )
            .map(|_| ())
        }
        Some(Command::Agent { task, level, yes }) => {
            let path = config_path()?;
            if Config::load(&path)?.is_none() {
                bail!(no_config_message(&path));
            }
            let finish = ask::run(
                task,
                ask::AskOptions {
                    yes,
                    agent: true,
                    level,
                },
            )?;
            if finish == ask::Finish::Capped {
                std::process::exit(2);
            }
            Ok(())
        }
        Some(Command::Core {
            action: CoreAction::Run,
        }) => core_process::run(),
        Some(Command::Config { action }) => match action {
            ConfigAction::Init => {
                let path = config_path()?;
                if path.exists()
                    && !Confirm::new()
                        .with_prompt(format!("{} already exists. Overwrite?", path.display()))
                        .default(false)
                        .interact()?
                {
                    return Ok(());
                }
                init_config(&path)
            }
            ConfigAction::Path => {
                println!("{}", config_path()?.display());
                Ok(())
            }
        },
        Some(Command::Auth { action }) => match action {
            AuthAction::Set { key_name } => auth_set(&key_name),
            AuthAction::Remove { key_name } => {
                if keyring_hint(OsKeyring.delete(&key_name), &key_name)? {
                    println!("Removed `{key_name}` from the OS keyring.");
                } else {
                    println!("No stored key named `{key_name}`.");
                }
                Ok(())
            }
            AuthAction::Status => auth_status(),
        },
        Some(Command::Mcp { action }) => mcp_command(action),
        Some(Command::Service { .. }) => {
            bail!("`lucy service` is not implemented yet (milestone 1)")
        }
    }
}

fn mcp_command(action: McpAction) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let path = config_path()?;
    match action {
        McpAction::Add {
            name,
            keys,
            env,
            cwd,
            url,
            bearer_key,
            header_keys,
            headers,
            command,
        } => {
            let args = mcp::AddArgs {
                name,
                command,
                keys,
                env,
                cwd,
                url,
                bearer_key,
                header_keys,
                headers,
            };
            let (name, server) = if std::io::stdin().is_terminal() {
                mcp::complete_add(args, &mut TerminalPrompter)?
            } else {
                mcp::complete_add(args, &mut NoPrompts)?
            };
            mcp::add_to_config(&path, &name, &server)?;
            println!("Added `{name}` to {}.", path.display());
            let checked = runtime.block_on(mcp::check(&name, &server))?;
            if matches!(checked, mcp::Checked::NeedsLogin) {
                if std::io::stdin().is_terminal()
                    && Confirm::new()
                        .with_prompt("Log in now?")
                        .default(true)
                        .interact()?
                {
                    runtime.block_on(mcp::login(&name, &server))?;
                } else {
                    println!("Sign in later with `lucy mcp login {name}`.");
                }
            }
            Ok(())
        }
        McpAction::List => {
            let config = Config::load(&path)?.with_context(|| no_config_message(&path))?;
            runtime.block_on(mcp::list(&config))
        }
        McpAction::Remove { name } => {
            mcp::remove_from_config(&path, &name)?;
            println!("Removed `{name}`. New chats won't offer its tools.");
            Ok(())
        }
        McpAction::Login { name } => {
            let config = Config::load(&path)?.with_context(|| no_config_message(&path))?;
            let server =
                config.mcp.servers.get(&name).with_context(|| {
                    format!("no MCP server named `{name}`; see `lucy mcp list`")
                })?;
            runtime.block_on(mcp::login(&name, server))
        }
        McpAction::Logout { name } => mcp::logout(&name),
        McpAction::Serve => runtime.block_on(mcp::serve()),
    }
}

fn parse_level(text: &str) -> Result<lucy_protocol::AgentLevel, String> {
    let number: u8 = text
        .parse()
        .map_err(|_| format!("`{text}` isn't a level; use 0, 1, 2 or 3"))?;
    lucy_protocol::AgentLevel::try_from(number)
}

fn config_path() -> Result<PathBuf> {
    config::default_path().context("could not find a config directory; set LUCY_CONFIG")
}

fn no_config_message(path: &Path) -> String {
    format!(
        "no config found at {}. Run `lucy config init` to set one up.",
        path.display()
    )
}

fn init_config(path: &Path) -> Result<()> {
    println!("No config yet. Let's pick a model for Lucy.\n");
    let config = wizard::run(&mut TerminalPrompter)?;
    config.save(path)?;

    println!("\nSaved {}", path.display());
    let profile = &config.profiles[&config.defaults.profile];
    if let Some(key_name) = &profile.key_name {
        println!("Store the API key with: lucy auth set {key_name}");
    }
    Ok(())
}

fn auth_set(key_name: &str) -> Result<()> {
    let secret = if std::io::stdin().is_terminal() {
        Password::new()
            .with_prompt(format!("API key for `{key_name}`"))
            .interact()?
    } else {
        let mut secret = String::new();
        std::io::stdin().read_to_string(&mut secret)?;
        secret.trim_end_matches(['\r', '\n']).to_owned()
    };
    if secret.is_empty() {
        bail!("no key given");
    }
    keyring_hint(OsKeyring.set(key_name, &secret), key_name)?;
    println!("Stored `{key_name}` in the OS keyring.");
    Ok(())
}

fn auth_status() -> Result<()> {
    let path = config_path()?;
    let config = Config::load(&path)?.with_context(|| no_config_message(&path))?;

    let mut key_names: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (profile, settings) in &config.profiles {
        if let Some(key_name) = &settings.key_name {
            key_names.entry(key_name).or_default().push(profile);
        }
    }
    for (server, settings) in &config.mcp.servers {
        let keys = settings
            .env_keys
            .values()
            .chain(settings.header_keys.values())
            .chain(&settings.bearer_key);
        for key_name in keys {
            key_names.entry(key_name).or_default().push(server);
        }
    }
    if key_names.is_empty() {
        println!("No profiles or MCP servers need a key.");
        return Ok(());
    }

    let secrets = Secrets::system();
    for (key_name, profiles) in key_names {
        let status = match secrets.resolve(key_name) {
            Ok(Some(key)) => match key.source {
                KeySource::Env(var) => format!("set via ${var}"),
                KeySource::Keyring => "set in OS keyring".to_owned(),
            },
            Ok(None) => format!("missing: run `lucy auth set {key_name}`"),
            Err(err) => format!("error: {err}"),
        };
        println!("{key_name} ({}): {status}", profiles.join(", "));
    }
    Ok(())
}

/// Turn "no keyring" into an error that says which env var to use instead.
fn keyring_hint<T>(result: Result<T, SecretsError>, key_name: &str) -> Result<T> {
    result.map_err(|err| match err {
        SecretsError::Unavailable(reason) => SecretsError::UnavailableFor {
            reason,
            env_var: secrets::env_var_name(key_name),
        }
        .into(),
        err => err.into(),
    })
}

struct TerminalPrompter;

impl wizard::Prompter for TerminalPrompter {
    fn select(&mut self, prompt: &str, items: &[&str], default: usize) -> Result<usize> {
        Ok(Select::new()
            .with_prompt(prompt)
            .items(items)
            .default(default)
            .interact()?)
    }

    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String> {
        let mut input = Input::<String>::new().with_prompt(prompt);
        match default {
            Some("") => input = input.allow_empty(true),
            Some(default) => input = input.default(default.to_owned()),
            None => {}
        }
        Ok(input.interact_text()?)
    }
}

/// Used when stdin isn't a terminal: anything missing has to come from the command line.
struct NoPrompts;

impl wizard::Prompter for NoPrompts {
    fn select(&mut self, prompt: &str, _items: &[&str], _default: usize) -> Result<usize> {
        bail!("can't ask \"{prompt}\" without a terminal; pass it on the command line")
    }

    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String> {
        match default {
            Some(default) => Ok(default.to_owned()),
            None => bail!("can't ask \"{prompt}\" without a terminal; pass it on the command line"),
        }
    }
}
