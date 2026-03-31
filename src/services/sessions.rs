#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::llm::ChatMessage;
use crate::services::settings;

pub fn sessions_dir() -> Result<PathBuf> {
    Ok(settings::root_dir()?.join("sessions"))
}

pub fn session_path(session: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{}.json", sanitize_session_name(session))))
}

pub fn load(session: &str) -> Result<Vec<ChatMessage>> {
    let path = session_path(session)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let messages = serde_json::from_str(&text)
        .with_context(|| format!("Invalid session JSON: {}", path.display()))?;
    Ok(messages)
}

pub fn save(session: &str, messages: &[ChatMessage]) -> Result<()> {
    let path = session_path(session)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(messages)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn list() -> Result<Vec<String>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn remove(session: &str) -> Result<bool> {
    let path = session_path(session)?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("Failed to remove {}", path.display()))?;
    Ok(true)
}

pub fn transcript_lines(messages: &[ChatMessage]) -> Vec<String> {
    let mut out = Vec::new();
    for message in messages {
        let role = if message.role.is_empty() {
            "assistant"
        } else {
            &message.role
        };
        if message.content.trim().is_empty() {
            continue;
        }
        out.push(format!("{role}> {}", message.content.trim()));
    }
    if out.is_empty() {
        out.push("assistant> session is empty".to_string());
    }
    out
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
        "default".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_session_name;

    #[test]
    fn sanitizes_session_names() {
        assert_eq!(sanitize_session_name("abc/def"), "abc_def");
        assert_eq!(sanitize_session_name(""), "default");
    }
}
