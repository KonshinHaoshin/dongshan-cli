use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::ProviderPreset;

#[derive(Parser, Debug)]
#[command(name = "dongshan", version, about = "A simple AI coding CLI in Rust")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Interactive onboarding for provider/api key/prompt selection
    Onboard,
    /// Interactive multi-turn chat
    Chat {
        /// Session name to persist chat history
        #[arg(long, default_value = "default")]
        session: String,
    },
    /// Manage API settings
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage saved prompts and active prompt
    Prompt {
        #[command(subcommand)]
        command: PromptCommand,
    },
    /// Basic file system tools (read/list/grep)
    Fs {
        #[command(subcommand)]
        command: FsCommand,
    },
    /// Review a single file with AI
    Review {
        /// Target source file path
        file: PathBuf,
        /// Extra requirement for the review
        #[arg(short, long)]
        prompt: Option<String>,
    },
    /// Edit a single file with AI instruction
    Edit {
        /// Target source file path
        file: PathBuf,
        /// Instruction for the code edit
        #[arg(short, long)]
        instruction: String,
        /// Write edited content back to the file
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Initialize default config
    Init,
    /// Show current config
    Show,
    /// Use a built-in provider preset
    Use {
        #[arg(value_enum)]
        provider: ProviderPreset,
    },
    /// Set config fields manually
    Set {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        /// Local content filter switch in dongshan (does not override provider policy)
        #[arg(long)]
        allow_nsfw: Option<bool>,
    },
}

#[derive(Subcommand, Debug)]
pub enum PromptCommand {
    /// List saved prompts
    List,
    /// Add or update a prompt
    Save { name: String, text: String },
    /// Remove a prompt
    Remove { name: String },
    /// Set active prompt
    Use { name: String },
    /// Show active prompt content
    Show,
    /// Set template variable used in prompt text, e.g. {{tone}}
    VarSet { key: String, value: String },
    /// Remove prompt template variable
    VarRemove { key: String },
    /// List prompt template variables
    VarList,
}

#[derive(Subcommand, Debug)]
pub enum FsCommand {
    /// Read a text file
    Read { file: PathBuf },
    /// Recursively list files under a path
    List {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Grep a pattern in files recursively
    Grep {
        pattern: String,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

