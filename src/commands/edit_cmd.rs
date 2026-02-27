use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::config::{Config, build_system_prompt};
use crate::fs_tools::read_text_file;
use crate::llm::call_llm;
use crate::util::backup_path;

pub async fn run_edit(cfg: &Config, file: &Path, instruction: &str, apply: bool) -> Result<()> {
    let original = read_text_file(file)?;
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("txt");

    let prompt = format!(
        "Edit this file according to the instruction.\n\
         Return ONLY the full updated file content with no markdown and no explanation.\n\n\
         Instruction:\n{}\n\n\
         File: {}\n```{}\n{}\n```",
        instruction,
        file.display(),
        ext,
        original
    );

    let edited = call_llm(cfg, &build_system_prompt(cfg, "edit"), &prompt).await?;

    if !apply {
        println!("{edited}");
        println!("\nDry run only. Use --apply to write changes.");
        return Ok(());
    }

    let backup = backup_path(file);
    fs::write(&backup, original)?;
    fs::write(file, edited)?;

    println!("Updated {}", file.display());
    println!("Backup  {}", backup.display());
    Ok(())
}
