use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::time::Duration;

use crate::config::{
    Config, ResponseFormatPolicy, ToolChoicePolicy, resolve_api_key, set_active_model,
};
use crate::util::WorkingStatus;

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<Value>,
    temperature: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub kind: String,
    pub media_type: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

#[derive(Debug, Clone)]
pub struct NativeFunctionCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct NativeLlmResponse {
    pub content: String,
    pub tool_calls: Vec<NativeFunctionCall>,
    pub assistant_message: Value,
}

pub fn build_openai_messages(system_prompt: &str, history: &[ChatMessage]) -> Vec<Value> {
    let mut messages = vec![json!({"role":"system","content":system_prompt})];
    for m in history {
        if m.content.trim().is_empty() && m.attachments.is_empty() {
            continue;
        }
        if m.attachments.is_empty() || m.role != "user" {
            messages.push(json!({"role": m.role, "content": m.content}));
            continue;
        }
        let mut content = Vec::new();
        if !m.content.trim().is_empty() {
            content.push(json!({
                "type": "text",
                "text": m.content,
            }));
        }
        for att in &m.attachments {
            if att.kind != "image" || att.data_url.trim().is_empty() {
                continue;
            }
            content.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": att.data_url,
                }
            }));
        }
        messages.push(json!({"role": m.role, "content": content}));
    }
    messages
}

pub async fn call_llm(cfg: &Config, system_prompt: &str, user_prompt: &str) -> Result<String> {
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: user_prompt.to_string(),
        attachments: Vec::new(),
    }];
    call_llm_with_history(cfg, system_prompt, &messages).await
}

