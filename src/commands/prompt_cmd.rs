use anyhow::{Result, bail};

use crate::cli::PromptCommand;
use crate::config::{current_prompt_text, load_config_or_default, save_config};
use crate::prompt_store::{list_prompt_names, remove_prompt, save_prompt};
use crate::util::truncate_preview;

pub fn handle_prompt(command: PromptCommand) -> Result<()> {
    let mut cfg = load_config_or_default()?;
    match command {
        PromptCommand::List => {
            println!("Active: {}", cfg.active_prompt);
            for name in list_prompt_names()? {
                let text = if name == cfg.active_prompt {
                    current_prompt_text(&cfg)
                } else {
                    String::new()
                };
                let preview = if text.is_empty() {
                    "(stored in prompts folder)".to_string()
                } else {
                    truncate_preview(&text, 90)
                };
                println!("- {}: {}", name, preview);
            }
        }
        PromptCommand::Save { name, text } => {
            save_prompt(&name, &text)?;
            if cfg.active_prompt.is_empty() {
                cfg.active_prompt = name;
            }
            save_config(&cfg)?;
            println!("Prompt saved.");
        }
        PromptCommand::Remove { name } => {
            remove_prompt(&name)?;
            if cfg.active_prompt == name {
                cfg.active_prompt = "default".to_string();
            }
            save_config(&cfg)?;
            println!("Prompt removed.");
        }
        PromptCommand::Use { name } => {
            if !list_prompt_names()?.iter().any(|p| p == &name) {
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
