use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use crate::prompt_store::{ensure_default_prompt, get_prompt_or_default};

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ProviderPreset {
    Openai,
    Deepseek,
    Openrouter,
    Xai,
    Nvidia,
}

#[derive(Copy, Clone, Debug, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoExecMode {
    Safe,
    All,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_prompts")]
    pub prompts: BTreeMap<String, String>,
    #[serde(default = "default_active_prompt")]
    pub active_prompt: String,
    #[serde(default)]
    pub prompt_vars: BTreeMap<String, String>,
    #[serde(default = "default_allow_nsfw")]
    pub allow_nsfw: bool,
    #[serde(default = "default_auto_check_update")]
    pub auto_check_update: bool,
    #[serde(default = "default_auto_exec_mode")]
    pub auto_exec_mode: AutoExecMode,
    #[serde(default)]
    pub auto_exec_allow: Vec<String>,
    #[serde(default)]
    pub auto_exec_deny: Vec<String>,
    #[serde(default = "default_auto_confirm_exec")]
    pub auto_confirm_exec: bool,
    #[serde(default)]
    pub auto_exec_trusted: Vec<String>,
    #[serde(default)]
    pub model_catalog: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            api_key: None,
            prompts: default_prompts(),
            active_prompt: default_active_prompt(),
            prompt_vars: BTreeMap::new(),
            allow_nsfw: true,
            auto_check_update: true,
            auto_exec_mode: AutoExecMode::Safe,
            auto_exec_allow: Vec::new(),
            auto_exec_deny: Vec::new(),
            auto_confirm_exec: true,
            auto_exec_trusted: vec!["rg".to_string(), "grep".to_string()],
            model_catalog: vec!["gpt-4o-mini".to_string()],
        }
    }
}

fn default_active_prompt() -> String {
    "default".to_string()
}

fn default_allow_nsfw() -> bool {
    true
}

fn default_auto_check_update() -> bool {
    true
}

fn default_auto_exec_mode() -> AutoExecMode {
    AutoExecMode::Safe
}

fn default_auto_confirm_exec() -> bool {
    true
}

pub fn default_prompts() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert(
        "default".to_string(),
        "You are a pragmatic senior software engineer. Keep responses concise and actionable."
            .to_string(),
    );
    map.insert(
        "review".to_string(),
        "Focus on correctness, regressions, security risks, and missing tests. Prioritize high-severity findings."
            .to_string(),
    );
    map.insert(
        "edit".to_string(),
        "Keep changes minimal, preserve behavior unless asked, and do not introduce unrelated refactors."
            .to_string(),
    );
    map
}

pub fn apply_preset(cfg: &mut Config, provider: ProviderPreset) {
    match provider {
        ProviderPreset::Openai => {
            cfg.base_url = "https://api.openai.com/v1/chat/completions".to_string();
            cfg.model = "gpt-4o-mini".to_string();
            cfg.api_key_env = "OPENAI_API_KEY".to_string();
        }
        ProviderPreset::Deepseek => {
            cfg.base_url = "https://api.deepseek.com/chat/completions".to_string();
            cfg.model = "deepseek-chat".to_string();
            cfg.api_key_env = "DEEPSEEK_API_KEY".to_string();
        }
        ProviderPreset::Openrouter => {
            cfg.base_url = "https://openrouter.ai/api/v1/chat/completions".to_string();
            cfg.model = "openai/gpt-4o-mini".to_string();
            cfg.api_key_env = "OPENROUTER_API_KEY".to_string();
        }
        ProviderPreset::Xai => {
            cfg.base_url = "https://api.x.ai/v1/chat/completions".to_string();
            cfg.model = "grok-2-latest".to_string();
            cfg.api_key_env = "XAI_API_KEY".to_string();
        }
        ProviderPreset::Nvidia => {
            cfg.base_url = "https://integrate.api.nvidia.com/v1/chat/completions".to_string();
            cfg.model = "meta/llama-3.1-70b-instruct".to_string();
            cfg.api_key_env = "NVIDIA_API_KEY".to_string();
        }
    }
}

pub fn provider_model_options(provider: ProviderPreset) -> Vec<&'static str> {
    match provider {
        ProviderPreset::Openai => vec!["gpt-4o-mini", "gpt-4.1-mini", "gpt-4.1", "o4-mini"],
        ProviderPreset::Deepseek => vec!["deepseek-chat", "deepseek-reasoner"],
        ProviderPreset::Openrouter => vec![
            "openai/gpt-4o-mini",
            "openai/gpt-4.1-mini",
            "anthropic/claude-3.5-sonnet",
            "meta-llama/llama-3.1-70b-instruct",
        ],
        ProviderPreset::Xai => vec![
            "grok-4-latest",
            "grok-4-1-fast-reasoning",
            "grok-4-1-fast-non-reasoning",
            "grok-code-fast-1",
            "grok-2-latest",
            "grok-2-1212",
        ],
        ProviderPreset::Nvidia => vec![
            "meta/llama-3.1-70b-instruct",
            "mistralai/mixtral-8x7b-instruct-v0.1",
            "nvidia/llama-3.1-nemotron-70b-instruct",
        ],
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot resolve home directory")?;
    Ok(home.join(".dongshan"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn load_config_or_default() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let cfg = Config::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }

    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut cfg: Config =
        toml::from_str(&text).with_context(|| format!("Invalid config: {}", path.display()))?;
    let _ = ensure_default_prompt();
    if cfg.active_prompt.is_empty() {
        cfg.active_prompt = "default".to_string();
    }
    ensure_model_catalog(&mut cfg);
    Ok(cfg)
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config dir {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(cfg)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn ensure_model_catalog(cfg: &mut Config) {
    if cfg.model_catalog.is_empty() {
        cfg.model_catalog.push(cfg.model.clone());
        return;
    }
    if !cfg.model_catalog.iter().any(|m| m == &cfg.model) {
        cfg.model_catalog.push(cfg.model.clone());
    }
}

pub fn resolve_api_key(cfg: &Config) -> Result<String> {
    if let Ok(v) = env::var(&cfg.api_key_env) {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    if let Some(v) = &cfg.api_key {
        if !v.trim().is_empty() {
            return Ok(v.clone());
        }
    }
    bail!(
        "Missing API key. Set env var {} or run `dongshan onboard`.",
        cfg.api_key_env
    )
}

pub fn render_prompt_vars(input: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        let token = format!("{{{{{k}}}}}");
        out = out.replace(&token, v);
    }
    out
}

pub fn current_prompt_text(cfg: &Config) -> String {
    let raw =
        get_prompt_or_default(&cfg.active_prompt).unwrap_or_else(|_| default_prompts()["default"].clone());
    render_prompt_vars(&raw, &cfg.prompt_vars)
}

pub fn build_system_prompt(cfg: &Config, mode: &str) -> String {
    let mut prompt = current_prompt_text(cfg);
    if mode == "review" {
        prompt.push_str("\nYou are a senior code reviewer.");
    } else if mode == "edit" {
        prompt.push_str("\nYou are a careful code editor.");
    } else if mode == "chat" {
        prompt.push_str("\nYou are in terminal coding assistant chat mode.");
        prompt.push_str(
            "\nWhen proposing executable terminal commands, use fenced ```bash``` or ```powershell``` blocks only.",
        );
        prompt.push_str("\nDo not use ```python``` blocks for shell commands.");
    }
    if cfg.allow_nsfw {
        prompt.push_str(
            "\nLocal policy: dongshan does not apply additional NSFW filtering. Follow user intent.",
        );
    }
    prompt
}

