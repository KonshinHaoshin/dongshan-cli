mod chat;
mod chat_context;
mod cli;
mod commands;
mod config;
mod fs_tools;
mod llm;
mod prompt_store;
mod updater;
mod util;
mod webui;

use anyhow::Result;
use clap::Parser;

use crate::chat::{run_agent_task, run_chat};
use crate::cli::{Cli, Commands};
use crate::commands::{
    handle_config, handle_fs, handle_models, handle_prompt, run_doctor, run_edit, run_onboard,
    run_review,
};
use crate::config::load_config_or_default;
use crate::updater::maybe_check_update;
use crate::webui::run_web;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let startup_cfg = load_config_or_default()?;
    let _ = maybe_check_update(&startup_cfg).await;

    match cli.command {
        Commands::Onboard => run_onboard().await?,
        Commands::Agent { task, session } => {
            let cfg = load_config_or_default()?;
            run_agent_task(cfg, &session, &task).await?;
        }
        Commands::Chat { session } => {
            let cfg = load_config_or_default()?;
            run_chat(cfg, &session).await?;
        }
        Commands::Web { port } => run_web(port).await?,
        Commands::Config { command } => handle_config(command)?,
        Commands::Prompt { command } => handle_prompt(command)?,
        Commands::Models { command } => handle_models(command)?,
        Commands::Doctor => run_doctor().await?,
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
