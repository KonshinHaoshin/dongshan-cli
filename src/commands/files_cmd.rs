use anyhow::Result;

use crate::cli::FilesCommand;
use crate::services::files as files_service;

pub fn handle_files(command: FilesCommand) -> Result<()> {
    match command {
        FilesCommand::Read { file } => {
            println!("{}", files_service::read_file(&file)?);
        }
        FilesCommand::List { path } => {
            print!("{}", files_service::list_files(&path)?);
        }
        FilesCommand::Grep { pattern, path } => {
            print!("{}", files_service::grep_files(&path, &pattern)?);
        }
    }
    Ok(())
}
