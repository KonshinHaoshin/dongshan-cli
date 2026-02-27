use anyhow::{Result, bail};

use crate::config::{ProviderPreset, apply_preset, config_path, load_config_or_default, save_config};
use crate::util::ask;

pub fn run_onboard() -> Result<()> {
    let mut cfg = load_config_or_default()?;

    println!("== dongshan onboard ==");
    println!("Config file: {}", config_path()?.display());

    println!("\nChoose provider:");
    println!("1) openai");
    println!("2) deepseek");
    println!("3) openrouter");
    let provider_input = ask("Provider [1-3] (default 1): ")?;
    let preset = match provider_input.trim() {
        "2" | "deepseek" => ProviderPreset::Deepseek,
        "3" | "openrouter" => ProviderPreset::Openrouter,
        _ => ProviderPreset::Openai,
    };
    apply_preset(&mut cfg, preset);

    let model = ask(&format!("Model (default {}): ", cfg.model))?;
    if !model.trim().is_empty() {
        cfg.model = model.trim().to_string();
    }

    let key = ask("API key (leave empty to keep existing): ")?;
    if !key.trim().is_empty() {
        cfg.api_key = Some(key.trim().to_string());
    }

    let nsfw = ask("Allow NSFW in dongshan local prompt flow? [Y/n]: ")?;
    if matches!(nsfw.trim().to_lowercase().as_str(), "n" | "no" | "0" | "false") {
        cfg.allow_nsfw = false;
    } else if !nsfw.trim().is_empty() {
        cfg.allow_nsfw = true;
    }

    println!("\nPrompt profile name to use (default):");
    println!(
        "Existing: {}",
        cfg.prompts.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    let active_name = ask(&format!("Active prompt name (default {}): ", cfg.active_prompt))?;
    if !active_name.trim().is_empty() {
        let name = active_name.trim().to_string();
        if !cfg.prompts.contains_key(&name) {
            let text = ask(&format!("Prompt '{}' text: ", name))?;
            if text.trim().is_empty() {
                bail!("Prompt text cannot be empty for new prompt '{}'", name);
            }
            cfg.prompts.insert(name.clone(), text);
        }
        cfg.active_prompt = name;
    }

    save_config(&cfg)?;
    println!("\nOnboarding finished.");
    println!("{}", toml::to_string_pretty(&cfg)?);
    println!("Note: provider APIs may still enforce their own policy checks.");
    Ok(())
}

