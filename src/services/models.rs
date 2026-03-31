#![allow(dead_code)]

use anyhow::Result;

use crate::config::{
    Config, ModelApiProvider, add_model_with_active_profile, ensure_model_catalog, remove_model,
    set_active_model, upsert_model_profile,
};

pub fn ensure_catalog(cfg: &mut Config) {
    ensure_model_catalog(cfg);
}

pub fn set_active(cfg: &mut Config, model: &str) {
    set_active_model(cfg, model);
}

pub fn add_with_profile(cfg: &mut Config, model: &str) {
    add_model_with_active_profile(cfg, model);
}

pub fn remove(cfg: &mut Config, model: &str) -> bool {
    remove_model(cfg, model)
}

pub fn upsert_profile(
    cfg: &mut Config,
    model: &str,
    base_url: Option<String>,
    api_key_env: Option<String>,
    api_key: Option<String>,
    provider: Option<ModelApiProvider>,
) {
    upsert_model_profile(cfg, model, base_url, api_key_env, api_key, provider);
}

pub fn catalog_text(cfg: &Config) -> String {
    let mut out = String::new();
    out.push_str(&format!("Current model: {}\n", cfg.model));
    for model in &cfg.model_catalog {
        let mark = if *model == cfg.model { "*" } else { " " };
        out.push_str(&format!("{mark} {model}\n"));
    }
    out
}

pub fn current_profile_text(cfg: &Config) -> Result<String> {
    let mut out = String::new();
    let profile = cfg
        .model_profiles
        .get(&cfg.model)
        .ok_or_else(|| anyhow::anyhow!("Model profile not found: {}", cfg.model))?;
    out.push_str(&format!("Model: {}\n", cfg.model));
    out.push_str(&format!("provider: {:?}\n", profile.provider));
    out.push_str(&format!("tool_mode: {:?}\n", profile.tool_mode));
    out.push_str(&format!("base_url: {}\n", profile.base_url));
    out.push_str(&format!("api_key_env: {}\n", profile.api_key_env));
    out.push_str(&format!(
        "api_key: {}\n",
        if profile.api_key.as_ref().is_some_and(|value| !value.trim().is_empty()) {
            "(set)"
        } else {
            "(empty)"
        }
    ));
    Ok(out)
}
