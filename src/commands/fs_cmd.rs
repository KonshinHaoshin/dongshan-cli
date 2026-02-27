use anyhow::Result;

use crate::cli::FsCommand;
use crate::fs_tools::{
    grep_recursive, list_files_recursive, read_text_file, try_rg_files, try_rg_grep,
};

pub fn handle_fs(command: FsCommand) -> Result<()> {
    match command {
        FsCommand::Read { file } => {
            let content = read_text_file(&file)?;
            println!("{content}");
        }
        FsCommand::List { path } => {
            if !try_rg_files(&path)? {
                list_files_recursive(&path)?;
            }
        }
        FsCommand::Grep { pattern, path } => {
            if !try_rg_grep(&path, &pattern)? {
                grep_recursive(&path, &pattern)?;
            }
        }
    }
    Ok(())
}
