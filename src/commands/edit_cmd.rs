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

    let report = build_change_report(&original, &edited);
    let backup = backup_path(file);
    fs::write(&backup, original)?;
    fs::write(file, edited)?;

    println!("Updated {}", file.display());
    println!("Backup  {}", backup.display());
    print_change_report(file, &report);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ChangeChunk {
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
}

#[derive(Debug, Default)]
struct ChangeReport {
    chunks: Vec<ChangeChunk>,
    inserted_lines: usize,
    deleted_lines: usize,
}

fn build_change_report(original: &str, edited: &str) -> ChangeReport {
    let old_lines: Vec<&str> = original.lines().collect();
    let new_lines: Vec<&str> = edited.lines().collect();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut chunks = Vec::new();

    while i < old_lines.len() && j < new_lines.len() {
        if old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
            continue;
        }

        let (di, dj) = find_next_anchor(&old_lines, &new_lines, i, j);
        if di.is_none() || dj.is_none() {
            chunks.push(ChangeChunk {
                old_start: i + 1,
                old_len: old_lines.len() - i,
                new_start: j + 1,
                new_len: new_lines.len() - j,
            });
            i = old_lines.len();
            j = new_lines.len();
            break;
        }

        let di = di.unwrap_or(0);
        let dj = dj.unwrap_or(0);
        if di > 0 || dj > 0 {
            chunks.push(ChangeChunk {
                old_start: i + 1,
                old_len: di,
                new_start: j + 1,
                new_len: dj,
            });
        }
        i += di;
        j += dj;
    }

    if i < old_lines.len() || j < new_lines.len() {
        chunks.push(ChangeChunk {
            old_start: i + 1,
            old_len: old_lines.len().saturating_sub(i),
            new_start: j + 1,
            new_len: new_lines.len().saturating_sub(j),
        });
    }

    let mut merged = Vec::new();
    for chunk in chunks {
        if let Some(last) = merged.last_mut()
            && are_adjacent(*last, chunk)
        {
            let last_old_end = last.old_start + last.old_len;
            let last_new_end = last.new_start + last.new_len;
            let chunk_old_end = chunk.old_start + chunk.old_len;
            let chunk_new_end = chunk.new_start + chunk.new_len;
            last.old_len = chunk_old_end.saturating_sub(last.old_start).max(last_old_end - last.old_start);
            last.new_len = chunk_new_end.saturating_sub(last.new_start).max(last_new_end - last.new_start);
            continue;
        }
        merged.push(chunk);
    }

    let inserted_lines = merged.iter().map(|c| c.new_len).sum::<usize>();
    let deleted_lines = merged.iter().map(|c| c.old_len).sum::<usize>();

    ChangeReport {
        chunks: merged,
        inserted_lines,
        deleted_lines,
    }
}

fn find_next_anchor(
    old_lines: &[&str],
    new_lines: &[&str],
    i: usize,
    j: usize,
) -> (Option<usize>, Option<usize>) {
    const LOOKAHEAD: usize = 80;
    let mut best: Option<(usize, usize, usize)> = None;
    let old_max = (old_lines.len() - i).min(LOOKAHEAD + 1);
    let new_max = (new_lines.len() - j).min(LOOKAHEAD + 1);

    for di in 0..old_max {
        for dj in 0..new_max {
            if old_lines[i + di] != new_lines[j + dj] {
                continue;
            }
            let score = di + dj;
            match best {
                None => best = Some((score, di, dj)),
                Some((best_score, _, _)) if score < best_score => best = Some((score, di, dj)),
                _ => {}
            }
        }
    }

    if let Some((_, di, dj)) = best {
        (Some(di), Some(dj))
    } else {
        (None, None)
    }
}

fn are_adjacent(left: ChangeChunk, right: ChangeChunk) -> bool {
    let left_old_end = left.old_start + left.old_len;
    let left_new_end = left.new_start + left.new_len;
    right.old_start <= left_old_end + 1 && right.new_start <= left_new_end + 1
}

fn fmt_range(start: usize, len: usize) -> String {
    if len == 0 {
        return "none".to_string();
    }
    if len == 1 {
        return format!("line {}", start);
    }
    format!("lines {}-{}", start, start + len - 1)
}

fn print_change_report(file: &Path, report: &ChangeReport) {
    println!("Changes:");
    println!("- file: {}", file.display());
    println!(
        "- hunks: {}, +{} / -{} lines",
        report.chunks.len(),
        report.inserted_lines,
        report.deleted_lines
    );

    if report.chunks.is_empty() {
        println!("- no textual changes detected");
        return;
    }

    for (idx, chunk) in report.chunks.iter().take(10).enumerate() {
        println!(
            "  {}. old {} -> new {}",
            idx + 1,
            fmt_range(chunk.old_start, chunk.old_len),
            fmt_range(chunk.new_start, chunk.new_len)
        );
    }
    if report.chunks.len() > 10 {
        println!("  ... {} more hunks", report.chunks.len() - 10);
    }
}
