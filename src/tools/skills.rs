use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::services::skills;
use crate::tools::spec::ToolHandler;

pub struct SkillsTool;

#[derive(Deserialize)]
struct Args {
    action: String,
}

impl ToolHandler for SkillsTool {
    fn name(&self) -> &'static str {
        "skills"
    }

    fn description(&self) -> &'static str {
        "List available skills."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        match args.action.as_str() {
            "list" => Ok(skills::load_all()?
                .into_iter()
                .map(|skill| skill.manifest.name)
                .collect::<Vec<_>>()
                .join("\n")),
            _ => bail!("Unsupported skills action: {}", args.action),
        }
    }
}
