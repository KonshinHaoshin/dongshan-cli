mod chat;
mod chat_context;
mod cli;
mod commands;
mod config;
mod fs_tools;
mod llm;
mod updater;
mod util;

use anyhow::Result;
use clap::Parser;

use crate::chat::run_chat;
use crate::cli::{Cli, Commands};
use crate::commands::{
    handle_config, handle_fs, handle_models, handle_prompt, run_edit, run_onboard, run_review,
};
use crate::config::load_config_or_default;
use crate::updater::maybe_check_update;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let startup_cfg = load_config_or_default()?;
    let _ = maybe_check_update(&startup_cfg).await;

    match cli.command {
        Commands::Onboard => run_onboard().await?,
        Commands::Chat { session } => {
            let cfg = load_config_or_default()?;
            run_chat(cfg, &session).await?;
        }
        Commands::Config { command } => handle_config(command)?,
        Commands::Prompt { command } => handle_prompt(command)?,
        Commands::Models { command } => handle_models(command)?,
        Commands::Fs { command } => handle_fs(command)?,
        Commands::Review { file, prompt } => {
            let cfg = load_config_or_default()?;
            run_review(&cfg, &file, prompt).await?;
        }
        Commands::Edit {
            file,
            instruction,
            apply,
        } => {
            let cfg = load_config_or_default()?;
            run_edit(&cfg, &file, &instruction, apply).await?;
        }
    }

    Ok(())
}
