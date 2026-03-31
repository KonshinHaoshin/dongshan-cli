use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::services::{models, settings};
use crate::tools::spec::ToolHandler;

pub struct ModelTool;

#[derive(Deserialize)]
struct Args {
    action: String,
    name: Option<String>,
}

impl ToolHandler for ModelTool {
    fn name(&self) -> &'static str {
        "model"
    }

    fn description(&self) -> &'static str {
        "Inspect or switch model profiles."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        match args.action.as_str() {
            "list" => {
                let mut cfg = settings::load()?;
                models::ensure_catalog(&mut cfg);
                Ok(models::catalog_text(&cfg))
            }
            "use" => {
                let name = args.name.ok_or_else(|| anyhow::anyhow!("missing model name"))?;
                let mut cfg = settings::load()?;
                models::set_active(&mut cfg, &name);
                settings::save(&cfg)?;
                Ok(format!("Active model switched to {}", name))
            }
            _ => bail!("Unsupported model action: {}", args.action),
        }
    }
}
