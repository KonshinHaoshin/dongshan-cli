use anyhow::Result;

use crate::config::{Config, current_prompt_text};
use crate::prompt_store::{list_prompt_names, list_prompts, remove_prompt, save_prompt};

pub fn list_names() -> Result<Vec<String>> {
    list_prompt_names()
}

pub fn list_docs() -> Result<Vec<(String, String)>> {
    Ok(list_prompts()?
        .into_iter()
        .map(|doc| (doc.name().to_string(), doc.content().to_string()))
        .collect())
}

pub fn save(name: &str, content: &str) -> Result<()> {
    save_prompt(name, content)
}

pub fn remove(name: &str) -> Result<()> {
    remove_prompt(name)
}

pub fn current_text(cfg: &Config) -> String {
    current_prompt_text(cfg)
}
