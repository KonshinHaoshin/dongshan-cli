use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::services::files;
use crate::tools::spec::ToolHandler;

pub struct GrepTool;

#[derive(Deserialize)]
struct Args {
    pattern: String,
    #[serde(default = "default_path")]
    path: PathBuf,
}

fn default_path() -> PathBuf {
    PathBuf::from(".")
}

impl ToolHandler for GrepTool {
    fn name(&self) -> &'static str {
        "fs_grep"
    }

    fn description(&self) -> &'static str {
        "Search file contents recursively."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        files::grep_files(&args.path, &args.pattern)
    }
}
