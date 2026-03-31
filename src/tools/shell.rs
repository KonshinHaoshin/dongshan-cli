use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::tools::spec::ToolHandler;

pub struct ShellTool;

#[derive(Deserialize)]
struct Args {
    command: String,
}

impl ToolHandler for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Run a shell command."
    }

    fn execute(&self, args: &Value) -> Result<String> {
        let args: Args = serde_json::from_value(args.clone())?;
        let output = if cfg!(windows) {
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &args.command])
                .output()
                .context("failed to launch powershell")?
        } else {
            Command::new("sh")
                .args(["-lc", &args.command])
                .output()
                .context("failed to launch shell")?
        };
        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&stderr);
        }
        Ok(text)
    }
}
