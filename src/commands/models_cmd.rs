use anyhow::{Result, bail};

use crate::cli::ModelsCommand;
use crate::config::{
    add_model_with_active_profile, ensure_model_catalog, load_config_or_default, remove_model,
    save_config, set_active_model,
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
        ModelsCommand::Add { name } => {
            add_model_with_active_profile(&mut cfg, &name);
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
    }

    Ok(())
}
