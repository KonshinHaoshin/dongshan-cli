use std::path::Path;

use anyhow::Result;

use crate::fs_tools::{grep_output, list_files_output, read_text_file};

pub fn read_file(path: &Path) -> Result<String> {
    read_text_file(path)
}

pub fn list_files(path: &Path) -> Result<String> {
    list_files_output(path)
}

pub fn grep_files(path: &Path, pattern: &str) -> Result<String> {
    grep_output(path, pattern)
}
