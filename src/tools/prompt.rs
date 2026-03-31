use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::services::{prompts, settings};
use crate::tools::spec::ToolHandler;

pub struct PromptTool;

#[derive(Deserialize)]
struct Args {
    action: String,
    name: Option<String>,
    content: Option<String>,
}

impl ToolHandler for PromptTool {
    fn name(&self) -> &'static str {
        "prompt"
    }

    fn description(&self) -> &'static str {
        "Manage prompts."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        match args.action.as_str() {
            "list" => Ok(prompts::list_names()?.join("\n")),
            "show" => {
                let cfg = settings::load()?;
                Ok(prompts::current_text(&cfg))
            }
            "save" => {
                let name = args.name.ok_or_else(|| anyhow::anyhow!("missing prompt name"))?;
                let content = args
                    .content
                    .ok_or_else(|| anyhow::anyhow!("missing prompt content"))?;
                prompts::save(&name, &content)?;
                Ok(format!("Saved prompt {}", name))
            }
            _ => bail!("Unsupported prompt action: {}", args.action),
        }
    }
}
