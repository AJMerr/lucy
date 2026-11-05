use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Input, Select};
use lucy_core::config::{self, Config};

mod wizard;

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
    /// Install or remove the core as a login service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
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
        None => {
            let path = config_path()?;
            if Config::load(&path)?.is_none() {
                if !std::io::stdin().is_terminal() {
                    bail!(no_config_message(&path));
                }
                init_config(&path)?;
            }
            bail!("interactive chat is not implemented yet (milestone 1)")
        }
        Some(Command::Ask { .. }) => {
            let path = config_path()?;
            if Config::load(&path)?.is_none() {
                bail!(no_config_message(&path));
            }
            bail!("`lucy ask` is not implemented yet (milestone 1)")
        }
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
        Some(Command::Auth { .. }) => bail!("`lucy auth` is not implemented yet (milestone 1)"),
        Some(Command::Service { .. }) => {
            bail!("`lucy service` is not implemented yet (milestone 1)")
        }
    }
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
