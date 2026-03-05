use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::config_dir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastDiagnostic {
    pub timestamp_unix: u64,
    pub model: String,
    pub phase: String,
    pub message: String,
    pub session: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnArtifact {
    pub timestamp_unix: u64,
    pub session: String,
    pub model: String,
    pub step: usize,
    pub phase: String,
    pub prompt_mode: String,
    pub request: String,
    pub response: String,
    #[serde(default)]
    pub tool_calls: Vec<Value>,
    pub executed_any: bool,
    pub had_failures: bool,
    #[serde(default)]
    pub changed_files: Vec<String>,
}

fn diagnostics_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("diagnostics").join("last_error.json"))
}

pub fn write_last_diagnostic(diag: &LastDiagnostic) -> Result<()> {
    let path = diagnostics_file()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create diagnostics dir {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(diag)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn read_last_diagnostic() -> Option<LastDiagnostic> {
    let path = diagnostics_file().ok()?;
    let text = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_turn_artifact(session: &str, artifact: &TurnArtifact) -> Result<PathBuf> {
    let safe_session = sanitize_session_name(session);
    let dir = config_dir()?.join("artifacts").join(safe_session);
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;

    let filename = format!(
        "{:04}-{}-{}.json",
        artifact.step,
        sanitize_file_part(&artifact.phase),
        artifact.timestamp_unix
    );
    let path = dir.join(filename);
    let text = serde_json::to_string_pretty(artifact)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

pub fn now_unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sanitize_session_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "session".to_string()
    } else {
        s
    }
}

fn sanitize_file_part(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "step".to_string() } else { s }
}
