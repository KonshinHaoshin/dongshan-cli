use anyhow::{Result, bail};

use crate::cli::ModelCommand;
use crate::services::models as model_service;
use crate::services::settings;

pub fn handle_model(command: ModelCommand) -> Result<()> {
    let mut cfg = settings::load()?;
    model_service::ensure_catalog(&mut cfg);

    match command {
        ModelCommand::List => {
            println!("Current model: {}", cfg.model);
            println!("Catalog:");
            for model in &cfg.model_catalog {
                let mark = if *model == cfg.model { "*" } else { " " };
                println!("{mark} {model}");
            }
        }
        ModelCommand::Use { name } => {
            if !cfg.model_catalog.iter().any(|m| m == &name) {
                bail!(
                    "Model not in catalog: {}. Use `dongshan model add {}` first.",
                    name,
                    name
                );
            }
            model_service::set_active(&mut cfg, &name);
            settings::save(&cfg)?;
            println!("Active model switched to {}", name);
        }
        ModelCommand::Add {
            name,
            provider,
            base_url,
            api_key_env,
            api_key,
        } => {
            model_service::add_with_profile(&mut cfg, &name);
            if provider.is_some()
                || base_url.is_some()
                || api_key_env.is_some()
                || api_key.is_some()
            {
                model_service::upsert_profile(
                    &mut cfg,
                    &name,
                    base_url,
                    api_key_env,
                    api_key,
                    provider,
                );
            }
            settings::save(&cfg)?;
            println!("Model added: {}", name);
        }
        ModelCommand::Remove { name } => {
            if name == cfg.model {
                bail!("Cannot remove active model: {}", name);
            }
            if !model_service::remove(&mut cfg, &name) {
                bail!("Model not found in catalog: {}", name);
            }
            settings::save(&cfg)?;
            println!("Model removed: {}", name);
        }
        ModelCommand::Show { name } => {
            let target = name.unwrap_or_else(|| cfg.model.clone());
            let Some(profile) = cfg.model_profiles.get(&target) else {
                bail!("Model profile not found: {}", target);
            };
            println!("Model: {}", target);
            println!("  provider: {:?}", profile.provider);
            println!("  tool_mode: {:?}", profile.tool_mode);
            println!("  base_url: {}", profile.base_url);
            println!("  api_key_env: {}", profile.api_key_env);
            println!(
                "  api_key: {}",
                if profile.api_key.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                    "(set)"
                } else {
                    "(empty)"
                }
            );
            println!("  active: {}", if target == cfg.model { "yes" } else { "no" });
        }
        ModelCommand::SetProfile {
            name,
            provider,
            base_url,
            api_key_env,
            api_key,
        } => {
            if provider.is_none()
                && base_url.is_none()
                && api_key_env.is_none()
                && api_key.is_none()
            {
                bail!(
                    "Nothing to set. Provide at least one of --provider/--base-url/--api-key-env/--api-key."
                );
            }
            model_service::upsert_profile(
                &mut cfg,
                &name,
                base_url,
                api_key_env,
                api_key,
                provider,
            );
            settings::save(&cfg)?;
            println!("Profile updated for model: {}", name);
        }
    }

    Ok(())
}
