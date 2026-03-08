use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

// ── color helpers ────────────────────────────────────────────────────────────

pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

fn ansi(code: &str, text: &str) -> String {
    if colors_enabled() {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

pub fn color_rust(text: &str) -> String {
    ansi("38;5;208", text)
} // Ferris orange
pub fn color_blue(text: &str) -> String {
    ansi("94", text)
} // bright blue
pub fn color_green(text: &str) -> String {
    ansi("32", text)
}
pub fn color_yellow(text: &str) -> String {
    ansi("33", text)
}
pub fn color_red(text: &str) -> String {
    ansi("31", text)
}
pub fn color_cyan(text: &str) -> String {
    ansi("36", text)
}
pub fn color_dim(text: &str) -> String {
    ansi("2", text)
}
pub fn color_bold(text: &str) -> String {
    ansi("1", text)
}

/// Kept for backward-compat (was the only color helper before).
pub fn blue_label(text: &str) -> String {
    color_blue(text)
}

// ── prompts / input ──────────────────────────────────────────────────────────

pub fn ask(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("Failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read stdin")?;
    Ok(input.trim_end_matches(['\n', '\r']).to_string())
}

pub fn ask_or_eof(label: &str) -> Result<Option<String>> {
    print!("{label}");
    io::stdout().flush().context("Failed to flush stdout")?;
    let mut input = String::new();
    let n = io::stdin()
        .read_line(&mut input)
        .context("Failed to read stdin")?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(input.trim_end_matches(['\n', '\r']).to_string()))
}

pub fn tagged_prompt(tag: &str, label: &str) -> String {
    format!("{} {}", blue_label(&format!("[{}]", tag)), label)
}

// ── startup banner ───────────────────────────────────────────────────────────

const FERRIS: &str = r#"
     _~^~^~_
 \) /  o o  \ (/
   '_   ~   _'
   / '-----' \
"#;

pub fn print_startup_banner(session: &str, model: &str, exec_mode: &str) {
    let sep = "─".repeat(44);

    if colors_enabled() {
        // Crab in Rust orange
        for line in FERRIS.trim_matches('\n').lines() {
            println!("{}", color_rust(line));
        }
        println!(
            "  {}  {}",
            color_bold(&color_rust("dongshan")),
            color_dim("v0.2.0  ·  AI Coding Assistant")
        );
        println!("  {}", color_rust(&sep));
        println!("  {}  {}", color_dim("session :"), color_cyan(session));
        println!("  {}  {}", color_dim("model   :"), color_blue(model));
        println!("  {}  {}", color_dim("mode    :"), color_yellow(exec_mode));
        println!("  {}", color_rust(&sep));
        println!(
            "  {}",
            color_dim("/help · /exit · /mode · /session · /model")
        );
    } else {
        for line in FERRIS.trim_matches('\n').lines() {
            println!("{}", line);
        }
        println!("  dongshan v0.2.0  ·  AI Coding Assistant");
        println!("  {}", sep);
        println!(
            "  session : {}  model : {}  mode : {}",
            session, model, exec_mode
        );
        println!("  {}", sep);
        println!("  /help · /exit · /mode · /session · /model");
    }
    println!();
}

// ── misc string helpers ───────────────────────────────────────────────────────

pub fn truncate_preview(text: &str, max_len: usize) -> String {
    truncate_with_suffix(text, max_len, "...")
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

pub fn truncate_with_suffix(text: &str, max_chars: usize, suffix: &str) -> String {
    let mut iter = text.char_indices();
    let mut count = 0usize;
    let mut cut_at = text.len();
    while let Some((idx, _)) = iter.next() {
        if count == max_chars {
            cut_at = idx;
            break;
        }
        count += 1;
    }
    if count < max_chars {
        text.to_string()
    } else {
        format!("{}{}", &text[..cut_at], suffix)
    }
}

pub fn prefix_chars(text: &str, max_chars: usize) -> String {
    let mut iter = text.char_indices();
    let mut count = 0usize;
    let mut cut_at = text.len();
    while let Some((idx, _)) = iter.next() {
        if count == max_chars {
            cut_at = idx;
            break;
        }
        count += 1;
    }
    text[..cut_at].to_string()
}

pub fn render_markdown_terminal(text: &str, enabled: bool) -> String {
    if !enabled {
        return text.to_string();
    }

    let mut out = String::new();
    let mut in_code_block = false;
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            let lang = trimmed.trim_start_matches("```").trim();
            if in_code_block {
                if lang.is_empty() {
                    out.push_str(&color_dim("┌ code"));
                } else {
                    out.push_str(&color_dim(&format!("┌ code ({lang})")));
                }
            } else {
                out.push_str(&color_dim("└ end"));
            }
            out.push('\n');
            continue;
        }

        if in_code_block {
            out.push_str(&color_dim("│ "));
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if let Some(h) = render_heading_clean(trimmed) {
            out.push_str(&h);
            out.push('\n');
            continue;
        }
        if let Some(q) = trimmed.strip_prefix(">") {
            out.push_str(&color_dim("▏ "));
            out.push_str(&style_inline_md_clean(q.trim_start()));
            out.push('\n');
            continue;
        }
        if is_hr(trimmed) {
            out.push_str(&color_dim("────────────────────────"));
            out.push('\n');
            continue;
        }
        if let Some(li) = render_list_item_clean(trimmed) {
            out.push_str(&li);
            out.push('\n');
            continue;
        }

        out.push_str(&style_inline_md_clean(line));
        out.push('\n');
    }
    out.trim_end_matches('\n').to_string()
}

fn render_heading_clean(line: &str) -> Option<String> {
    let mut level = 0usize;
    for ch in line.chars() {
        if ch == '#' {
            level += 1;
        } else {
            break;
        }
    }
    if level == 0 {
        return None;
    }
    let text = line[level..].trim_start();
    if text.is_empty() {
        return None;
    }
    let prefix = match level {
        1 => "# ",
        2 => "## ",
        3 => "### ",
        _ => "- ",
    };
    Some(color_bold(&format!(
        "{prefix}{}",
        style_inline_md_clean(text)
    )))
}

fn render_list_item_clean(line: &str) -> Option<String> {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some(format!("* {}", style_inline_md_clean(rest)));
    }

    let mut idx = 0usize;
    let bytes = line.as_bytes();
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx > 0 && idx + 1 < bytes.len() && bytes[idx] == b'.' && bytes[idx + 1] == b' ' {
        let rest = &line[idx + 2..];
        return Some(format!("{}. {}", &line[..idx], style_inline_md_clean(rest)));
    }
    None
}

