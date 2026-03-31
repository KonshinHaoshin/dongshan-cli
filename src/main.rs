#![allow(dead_code)]
#![allow(clippy::all)]

#[allow(dead_code)]
mod chat;
mod chat_context;
mod cli;
mod commands;
mod config;
mod diagnostics;
mod entrypoints;
#[allow(dead_code)]
mod fs_tools;
#[allow(dead_code)]
mod llm;
mod prompt_store;
mod query;
mod services;
mod skills;
mod state;
mod tools;
mod updater;
#[allow(dead_code)]
mod util;
mod webui;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::commands::{
    handle_config, handle_files, handle_model, handle_placeholder, handle_prompt, handle_skills,
    run_doctor, run_edit, run_onboard, run_review,
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
            entrypoints::agent::run_agent_app(&session, &task).await?;
        }
        Commands::Chat { session } => {
            entrypoints::chat::run_chat_app(&session).await?;
        }
        Commands::Web { port } => run_web(port).await?,
        Commands::Config { command } => handle_config(command)?,
        Commands::Prompt { command } => handle_prompt(command)?,
        Commands::Model { command } => handle_model(command)?,
        Commands::Skills { command } => handle_skills(command)?,
        Commands::Doctor => run_doctor().await?,
        Commands::Files { command } => handle_files(command)?,
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
        Commands::Permissions => handle_placeholder("permissions")?,
        Commands::Tasks => handle_placeholder("tasks")?,
        Commands::Resume => handle_placeholder("resume")?,
        Commands::Plan => handle_placeholder("plan")?,
    }

    Ok(())
}
