use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn read_text_file(path: &Path) -> Result<String> {
    if !path.exists() {
        bail!("File does not exist: {}", path.display());
    }
    let text = fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(text)
}

pub fn try_rg_files(path: &Path) -> Result<bool> {
    let output = Command::new("rg").arg("--files").arg(path).output();
    let Ok(output) = output else {
        return Ok(false);
    };
    if !output.status.success() {
        return Ok(false);
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(true)
}

pub fn try_rg_grep(path: &Path, pattern: &str) -> Result<bool> {
    let output = Command::new("rg").arg("-n").arg(pattern).arg(path).output();
    let Ok(output) = output else {
        return Ok(false);
    };
    if !output.status.success() {
        return Ok(false);
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(true)
}

pub fn list_files_output(path: &Path) -> Result<String> {
    if let Some(out) = rg_files_output(path)? {
        return Ok(out);
    }
    list_files_recursive_output(path)
}

pub fn grep_output(path: &Path, pattern: &str) -> Result<String> {
    if let Some(out) = rg_grep_output(path, pattern)? {
        return Ok(out);
    }
    grep_recursive_output(path, pattern)
}

pub fn rg_files_output(path: &Path) -> Result<Option<String>> {
    let output = Command::new("rg").arg("--files").arg(path).output();
    let Ok(output) = output else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

pub fn rg_grep_output(path: &Path, pattern: &str) -> Result<Option<String>> {
    let output = Command::new("rg").arg("-n").arg(pattern).arg(path).output();
    let Ok(output) = output else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

pub fn list_files_recursive(root: &Path) -> Result<()> {
    let out = list_files_recursive_output(root)?;
    print!("{out}");
    Ok(())
}

pub fn grep_recursive(root: &Path, pattern: &str) -> Result<()> {
    let out = grep_recursive_output(root, pattern)?;
    print!("{out}");
    Ok(())
}

fn list_files_recursive_output(root: &Path) -> Result<String> {
    if !root.exists() {
        bail!("Path does not exist: {}", root.display());
    }
    let mut out = String::new();
    for entry in walk(root)? {
        if entry.is_file() {
            out.push_str(&format!("{}\n", entry.display()));
        }
    }
    Ok(out)
}

fn grep_recursive_output(root: &Path, pattern: &str) -> Result<String> {
    if !root.exists() {
        bail!("Path does not exist: {}", root.display());
    }
    let pattern_lower = pattern.to_lowercase();
    let mut out = String::new();
    for entry in walk(root)? {
        if !entry.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&entry) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&pattern_lower) {
                out.push_str(&format!("{}:{}:{}\n", entry.display(), idx + 1, line.trim()));
            }
        }
    }
    Ok(out)
}

fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if is_ignored_dir(&path) {
            continue;
        }
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to read metadata {}", path.display()))?;
        if metadata.is_dir() {
            let entries = fs::read_dir(&path)
                .with_context(|| format!("Failed to read dir {}", path.display()))?;
            for entry in entries {
                let entry = entry?;
                stack.push(entry.path());
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn is_ignored_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "node_modules" | "target" | ".idea" | ".vscode")
}