fn style_inline_md_clean(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut in_bold = false;
    let chars: Vec<char> = text.chars().collect();
    let mut idx = 0usize;
    let mut buf = String::new();

    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '`' {
            flush_md_segment_clean(&mut out, &mut buf, in_code, in_bold);
            in_code = !in_code;
            idx += 1;
            continue;
        }
        if !in_code && ch == '*' && idx + 1 < chars.len() && chars[idx + 1] == '*' {
            flush_md_segment_clean(&mut out, &mut buf, in_code, in_bold);
            in_bold = !in_bold;
            idx += 2;
            continue;
        }
        buf.push(ch);
        idx += 1;
    }

    flush_md_segment_clean(&mut out, &mut buf, in_code, in_bold);
    out
}

fn flush_md_segment_clean(out: &mut String, buf: &mut String, in_code: bool, in_bold: bool) {
    if buf.is_empty() {
        return;
    }
    if in_code {
        out.push_str(&color_cyan(buf));
    } else if in_bold {
        out.push_str(&color_bold(buf));
    } else {
        out.push_str(buf);
    }
    buf.clear();
}

#[allow(dead_code)]
fn render_heading(line: &str) -> Option<String> {
    let mut level = 0usize;
    for ch in line.chars() {
        if ch == '#' {
            level += 1;
        } else {
            break;
        }
    }
    if level == 0 {
        return None;
    }
    let text = line[level..].trim_start();
    if text.is_empty() {
        return None;
    }
    let prefix = match level {
        1 => "█ ",
        2 => "▓ ",
        3 => "▒ ",
        _ => "▪ ",
    };
    Some(color_bold(&format!("{prefix}{}", style_inline_md(text))))
}

#[allow(dead_code)]
fn render_list_item(line: &str) -> Option<String> {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some(format!("• {}", style_inline_md(rest)));
    }

    let mut idx = 0usize;
    let bytes = line.as_bytes();
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx > 0 && idx + 1 < bytes.len() && bytes[idx] == b'.' && bytes[idx + 1] == b' ' {
        let rest = &line[idx + 2..];
        return Some(format!("{}. {}", &line[..idx], style_inline_md(rest)));
    }
    None
}

fn is_hr(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    t.chars().all(|c| c == '-' || c == '*' || c == '_')
}

#[allow(dead_code)]
fn style_inline_md(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut buf = String::new();
    for ch in text.chars() {
        if ch == '`' {
            if in_code {
                out.push_str(&color_cyan(&buf));
                buf.clear();
            } else {
                out.push_str(&buf);
                buf.clear();
            }
            in_code = !in_code;
            continue;
        }
        buf.push(ch);
    }
    if in_code {
        out.push_str(&color_cyan(&buf));
    } else {
        out.push_str(&buf);
    }
    out
}

// ── working-status spinner ───────────────────────────────────────────────────

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_INTERVAL: Duration = Duration::from_millis(100);

pub struct WorkingStatus {
    label: String,
    start: Instant,
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    finished: bool,
}

impl WorkingStatus {
    pub fn start(label: impl Into<String>) -> Self {
        let label = label.into();
        let start = Instant::now();
        let done = Arc::new(AtomicBool::new(false));
        let done_flag = Arc::clone(&done);
        let label_clone = label.clone();
        let use_color = colors_enabled();

        let handle = thread::spawn(move || {
            let mut frame = 0usize;
            while !done_flag.load(Ordering::Relaxed) {
                let secs = start.elapsed().as_secs();
                let spin = SPINNER[frame % SPINNER.len()];
                if use_color {
                    print!(
                        "\r{} {}  ",
                        format!("\x1b[36m{}\x1b[0m", spin),
                        format!("\x1b[2m{} {}s\x1b[0m", label_clone, secs)
                    );
                } else {
                    print!("\r{} {} {}s  ", spin, label_clone, secs);
                }
                let _ = io::stdout().flush();
                thread::sleep(SPINNER_INTERVAL);
                frame += 1;
            }
        });

        Self {
            label,
            start,
            done,
            handle: Some(handle),
            finished: false,
        }
    }

    pub fn finish(mut self) {
        self.stop_thread();
        let secs = self.start.elapsed().as_secs();
        if colors_enabled() {
            print!(
                "\r{} {}\n",
                "\x1b[32m✓\x1b[0m",
                format!("\x1b[2m{} {}s\x1b[0m", self.label, secs)
            );
        } else {
            print!("\r✓ {} {}s\n", self.label, secs);
        }
        let _ = io::stdout().flush();
        self.finished = true;
    }

    fn stop_thread(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WorkingStatus {
    fn drop(&mut self) {
        if !self.finished {
            self.stop_thread();
            print!("\r\x1b[K"); // clear the spinner line
            let _ = io::stdout().flush();
        }
    }
}

