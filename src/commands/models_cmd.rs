use anyhow::{Result, bail};

use crate::cli::ModelsCommand;
use crate::config::{ensure_model_catalog, load_config_or_default, save_config};

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
            cfg.model = name.clone();
            save_config(&cfg)?;
            println!("Active model switched to {}", name);
        }
        ModelsCommand::Add { name } => {
            if !cfg.model_catalog.iter().any(|m| m == &name) {
                cfg.model_catalog.push(name.clone());
            }
            save_config(&cfg)?;
            println!("Model added: {}", name);
        }
        ModelsCommand::Remove { name } => {
            if name == cfg.model {
                bail!("Cannot remove active model: {}", name);
            }
            let before = cfg.model_catalog.len();
            cfg.model_catalog.retain(|m| m != &name);
            if cfg.model_catalog.len() == before {
                bail!("Model not found in catalog: {}", name);
            }
            save_config(&cfg)?;
            println!("Model removed: {}", name);
        }
    }

    Ok(())
}
