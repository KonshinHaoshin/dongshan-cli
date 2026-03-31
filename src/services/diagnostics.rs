use anyhow::Result;

use crate::diagnostics::{LastDiagnostic, read_last_diagnostic};

pub fn last_error() -> Option<LastDiagnostic> {
    read_last_diagnostic()
}

pub fn render_last_error() -> Result<String> {
    Ok(match read_last_diagnostic() {
        Some(diag) => format!("[{}] {}", diag.phase, diag.message),
        None => "No diagnostics recorded.".to_string(),
    })
}
