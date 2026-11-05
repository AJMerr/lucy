use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

/// Lucy: a model-agnostic personal assistant.
#[derive(Parser)]
#[command(name = "lucy", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Ask a single question and print the answer (pipe-friendly).
    Ask {
        /// The prompt. Reads stdin when omitted.
        prompt: Option<String>,
    },
    /// Manage provider API keys in the OS keyring.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Install or remove the core as a login service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Store an API key for a provider.
    Set { provider: String },
    /// Remove a stored API key.
    Remove { provider: String },
}

#[derive(Subcommand)]
enum ServiceAction {
    Install,
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => bail!("interactive chat is not implemented yet (milestone 1)"),
        Some(Command::Ask { .. }) => bail!("`lucy ask` is not implemented yet (milestone 1)"),
        Some(Command::Auth { .. }) => bail!("`lucy auth` is not implemented yet (milestone 1)"),
        Some(Command::Service { .. }) => {
            bail!("`lucy service` is not implemented yet (milestone 1)")
        }
    }
}
