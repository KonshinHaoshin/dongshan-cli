use std::path::Path;

use anyhow::Result;

use crate::config::{Config, build_system_prompt};
use crate::fs_tools::read_text_file;
use crate::llm::call_llm;

pub async fn run_review(cfg: &Config, file: &Path, extra_prompt: Option<String>) -> Result<()> {
    let code = read_text_file(file)?;
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("txt");

    let mut user_prompt = format!(
        "Please review this code. Focus on correctness, bugs, risks, and missing tests.\n\
         Provide concise findings with severity and actionable suggestions.\n\n\
         File: {}\n```{}\n{}\n```",
        file.display(),
        ext,
        code
    );

    if let Some(p) = extra_prompt {
        user_prompt.push_str("\n\nExtra requirement:\n");
        user_prompt.push_str(&p);
    }

    let answer = call_llm(cfg, &build_system_prompt(cfg, "review"), &user_prompt).await?;

    println!("{answer}");
    Ok(())
}
