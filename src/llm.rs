use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
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
    call_llm_with_history_impl(cfg, system_prompt, history, false).await
}

pub async fn call_llm_with_history_stream(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<String> {
    call_llm_with_history_impl(cfg, system_prompt, history, true).await
}

async fn call_llm_with_history_impl(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
    stream_output: bool,
) -> Result<String> {
    let working = if stream_output {
        None
    } else {
        Some(WorkingStatus::start("waiting response"))
    };
    let api_key = resolve_api_key(cfg)?;
    let mut messages = vec![json!({"role":"system","content":system_prompt})];
    for m in history {
        messages.push(json!({"role": m.role, "content": m.content}));
    }
    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.2,
        "stream": stream_output
    });

    let timeout_secs = if stream_output { 900 } else { 120 };
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
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
    if !status.is_success() {
        let text = resp.text().await.context("Failed to read response body")?;
        bail!("API error {}: {}", status, text);
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let out = if stream_output && content_type.contains("text/event-stream") {
        parse_sse_response(resp, true).await?
    } else {
        let text = resp.text().await.context("Failed to read response body")?;
        let val: Value = serde_json::from_str(&text).context("Invalid JSON response")?;
        extract_content(&val).context("Cannot parse response content")?
    };

    if let Some(working) = working {
        working.finish();
    }
    Ok(out.trim().to_string())
}

async fn parse_sse_response(mut resp: reqwest::Response, print_live: bool) -> Result<String> {
    let mut full = String::new();
    let mut buffer = String::new();

    while let Some(chunk) = resp.chunk().await.context("Failed to read stream chunk")? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(idx) = buffer.find('\n') {
            let mut line = buffer[..idx].to_string();
            buffer = buffer[idx + 1..].to_string();
            if line.ends_with('\r') {
                line.pop();
            }
            if !line.starts_with("data:") {
                continue;
            }
            let data = line[5..].trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                return Ok(full);
            }

            let Ok(val) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let delta = extract_delta_content(&val).unwrap_or_default();
            if delta.is_empty() {
                continue;
            }
            if print_live {
                print!("{}", delta);
                let _ = io::stdout().flush();
            }
            full.push_str(&delta);
        }
    }

    Ok(full)
}

fn extract_delta_content(value: &Value) -> Option<String> {
    let content = value.get("choices")?.get(0)?.get("delta")?.get("content")?;
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
