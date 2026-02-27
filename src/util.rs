use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn ask(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("Failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read stdin")?;
    Ok(input.trim_end_matches(['\n', '\r']).to_string())
}

pub fn truncate_preview(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len])
    }
}

pub fn backup_path(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("backup")
        .to_string();
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let file_name = if ext.is_empty() {
        format!("{stem}.bak")
    } else {
        format!("{stem}.bak.{ext}")
    };
    file.with_file_name(file_name)
}
