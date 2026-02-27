use anyhow::{Result, bail};

use crate::cli::ModelsCommand;
use crate::config::{
    add_model_with_active_profile, ensure_model_catalog, load_config_or_default, remove_model,
    save_config, set_active_model, upsert_model_profile,
};

pub fn handle_models(command: ModelsCommand) -> Result<()> {
    let mut cfg = load_config_or_default()?;
    ensure_model_catalog(&mut cfg);

    match command {
        ModelsCommand::List => {
            println!("Current model: {}", cfg.model);
            println!("Catalog:");
            for m in &cfg.model_catalog {
                let mark = if *m == cfg.model { "*" } else { " " };
                println!("{mark} {m}");
            }
        }
        ModelsCommand::Use { name } => {
            if !cfg.model_catalog.iter().any(|m| m == &name) {
                bail!("Model not in catalog: {}. Use `dongshan models add {}` first.", name, name);
            }
            set_active_model(&mut cfg, &name);
            save_config(&cfg)?;
            println!("Active model switched to {}", name);
        }
        ModelsCommand::Add {
            name,
            base_url,
            api_key_env,
            api_key,
        } => {
            add_model_with_active_profile(&mut cfg, &name);
            if base_url.is_some() || api_key_env.is_some() || api_key.is_some() {
                upsert_model_profile(&mut cfg, &name, base_url, api_key_env, api_key);
            }
            save_config(&cfg)?;
            println!("Model added: {}", name);
        }
        ModelsCommand::Remove { name } => {
            if name == cfg.model {
                bail!("Cannot remove active model: {}", name);
            }
            if !remove_model(&mut cfg, &name) {
                bail!("Model not found in catalog: {}", name);
            }
            save_config(&cfg)?;
            println!("Model removed: {}", name);
        }
        ModelsCommand::Show { name } => {
            let target = name.unwrap_or_else(|| cfg.model.clone());
            let Some(p) = cfg.model_profiles.get(&target) else {
                bail!("Model profile not found: {}", target);
            };
            println!("Model: {}", target);
            println!("  base_url: {}", p.base_url);
            println!("  api_key_env: {}", p.api_key_env);
            println!(
                "  api_key: {}",
                if p.api_key.as_ref().is_some_and(|v| !v.trim().is_empty()) {
                    "(set)"
                } else {
                    "(empty)"
                }
            );
            println!(
                "  active: {}",
                if target == cfg.model { "yes" } else { "no" }
            );
        }
        ModelsCommand::SetProfile {
            name,
            base_url,
            api_key_env,
            api_key,
        } => {
            if base_url.is_none() && api_key_env.is_none() && api_key.is_none() {
                bail!("Nothing to set. Provide at least one of --base-url/--api-key-env/--api-key.");
            }
            upsert_model_profile(&mut cfg, &name, base_url, api_key_env, api_key);
            save_config(&cfg)?;
            println!("Profile updated for model: {}", name);
        }
    }

    Ok(())
}
