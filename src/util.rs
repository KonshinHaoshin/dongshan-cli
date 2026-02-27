use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
        let label_for_thread = label.clone();

        let handle = thread::spawn(move || {
            while !done_flag.load(Ordering::Relaxed) {
                let secs = start.elapsed().as_secs();
                print!("\r(working {} {}s)", label_for_thread, secs);
                let _ = io::stdout().flush();
                thread::sleep(Duration::from_secs(1));
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
        print!("\r(done {} {}s)\n", self.label, secs);
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
            print!("\r");
            let _ = io::stdout().flush();
        }
    }
}
