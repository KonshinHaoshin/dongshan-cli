use anyhow::Result;

use crate::cli::ConfigCommand;
use crate::config::{Config, apply_preset, config_path, load_config_or_default, save_config};

pub fn handle_config(command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Init => {
            let cfg = Config::default();
            save_config(&cfg)?;
            println!("Initialized config at {}", config_path()?.display());
        }
        ConfigCommand::Show => {
            let cfg = load_config_or_default()?;
            println!("{}", toml::to_string_pretty(&cfg)?);
            println!("Config path: {}", config_path()?.display());
            println!("Note: allow_nsfw is local dongshan behavior only.");
        }
        ConfigCommand::Use { provider } => {
            let mut cfg = load_config_or_default()?;
            apply_preset(&mut cfg, provider);
            save_config(&cfg)?;
            println!("Switched provider preset: {provider:?}");
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
        ConfigCommand::Set {
            base_url,
            model,
            api_key_env,
            api_key,
            allow_nsfw,
        } => {
            let mut cfg = load_config_or_default()?;
            if let Some(v) = base_url {
                cfg.base_url = v;
            }
            if let Some(v) = model {
                cfg.model = v;
            }
            if let Some(v) = api_key_env {
                cfg.api_key_env = v;
            }
            if let Some(v) = api_key {
                if v.trim().is_empty() {
                    cfg.api_key = None;
                } else {
                    cfg.api_key = Some(v);
                }
            }
            if let Some(v) = allow_nsfw {
                cfg.allow_nsfw = v;
            }
            save_config(&cfg)?;
            println!("Config updated:");
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
    }

    Ok(())
}

