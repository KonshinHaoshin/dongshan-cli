use anyhow::Result;

use crate::cli::ConfigCommand;
use crate::config::{
    Config, apply_preset, config_path, ensure_model_catalog, load_config_or_default, save_config,
    set_active_model, update_active_model_profile,
};

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
            auto_check_update,
            auto_exec_mode,
            auto_exec_allow,
            auto_exec_deny,
            auto_confirm_exec,
            auto_exec_trusted,
        } => {
            let mut cfg = load_config_or_default()?;
            if let Some(v) = model {
                set_active_model(&mut cfg, &v);
            }
            if let Some(v) = base_url {
                cfg.base_url = v;
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
            update_active_model_profile(&mut cfg);
            if let Some(v) = allow_nsfw {
                cfg.allow_nsfw = v;
            }
            if let Some(v) = auto_check_update {
                cfg.auto_check_update = v;
            }
            if let Some(v) = auto_exec_mode {
                cfg.auto_exec_mode = v;
            }
            if let Some(v) = auto_exec_allow {
                cfg.auto_exec_allow = parse_csv_list(&v);
            }
            if let Some(v) = auto_exec_deny {
                cfg.auto_exec_deny = parse_csv_list(&v);
            }
            if let Some(v) = auto_confirm_exec {
                cfg.auto_confirm_exec = v;
            }
            if let Some(v) = auto_exec_trusted {
                cfg.auto_exec_trusted = parse_csv_list(&v);
            }
            ensure_model_catalog(&mut cfg);
            save_config(&cfg)?;
            println!("Config updated:");
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
    }

    Ok(())
}

fn parse_csv_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_string())
        .collect()
}

