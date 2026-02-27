use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDoc {
    name: String,
    content: String,
}

const DEFAULT_PROMPT_NAME: &str = "default";
const DEFAULT_PROMPT_CONTENT: &str =
    "You are a pragmatic senior software engineer. Keep responses concise and actionable.";

fn root_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot resolve home directory")?;
    Ok(home.join(".dongshan").join("prompts"))
}

fn safe_filename(name: &str) -> String {
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
    if s.is_empty() { "prompt".to_string() } else { s }
}

fn path_for_name(name: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join(format!("{}.json", safe_filename(name))))
}

pub fn ensure_default_prompt() -> Result<()> {
    let dir = root_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    let default_path = path_for_name(DEFAULT_PROMPT_NAME)?;
    if !default_path.exists() {
        save_prompt(DEFAULT_PROMPT_NAME, DEFAULT_PROMPT_CONTENT)?;
    }
    Ok(())
}

pub fn list_prompt_names() -> Result<Vec<String>> {
    ensure_default_prompt()?;
    let dir = root_dir()?;
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let doc: PromptDoc =
            serde_json::from_str(&text).with_context(|| format!("Invalid JSON {}", path.display()))?;
        out.push(doc.name);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn list_prompts() -> Result<Vec<PromptDoc>> {
    ensure_default_prompt()?;
    let dir = root_dir()?;
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let doc: PromptDoc =
            serde_json::from_str(&text).with_context(|| format!("Invalid JSON {}", path.display()))?;
        out.push(doc);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn get_prompt(name: &str) -> Result<Option<String>> {
    ensure_default_prompt()?;
    let target = name.trim();
    let dir = root_dir()?;
    let entries = fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let doc: PromptDoc =
            serde_json::from_str(&text).with_context(|| format!("Invalid JSON {}", path.display()))?;
        if doc.name == target {
            return Ok(Some(doc.content));
        }
    }
    Ok(None)
}

pub fn save_prompt(name: &str, content: &str) -> Result<()> {
    let n = name.trim();
    if n.is_empty() {
        bail!("Prompt name cannot be empty");
    }
    let path = path_for_name(n)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let doc = PromptDoc {
        name: n.to_string(),
        content: content.to_string(),
    };
    let text = serde_json::to_string_pretty(&doc)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn remove_prompt(name: &str) -> Result<()> {
    let target = name.trim();
    if target == DEFAULT_PROMPT_NAME {
        bail!("Cannot remove default prompt");
    }
    let dir = root_dir()?;
    let entries = fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let doc: PromptDoc =
            serde_json::from_str(&text).with_context(|| format!("Invalid JSON {}", path.display()))?;
        if doc.name == target {
            fs::remove_file(&path).with_context(|| format!("Failed to remove {}", path.display()))?;
            return Ok(());
        }
    }
    bail!("Prompt not found: {}", target)
}

pub fn get_prompt_or_default(name: &str) -> Result<String> {
    if let Some(v) = get_prompt(name)? {
        return Ok(v);
    }
    Ok(DEFAULT_PROMPT_CONTENT.to_string())
}

impl PromptDoc {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn content(&self) -> &str {
        &self.content
    }
}
