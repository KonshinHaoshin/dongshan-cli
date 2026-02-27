use anyhow::{Result, bail};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

use crate::config::{load_config_or_default, resolve_api_key};

pub async fn run_doctor() -> Result<()> {
    let cfg = load_config_or_default()?;
    println!("== dongshan doctor ==");
    println!("Model: {}", cfg.model);

    let Some(profile) = cfg.model_profiles.get(&cfg.model) else {
        bail!("No profile found for current model: {}", cfg.model);
    };
    println!("Profile base_url: {}", profile.base_url);
    println!("Profile api_key_env: {}", profile.api_key_env);

    let _url =
        reqwest::Url::parse(&profile.base_url).map_err(|e| anyhow::anyhow!("Invalid base_url: {e}"))?;
    println!("[ok] base_url is valid URL");

    let api_key = resolve_api_key(&cfg)?;
    if api_key.trim().is_empty() {
        bail!("Resolved API key is empty");
    }
    println!("[ok] API key resolved");

    let client = Client::builder().timeout(Duration::from_secs(12)).build()?;

    let models_url = derive_models_url(&profile.base_url);
    let models_resp = client
        .get(&models_url)
        .bearer_auth(&api_key)
        .header("User-Agent", "dongshan-doctor")
        .send()
        .await;
    match models_resp {
        Ok(resp) if resp.status().is_success() => {
            println!("[ok] /models endpoint reachable: {}", models_url);
        }
        Ok(resp) => {
            println!(
                "[warn] /models returned status {} (some providers may not support it): {}",
                resp.status(),
                models_url
            );
        }
        Err(e) => {
            println!(
                "[warn] /models request failed (network/provider specific): {} ({})",
                models_url, e
            );
        }
    }

    let body = json!({
        "model": cfg.model,
        "messages": [{"role":"user","content":"ping"}],
        "temperature": 0,
        "max_tokens": 8
    });

    let chat_resp = client
        .post(&profile.base_url)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Chat completion request failed: {e}"))?;

    if !chat_resp.status().is_success() {
        let status = chat_resp.status();
        let text = chat_resp.text().await.unwrap_or_default();
        bail!("Chat completion failed: {} {}", status, text);
    }

    println!("[ok] chat completion test succeeded");
    println!("doctor finished: healthy");
    Ok(())
}

fn derive_models_url(base_url: &str) -> String {
    if base_url.contains("/chat/completions") {
        return base_url.replace("/chat/completions", "/models");
    }
    if base_url.ends_with("/v1") {
        return format!("{}/models", base_url);
    }
    format!("{}/models", base_url.trim_end_matches('/'))
}
