use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

pub fn augment_user_input_with_workspace_context(input: &str) -> Result<String> {
    let cwd = env::current_dir()?;
    let mut out = format!("Workspace CWD: {}\nUser request: {}", cwd.display(), input);

    if is_project_analysis_request(input) {
        let snapshot = build_project_snapshot(&cwd)?;
        out = format!(
            "Workspace CWD: {}\nAuto project snapshot:\n{}\n\nUser request: {}",
            cwd.display(),
            snapshot,
            input
        );
    }

    Ok(out)
}

fn is_project_analysis_request(input: &str) -> bool {
    let t = input.to_lowercase();
    let keys = [
        "分析这个项目",
        "分析项目",
        "审查这个项目",
        "看看这个项目",
        "analyze this project",
        "analyze the project",
        "review this project",
        "review the project",
        "look at this project",
    ];
    keys.iter().any(|k| t.contains(k))
}

fn build_project_snapshot(root: &Path) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();

    let root_entries = read_root_entries(root)?;
    lines.push("Root entries:".to_string());
    if root_entries.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for entry in root_entries.iter().take(80) {
            lines.push(format!("- {}", entry));
        }
        if root_entries.len() > 80 {
            lines.push(format!("- ... ({} more)", root_entries.len() - 80));
        }
    }

    let files = collect_files(root)?;
    lines.push(format!("Total indexed files: {}", files.len()));
    lines.push("Sample files:".to_string());
    for path in files.iter().take(120) {
        lines.push(format!("- {}", path.display()));
    }
    if files.len() > 120 {
        lines.push(format!("- ... ({} more)", files.len() - 120));
    }

    let manifests = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "pom.xml",
    ];
    lines.push("Manifest previews:".to_string());
    let mut found_manifest = false;
    for m in manifests {
        let p = root.join(m);
        if p.exists() && p.is_file() {
            found_manifest = true;
            let text = fs::read_to_string(&p).unwrap_or_else(|_| "<unreadable>".to_string());
            let preview: String = text.lines().take(80).collect::<Vec<_>>().join("\n");
            lines.push(format!("--- {} ---\n{}", m, preview));
        }
    }
    if !found_manifest {
        lines.push("- none found in workspace root".to_string());
    }

    Ok(lines.join("\n"))
}

fn read_root_entries(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let entries = fs::read_dir(root)?;
    for entry in entries {
        let entry = entry?;
        let p = entry.path();
        if is_ignored(&p) {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        if p.is_dir() {
            out.push(format!("{}/", name));
        } else {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    if let Some(from_rg) = collect_files_by_rg(root) {
        return Ok(from_rg);
    }
    collect_files_by_walk(root)
}

fn collect_files_by_rg(root: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("rg").arg("--files").arg(root).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        files.push(PathBuf::from(line));
    }
    files.sort();
    Some(files)
}

fn collect_files_by_walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if is_ignored(&path) {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                stack.push(entry.path());
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn is_ignored(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "node_modules" | "target" | ".idea" | ".vscode")
}
