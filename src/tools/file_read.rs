use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::services::files;
use crate::tools::spec::ToolHandler;

pub struct FileReadTool;

#[derive(Deserialize)]
struct Args {
    path: PathBuf,
}

impl ToolHandler for FileReadTool {
    fn name(&self) -> &'static str {
        "fs_read_file"
    }

    fn description(&self) -> &'static str {
        "Read a text file from disk."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        files::read_file(&args.path)
    }
}
