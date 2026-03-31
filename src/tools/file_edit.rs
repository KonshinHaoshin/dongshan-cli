use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::tools::spec::ToolHandler;

pub struct FileEditTool;

#[derive(Deserialize)]
struct Args {
    path: PathBuf,
    old_str: String,
    new_str: String,
    #[serde(default)]
    replace_all: bool,
}

impl ToolHandler for FileEditTool {
    fn name(&self) -> &'static str {
        "fs_edit_file"
    }

    fn description(&self) -> &'static str {
        "Replace text inside an existing file."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        let original = fs::read_to_string(&args.path)
            .with_context(|| format!("Failed to read {}", args.path.display()))?;
        if !original.contains(&args.old_str) {
            bail!("Target text not found in {}", args.path.display());
        }
        let updated = if args.replace_all {
            original.replace(&args.old_str, &args.new_str)
        } else {
            original.replacen(&args.old_str, &args.new_str, 1)
        };
        fs::write(&args.path, updated)
            .with_context(|| format!("Failed to write {}", args.path.display()))?;
        Ok(format!("Edited {}", args.path.display()))
    }
}
