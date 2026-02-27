use std::collections::BTreeSet;
use std::env;
use std::time::Duration;

use anyhow::{Result, bail};
use reqwest::Client;
use serde_json::Value;

use crate::config::{
    AutoExecMode, Config, ProviderPreset, add_model_with_active_profile, apply_preset, config_path,
    load_config_or_default, provider_model_options, save_config, set_active_model,
    update_active_model_profile,
};
use crate::prompt_store::{list_prompt_names, save_prompt};
use crate::util::ask;

pub async fn run_onboard() -> Result<()> {
    let mut cfg = load_config_or_default()?;

    println!("== dongshan onboard ==");
    println!("Config file: {}", config_path()?.display());

    println!("\nChoose provider:");
    println!("1) openai");
    println!("2) deepseek");
    println!("3) openrouter");
    println!("4) xai");
    println!("5) nvidia");
    let provider_input = ask("Provider [1-5] (default 1): ")?;
    let preset = match provider_input.trim() {
        "2" | "deepseek" => ProviderPreset::Deepseek,
        "3" | "openrouter" => ProviderPreset::Openrouter,
        "4" | "xai" => ProviderPreset::Xai,
        "5" | "nvidia" => ProviderPreset::Nvidia,
        _ => ProviderPreset::Openai,
    };
    apply_preset(&mut cfg, preset);

    let key = ask("API key (leave empty to keep existing): ")?;
    if !key.trim().is_empty() {
        cfg.api_key = Some(key.trim().to_string());
    }

    let mut model_options = provider_model_options(preset)
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    if let Some(online) = fetch_provider_models_online(preset, &cfg).await? {
        model_options = merge_unique(model_options, online);
        println!("Fetched online model list.");
    } else {
        println!("Online model list unavailable, using built-in suggestions.");
    }

    println!("\nChoose model:");
    for (idx, m) in model_options.iter().enumerate() {
        println!("{}) {}", idx + 1, m);
    }
    println!("0) custom input");
    let model_choice = ask(&format!("Model option [0-{}] (default 1): ", model_options.len()))?;
    let choice_num = model_choice.trim().parse::<usize>().unwrap_or(1);
    let selected_model = if choice_num == 0 {
        let model = ask(&format!("Model (default {}): ", cfg.model))?;
        if !model.trim().is_empty() {
            model.trim().to_string()
        } else {
            cfg.model.clone()
        }
    } else if choice_num <= model_options.len() {
        model_options[choice_num - 1].to_string()
    } else {
        cfg.model.clone()
    };
    set_active_model(&mut cfg, &selected_model);
    add_model_with_active_profile(&mut cfg, &selected_model);
    update_active_model_profile(&mut cfg);

    let nsfw = ask("Allow NSFW in dongshan local prompt flow? [Y/n]: ")?;
    if matches!(nsfw.trim().to_lowercase().as_str(), "n" | "no" | "0" | "false") {
        cfg.allow_nsfw = false;
    } else if !nsfw.trim().is_empty() {
        cfg.allow_nsfw = true;
    }

    println!("\nCommand auto-exec policy:");
    println!("1) safe   (recommended)");
    println!("2) all    (LLM decides, execute all commands)");
    println!("3) custom (you configure allow/deny later)");
    let exec_mode = ask("Policy [1-3] (default 1): ")?;
    cfg.auto_exec_mode = match exec_mode.trim() {
        "2" | "all" => AutoExecMode::All,
        "3" | "custom" => AutoExecMode::Custom,
        _ => AutoExecMode::Safe,
    };

    println!("\nPrompt profile name to use (default):");
    let prompt_names = list_prompt_names().unwrap_or_else(|_| vec!["default".to_string()]);
    println!(
        "Existing: {}",
        prompt_names.join(", ")
    );
    let active_name = ask(&format!("Active prompt name (default {}): ", cfg.active_prompt))?;
    if !active_name.trim().is_empty() {
        let name = active_name.trim().to_string();
        if !prompt_names.iter().any(|p| p == &name) {
            let text = ask(&format!("Prompt '{}' text: ", name))?;
            if text.trim().is_empty() {
                bail!("Prompt text cannot be empty for new prompt '{}'", name);
            }
            save_prompt(&name, &text)?;
        }
        cfg.active_prompt = name;
    }

    save_config(&cfg)?;
    println!("\nOnboarding finished.");
    println!("{}", toml::to_string_pretty(&cfg)?);
    println!("Note: provider APIs may still enforce their own policy checks.");
    Ok(())
}

fn merge_unique(base: Vec<String>, extra: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for v in base.into_iter().chain(extra.into_iter()) {
        let key = v.trim().to_string();
        if key.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.insert(key.clone());
        out.push(key);
    }
    out
}

async fn fetch_provider_models_online(provider: ProviderPreset, cfg: &Config) -> Result<Option<Vec<String>>> {
    let client = Client::builder().timeout(Duration::from_secs(6)).build()?;
    let (url, needs_auth) = match provider {
        ProviderPreset::Openrouter => ("https://openrouter.ai/api/v1/models".to_string(), false),
        _ => (cfg.base_url.replace("/chat/completions", "/models"), true),
    };

    let mut req = client
        .get(url)
        .header("User-Agent", "dongshan-onboard-model-fetch");
    if needs_auth
        && let Some(k) = resolve_api_key_optional(cfg)
    {
        req = req.bearer_auth(k);
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(None),
    };

    let val: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let models = extract_model_ids(&val);
    if models.is_empty() {
        Ok(None)
    } else {
        Ok(Some(models))
    }
}

fn resolve_api_key_optional(cfg: &Config) -> Option<String> {
    if let Ok(v) = env::var(&cfg.api_key_env)
        && !v.trim().is_empty()
    {
        return Some(v);
    }
    if let Some(v) = &cfg.api_key
        && !v.trim().is_empty()
    {
        return Some(v.clone());
    }
    None
}

fn extract_model_ids(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(data) = v.get("data").and_then(|x| x.as_array()) else {
        return out;
    };
    for item in data {
        if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
            out.push(id.to_string());
            continue;
        }
        if let Some(id) = item.get("name").and_then(|x| x.as_str()) {
            out.push(id.to_string());
            continue;
        }
        if let Some(id) = item.get("model").and_then(|x| x.as_str()) {
            out.push(id.to_string());
        }
    }
    out.truncate(40);
    out
}