pub async fn call_llm_with_history(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<String> {
    call_llm_with_history_impl(cfg, system_prompt, history, false, None, false, None).await
}

#[allow(dead_code)]
pub async fn call_llm_with_history_stream(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<String> {
    call_llm_with_history_impl(cfg, system_prompt, history, true, None, false, None).await
}

pub async fn call_llm_with_history_stream_tools(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[Value],
) -> Result<String> {
    call_llm_with_history_impl(cfg, system_prompt, history, true, Some(tools), true, None).await
}

pub async fn call_llm_with_history_json_object(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<String> {
    call_llm_with_history_impl(
        cfg,
        system_prompt,
        history,
        false,
        None,
        false,
        Some(ResponseFormatPolicy::JsonObject),
    )
    .await
}

async fn call_llm_with_history_impl(
    cfg: &Config,
    system_prompt: &str,
    history: &[ChatMessage],
    stream_output: bool,
    tools: Option<&[Value]>,
    prefer_executor_model: bool,
    response_format_override: Option<ResponseFormatPolicy>,
) -> Result<String> {
    let messages = build_openai_messages(system_prompt, history);
    let working = if stream_output { None } else { Some(WorkingStatus::start("waiting response")) };
    let candidates = request_model_candidates(cfg, prefer_executor_model);
    let mut last_err = None;
    for candidate in candidates {
        match call_single_llm_with_history(
            &candidate,
            &messages,
            stream_output,
            tools,
            response_format_override,
        )
        .await
        {
            Ok(out) => {
                if let Some(working) = working {
                    working.finish();
                }
                return Ok(out.trim().to_string());
            }
            Err(err) => last_err = Some(err),
        }
    }
    if let Some(working) = working {
        working.finish();
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No available model candidates for request")))
}

pub async fn call_llm_with_messages_native_tools(
    cfg: &Config,
    messages: &[Value],
    tools: &[Value],
) -> Result<NativeLlmResponse> {
    let candidates = request_model_candidates(cfg, true);
    let mut last_err = None;
    for candidate in candidates {
        match call_single_llm_native_tools(&candidate, messages, tools).await {
            Ok(resp) => return Ok(resp),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No available model candidates for tool request")))
}

fn request_model_candidates(cfg: &Config, prefer_executor_model: bool) -> Vec<Config> {
    let mut names = Vec::<String>::new();
    if prefer_executor_model
        && let Some(name) = cfg.executor_model.as_deref().map(str::trim)
        && !name.is_empty()
    {
        names.push(name.to_string());
    }
    names.push(cfg.model.clone());
    for name in &cfg.fallback_models {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            names.push(trimmed.to_string());
        }
    }
    let mut out = Vec::new();
    for name in names {
        if out.iter().any(|c: &Config| c.model == name) {
            continue;
        }
        let mut next = cfg.clone();
        set_active_model(&mut next, &name);
        out.push(next);
    }
    if out.is_empty() {
        out.push(cfg.clone());
    }
    out
}

async fn call_single_llm_with_history(
    cfg: &Config,
    messages: &[Value],
    stream_output: bool,
    tools: Option<&[Value]>,
    response_format_override: Option<ResponseFormatPolicy>,
) -> Result<String> {
    let request = build_openai_chat_request(
        cfg,
        messages,
        stream_output,
        tools,
        response_format_override,
    );
    let resp = send_openai_chat_request(cfg, &request, request_timeout_secs(stream_output)).await?;
    parse_openai_text_response(resp, stream_output).await
}

async fn call_single_llm_native_tools(
    cfg: &Config,
    messages: &[Value],
    tools: &[Value],
) -> Result<NativeLlmResponse> {
    let request = build_openai_chat_request(cfg, messages, false, Some(tools), None);
    let resp = send_openai_chat_request(cfg, &request, request_timeout_secs(false).max(900)).await?;
    parse_openai_native_response(resp).await
}

fn build_openai_chat_request(
    cfg: &Config,
    messages: &[Value],
    stream: bool,
    tools: Option<&[Value]>,
    response_format_override: Option<ResponseFormatPolicy>,
) -> OpenAiChatRequest {
    let response_format_policy = response_format_override.unwrap_or(cfg.response_format_policy);
    OpenAiChatRequest {
        model: cfg.model.clone(),
        messages: messages.to_vec(),
        temperature: 0.2,
        stream,
        tools: tools.map(|v| v.to_vec()),
        tool_choice: tools.and_then(|_| openai_tool_choice_value(cfg.tool_choice_policy)),
        response_format: openai_response_format_value(response_format_policy),
    }
}

fn openai_tool_choice_value(policy: ToolChoicePolicy) -> Option<Value> {
    match policy {
        ToolChoicePolicy::Auto => Some(json!("auto")),
        ToolChoicePolicy::None => Some(json!("none")),
        ToolChoicePolicy::Required => Some(json!("required")),
    }
}

fn openai_response_format_value(policy: ResponseFormatPolicy) -> Option<Value> {
    match policy {
        ResponseFormatPolicy::Text => None,
        ResponseFormatPolicy::JsonObject => Some(json!({"type":"json_object"})),
    }
}

fn request_timeout_secs(stream: bool) -> u64 {
    if stream { 900 } else { 120 }
}

async fn send_openai_chat_request(
    cfg: &Config,
    body: &OpenAiChatRequest,
    timeout_secs: u64,
) -> Result<reqwest::Response> {
    let api_key = resolve_api_key(cfg)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .post(&cfg.base_url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .with_context(|| format!("Request failed: {} [{}]", cfg.base_url, cfg.model))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.context("Failed to read response body")?;
        bail!("API error {} [{}]: {}", status, cfg.model, text);
    }
    Ok(resp)
}

async fn parse_openai_text_response(resp: reqwest::Response, stream_output: bool) -> Result<String> {
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if stream_output && content_type.contains("text/event-stream") {
        return parse_sse_response(resp, false).await;
    }
    let text = resp.text().await.context("Failed to read response body")?;
    let val: Value = serde_json::from_str(&text).context("Invalid JSON response")?;
    extract_content(&val).context("Cannot parse response content")
}

async fn parse_openai_native_response(resp: reqwest::Response) -> Result<NativeLlmResponse> {
    let text = resp.text().await.context("Failed to read response body")?;
    let val: Value = serde_json::from_str(&text).context("Invalid JSON response")?;
    let assistant_message = val
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .cloned()
        .context("Cannot parse response message")?;
    Ok(NativeLlmResponse {
        content: extract_content_from_message(&assistant_message).unwrap_or_default(),
        tool_calls: extract_native_tool_calls(&assistant_message),
        assistant_message,
    })
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
    let content = value
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?;
    extract_content_value(content)
}

fn extract_content_from_message(message: &Value) -> Option<String> {
    extract_content_value(message.get("content")?)
}

fn extract_content_value(content: &Value) -> Option<String> {
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

fn extract_native_tool_calls(message: &Value) -> Vec<NativeFunctionCall> {
    let Some(items) = message.get("tool_calls").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (idx, tc) in items.iter().enumerate() {
        let name = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("call_{}", idx + 1));
        let arguments = tc
            .get("function")
            .and_then(|f| f.get("arguments"))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            })
            .unwrap_or_else(|| "{}".to_string());
        out.push(NativeFunctionCall {
            id,
            name,
            arguments,
        });
    }
    out
}

