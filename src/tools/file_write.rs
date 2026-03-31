use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::tools::spec::ToolHandler;

pub struct FileWriteTool;

#[derive(Deserialize)]
struct Args {
    path: PathBuf,
    content: String,
    #[serde(default)]
    overwrite: bool,
}

impl ToolHandler for FileWriteTool {
    fn name(&self) -> &'static str {
        "fs_create_file"
    }

    fn description(&self) -> &'static str {
        "Create or overwrite a file."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        if args.path.exists() && !args.overwrite {
            bail!("File already exists: {}", args.path.display());
        }
        if let Some(parent) = args.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::write(&args.path, args.content)
            .with_context(|| format!("Failed to write {}", args.path.display()))?;
        Ok(format!("Wrote {}", args.path.display()))
    }
}
