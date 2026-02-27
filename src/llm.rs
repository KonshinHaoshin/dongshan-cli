use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

use crate::config::{Config, resolve_api_key};
use crate::util::WorkingStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub async fn call_llm(cfg: &Config, system_prompt: &str, user_prompt: &str) -> Result<String> {
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: user_prompt.to_string(),
    }];
    call_llm_with_history(cfg, system_prompt, &messages).await
}

pub async fn call_llm_with_history(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<String> {
    let working = WorkingStatus::start(format!("model {}", cfg.model));
    let api_key = resolve_api_key(cfg)?;
    let mut messages = vec![json!({"role":"system","content":system_prompt})];
    for m in history {
        messages.push(json!({"role": m.role, "content": m.content}));
    }
    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.2
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .post(&cfg.base_url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Request failed: {}", cfg.base_url))?;

    let status = resp.status();
    let text = resp.text().await.context("Failed to read response body")?;

    if !status.is_success() {
        bail!("API error {}: {}", status, text);
    }

    let val: Value = serde_json::from_str(&text).context("Invalid JSON response")?;
    let content = extract_content(&val).context("Cannot parse response content")?;
    working.finish();
    Ok(content.trim().to_string())
}

fn extract_content(value: &Value) -> Option<String> {
    let content = value.get("choices")?.get(0)?.get("message")?.get("content")?;

    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("text")
                    && let Some(t) = item.get("text").and_then(|t| t.as_str())
                {
                    out.push_str(t);
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}
