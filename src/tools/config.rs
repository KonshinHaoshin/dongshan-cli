use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::services::settings;
use crate::tools::spec::ToolHandler;

pub struct ConfigTool;

#[derive(Deserialize)]
struct Args {
    action: String,
}

impl ToolHandler for ConfigTool {
    fn name(&self) -> &'static str {
        "config"
    }

    fn description(&self) -> &'static str {
        "Inspect configuration."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        match args.action.as_str() {
            "show" => Ok(toml::to_string_pretty(&settings::load()?)?),
            _ => bail!("Unsupported config action: {}", args.action),
        }
    }
}
