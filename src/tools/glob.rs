use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::services::files;
use crate::tools::spec::ToolHandler;

pub struct GlobTool;

#[derive(Deserialize)]
struct Args {
    #[serde(default = "default_path")]
    path: PathBuf,
}

fn default_path() -> PathBuf {
    PathBuf::from(".")
}

impl ToolHandler for GlobTool {
    fn name(&self) -> &'static str {
        "fs_list_files"
    }

    fn description(&self) -> &'static str {
        "List files recursively."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        files::list_files(&args.path)
    }
}
