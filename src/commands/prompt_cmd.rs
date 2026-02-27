use anyhow::{Result, bail};

use crate::cli::PromptCommand;
use crate::config::{current_prompt_text, default_prompts, load_config_or_default, save_config};
use crate::util::truncate_preview;

pub fn handle_prompt(command: PromptCommand) -> Result<()> {
    let mut cfg = load_config_or_default()?;
    match command {
        PromptCommand::List => {
            println!("Active: {}", cfg.active_prompt);
            for (name, text) in &cfg.prompts {
                println!("- {}: {}", name, truncate_preview(text, 90));
            }
        }
        PromptCommand::Save { name, text } => {
            cfg.prompts.insert(name.clone(), text);
            if cfg.active_prompt.is_empty() {
                cfg.active_prompt = name;
            }
            save_config(&cfg)?;
            println!("Prompt saved.");
        }
        PromptCommand::Remove { name } => {
            if cfg.prompts.remove(&name).is_none() {
                bail!("Prompt not found: {name}");
            }
            if cfg.active_prompt == name {
                cfg.active_prompt = "default".to_string();
                if !cfg.prompts.contains_key("default") {
                    cfg.prompts
                        .insert("default".to_string(), default_prompts()["default"].clone());
                }
            }
            save_config(&cfg)?;
            println!("Prompt removed.");
        }
        PromptCommand::Use { name } => {
            if !cfg.prompts.contains_key(&name) {
                bail!("Prompt not found: {name}");
            }
            cfg.active_prompt = name;
            save_config(&cfg)?;
            println!("Active prompt updated.");
        }
        PromptCommand::Show => {
            let text = current_prompt_text(&cfg);
            println!("Active prompt: {}", cfg.active_prompt);
            println!("{text}");
        }
        PromptCommand::VarSet { key, value } => {
            cfg.prompt_vars.insert(key, value);
            save_config(&cfg)?;
            println!("Prompt variable saved.");
        }
        PromptCommand::VarRemove { key } => {
            cfg.prompt_vars.remove(&key);
            save_config(&cfg)?;
            println!("Prompt variable removed.");
        }
        PromptCommand::VarList => {
            if cfg.prompt_vars.is_empty() {
                println!("No prompt variables.");
            } else {
                for (k, v) in &cfg.prompt_vars {
                    println!("{k}={v}");
                }
            }
        }
    }
    Ok(())
}
