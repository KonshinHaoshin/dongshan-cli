use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::{AutoExecMode, ProviderPreset};

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
    /// Non-interactive one-shot agent run
    Agent {
        /// Task for the agent to execute
        task: String,
        /// Session name to persist run history
        #[arg(long, default_value = "default")]
        session: String,
    },
    /// Interactive multi-turn chat
    Chat {
        /// Session name to persist chat history
        #[arg(long, default_value = "default")]
        session: String,
    },
    /// Local web console for prompt/model/policy management
    Web {
        /// Listen port
        #[arg(long, default_value_t = 3721)]
        port: u16,
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
    /// Manage available models and active model
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    /// Diagnose current model/profile/network health
    Doctor,
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
        /// Automatically check new version on startup
        #[arg(long)]
        auto_check_update: Option<bool>,
        /// Command auto-exec policy: safe | all | custom
        #[arg(long, value_enum)]
        auto_exec_mode: Option<AutoExecMode>,
        /// Comma-separated allowlist for `custom` mode, e.g. "rg,ls,git status"
        #[arg(long)]
        auto_exec_allow: Option<String>,
        /// Comma-separated denylist (highest priority), e.g. "rm,del,git reset"
        #[arg(long)]
        auto_exec_deny: Option<String>,
        /// Ask before running non-trusted commands in chat
        #[arg(long)]
        auto_confirm_exec: Option<bool>,
        /// Comma-separated trusted command prefixes, e.g. "rg,grep,git status"
        #[arg(long)]
        auto_exec_trusted: Option<String>,
        /// Maximum number of chat messages kept before compaction
        #[arg(long)]
        history_max_messages: Option<usize>,
        /// Maximum total chat characters kept before compaction
        #[arg(long)]
        history_max_chars: Option<usize>,
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

#[derive(Subcommand, Debug)]
pub enum ModelsCommand {
    /// List saved model catalog and current active model
    List,
    /// Use one model as current active model
    Use { name: String },
    /// Add a model to local catalog
    Add {
        name: String,
        /// Optional custom base URL for this model profile
        #[arg(long)]
        base_url: Option<String>,
        /// Optional env var name for API key of this model profile
        #[arg(long)]
        api_key_env: Option<String>,
        /// Optional API key stored in config for this model profile
        #[arg(long)]
        api_key: Option<String>,
    },
    /// Remove a model from local catalog
    Remove { name: String },
    /// Show one model profile (or current model when omitted)
    Show { name: Option<String> },
    /// Set profile for one model
    SetProfile {
        name: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
    },
}

