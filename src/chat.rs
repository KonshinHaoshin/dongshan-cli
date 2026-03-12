use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use encoding_rs::GBK;
use serde::Serialize;
use serde_json::{Value, json};

use crate::chat_context::augment_user_input_with_workspace_context;
use crate::config::{
    AutoExecMode, Config, ModelApiProvider, ToolCallMode, active_effective_tool_mode,
    build_system_prompt, config_dir, current_prompt_text, ensure_model_catalog, save_config,
    set_active_model, set_model_tool_mode,
};
use crate::diagnostics::{
    LastDiagnostic, TurnArtifact, now_unix_ts, read_last_diagnostic, write_last_diagnostic,
    write_turn_artifact,
};
use crate::fs_tools::{
    grep_output, grep_recursive, list_files_output, list_files_recursive, read_text_file,
    try_rg_files, try_rg_grep,
};
use crate::llm::{
    ChatAttachment, ChatMessage, NativeFunctionCall, build_openai_messages, call_llm_with_history,
    call_llm_with_history_stream_tools, call_llm_with_messages_native_tools,
};
use crate::prompt_store::list_prompt_names;
use crate::skills::{
    find_skill, load_active_skill_for_session, load_skills, pick_skill_for_input,
    runtime_active_skill, save_active_skill_for_session, set_runtime_active_skill,
};
use crate::util::{
    WorkingStatus, ask, ask_or_eof, color_blue, color_cyan, color_dim, color_green, color_red,
    color_rust, color_yellow, prefix_chars, print_startup_banner, render_markdown_terminal, tagged_prompt,
    truncate_preview, truncate_with_suffix,
};
const MAX_AUTO_TOOL_STEPS: usize = 3;
const MAX_COMMANDS_PER_RESPONSE: usize = 8;
const MAX_FAILED_COMMANDS_PER_RESPONSE: usize = 2;
const MAX_INVALID_FORMAT_RETRIES: usize = 2;
const MAX_WRITE_TASK_RETRIES: usize = 2;
const MAX_WRITE_CLAIM_RETRIES: usize = 1;
const MAX_DIFF_PREVIEW_FILES: usize = 20;
const MAX_FS_PREVIEW_TEXT_BYTES: usize = 1400;
const MAX_FS_SNAPSHOT_HASH_BYTES: u64 = 1_000_000;
const STRICT_TOOL_CALL_INSTRUCTION: &str = "You must execute using strict JSON tool_calls only. Allowed format example: {\"tool_calls\":[{\"tool\":\"fs_create_file\",\"args\":{\"path\":\"analysis.md\",\"content\":\"...\"}}]}. Do not output <think>, code_execution, or markdown code fences.";
const WRITE_TASK_RETRY_MSG: &str = "The user asked you to create or modify files. Do not ask the user to save manually. You must execute tool_calls to write files in workspace, then report result. Use strict JSON tool_calls only.";
const WRITE_CLAIM_RETRY_MSG: &str = "You claimed file creation/update, but no file changes were detected. Do not claim success unless a real tool call has executed and changed files. Now execute required tool_calls to create/update the target file using strict JSON only.";

fn user_visible_write_fail() -> &'static str {
    "No real file changes were applied."
}

fn user_visible_manual_delegation() -> &'static str {
    "This reply only described manual edits; it did not modify files."
}

#[derive(Default, Clone)]
struct DiffPreviewCache {
    key: String,
    preview: String,
}

static DIFF_PREVIEW_CACHE: OnceLock<Mutex<DiffPreviewCache>> = OnceLock::new();
static FS_BASELINE_SNAPSHOT: OnceLock<Mutex<Option<BTreeMap<String, FsEntry>>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsEntry {
    is_dir: bool,
    size: u64,
    mtime_unix: i64,
    digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatExecutionMode {
    ChatOnly,
    AgentAuto,
    AgentForce,
}

impl ChatExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            ChatExecutionMode::ChatOnly => "chat",
            ChatExecutionMode::AgentAuto => "agent-auto",
            ChatExecutionMode::AgentForce => "agent-force",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "chat" | "chat-only" => Some(ChatExecutionMode::ChatOnly),
            "auto" | "agent-auto" => Some(ChatExecutionMode::AgentAuto),
            "agent" | "agent-force" => Some(ChatExecutionMode::AgentForce),
            _ => None,
        }
    }
}

pub async fn run_chat(mut cfg: Config, session: &str) -> Result<()> {
    let mut active_session = resolve_session_name(session)?;
    let mut exec_mode = ChatExecutionMode::AgentAuto;
    let mut render_markdown = true;
    let mut active_skill = load_active_skill_for_session(&active_session)?;
    print_startup_banner(&active_session, &cfg.model, exec_mode.as_str());
    let mut history = load_session_or_default(&active_session)?;
    loop {
        println!("\n{}", color_dim("────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────"));
        let Some(input) = ask_or_eof(&format!("{} ", color_rust("● you>")))? else {
            break;
        };
        if input.trim().eq_ignore_ascii_case("/exit") {
            break;
        }
        if input.trim().is_empty() {
            continue;
        }
        let changed_before = current_changed_file_set()?;

        if input.trim_start().starts_with('/') {
            handle_chat_slash_command(
                input.trim(),
                &mut cfg,
                &mut history,
                &mut active_session,
                &mut exec_mode,
                &mut render_markdown,
                &mut active_skill,
            )
            .await?;
            save_session(&active_session, &history)?;
            print_changed_files_delta(&changed_before)?;
            continue;
        }

        let turn_skill = match active_skill.as_deref() {
            Some(name) => find_skill(name)?,
            None => pick_skill_for_input(input.trim())?,
        };
        if active_skill.is_none() && let Some(skill) = &turn_skill {
            println!("assistant> Activated skill: {}", skill.manifest.name);
        }

        if handle_natural_language_tool_command(
            input.trim(),
            &mut cfg,
            &mut history,
            render_markdown,
            turn_skill.as_ref().map(|s| s.manifest.name.as_str()),
        )
        .await?
        {
            save_session(&active_session, &history)?;
            print_changed_files_delta(&changed_before)?;
            continue;
        }

        let use_agent = should_use_agent_for_turn(&cfg, &history, input.trim(), exec_mode).await;
        let use_agent = match use_agent {
            Ok(v) => v,
            Err(_) => should_use_agent_for_input(input.trim(), exec_mode),
        };

        let ctx_working = WorkingStatus::start("collecting workspace context");
        let augmented_input = augment_user_input_with_workspace_context(&input)?;
        ctx_working.finish();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: augmented_input,
            attachments: Vec::new(),
        });

        maybe_compact_history(&mut history, &cfg);
        if use_agent {
            run_agent_turn(
                &mut cfg,
                &mut history,
                "chat",
                Some(&active_session),
                render_markdown,
                turn_skill.as_ref().map(|s| s.manifest.name.as_str()),
            )
            .await?;
        } else {
            run_chat_turn(
                &mut cfg,
                &mut history,
                "chat-lite",
                render_markdown,
                turn_skill.as_ref().map(|s| s.manifest.name.as_str()),
            )
            .await?;
        }
        save_session(&active_session, &history)?;
        print_changed_files_delta(&changed_before)?;
    }

    Ok(())
}

async fn handle_natural_language_tool_command(
    input: &str,
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    render_markdown: bool,
    active_skill: Option<&str>,
) -> Result<bool> {
    let lower = input.to_lowercase();

    if is_prompt_list_request(input, &lower) {
        let mut out = String::new();
        out.push_str(&format!("Active: {}\n", cfg.active_prompt));
        for name in list_prompt_names().unwrap_or_default() {
            let preview = if name == cfg.active_prompt {
                truncate_preview(&current_prompt_text(cfg), 90)
            } else {
                "(stored)".to_string()
            };
            out.push_str(&format!("- {name}: {preview}\n"));
        }
        println!("{out}");
        push_tool_result(history, input, "prompt.list", &out);
        return Ok(true);
    }

    if let Some(name) = parse_prompt_use(input, &lower) {
        if !list_prompt_names()
            .unwrap_or_default()
            .iter()
            .any(|p| p == &name)
        {
            println!("Prompt not found: {name}");
            return Ok(true);
        }
        cfg.active_prompt = name.clone();
        save_config(cfg)?;
        let out = format!("Active prompt switched to '{name}'.");
        println!("{out}");
        push_tool_result(history, input, "prompt.use", &out);
        return Ok(true);
    }

    if is_config_show_request(input, &lower) {
        let out = toml::to_string_pretty(cfg)?;
        println!("{out}");
        push_tool_result(history, input, "config.show", &out);
        return Ok(true);
    }

    if is_model_list_request(input, &lower) {
        ensure_model_catalog(cfg);
        println!("Current model: {}", cfg.model);
        for m in &cfg.model_catalog {
            let mark = if *m == cfg.model { "*" } else { " " };
            println!("{mark} {m}");
        }
        push_tool_result(
            history,
            input,
            "model.list",
            &format!("current={}", cfg.model),
        );
        return Ok(true);
    }

    if let Some(name) = parse_model_use(input, &lower) {
        ensure_model_catalog(cfg);
        if !cfg.model_catalog.iter().any(|m| m == &name) {
            println!("Model not found in catalog: {name}");
            return Ok(true);
        }
        set_active_model(cfg, &name);
        save_config(cfg)?;
        let out = format!("Active model switched to '{name}'.");
        println!("{out}");
        push_tool_result(history, input, "model.use", &out);
        return Ok(true);
    }

    if let Some(path) = extract_existing_file_path(input)
        && should_auto_inspect_path(input, &lower)
        && !is_list_request(input, &lower)
        && !is_grep_request(input, &lower)
    {
        inspect_path_reference(
            cfg,
            history,
            input,
            &path,
            render_markdown,
            active_skill,
            has_followup_analysis_intent(input, &lower),
        )
        .await?;
        return Ok(true);
    }

    if is_read_request(input, &lower) {
        if let Some(path) = extract_path(input) {
            inspect_path_reference(
                cfg,
                history,
                input,
                &path,
                render_markdown,
                active_skill,
                has_followup_analysis_intent(input, &lower),
            )
            .await?;
            return Ok(true);
        }
    }

    if is_list_request(input, &lower) {
        let path = extract_path(input).unwrap_or_else(|| ".".to_string());
        let out = list_files_output(Path::new(&path))?;
        print!("{out}");
        push_tool_result(history, input, "fs.list", &clip_output(&out, 8000));
        return Ok(true);
    }

    if is_grep_request(input, &lower)
        && let Some(pattern) = extract_search_pattern(input)
    {
        let path = extract_path(input).unwrap_or_else(|| ".".to_string());
        let out = grep_output(Path::new(&path), &pattern)?;
        if out.trim().is_empty() {
            println!("No matches found.");
            push_tool_result(history, input, "fs.grep", "No matches found.");
        } else {
            print!("{out}");
            push_tool_result(history, input, "fs.grep", &clip_output(&out, 8000));
        }
        return Ok(true);
    }

    Ok(false)
}

async fn inspect_path_reference(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    user_input: &str,
    path: &str,
    render_markdown: bool,
    active_skill: Option<&str>,
    analyze: bool,
) -> Result<()> {
    let target = Path::new(path);
    if !target.exists() {
        println!("Path not found: {path}");
        push_tool_result(history, user_input, "fs.path", &format!("Path not found: {path}"));
        return Ok(());
    }
    if target.is_dir() {
        let out = list_files_output(target)?;
        print!("{out}");
        push_tool_result(history, user_input, "fs.list", &clip_output(&out, 8000));
        return Ok(());
    }
    if is_probably_binary_file(target) {
        let summary = describe_path_metadata(target)?;
        println!("{summary}");
        push_tool_result(history, user_input, "fs.stat", &summary);
        return Ok(());
    }
    if analyze {
        submit_file_to_model(cfg, history, user_input, path, render_markdown, active_skill).await?;
        return Ok(());
    }
    let content = read_text_file(target)?;
    push_tool_result(history, user_input, "fs.read", &clip_output(&content, 8000));
    println!(
        "Read {} (content hidden). Ask a follow-up question to analyze it.",
        path
    );
    Ok(())
}

async fn submit_file_to_model(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    user_request: &str,
    path: &str,
    render_markdown: bool,
    active_skill: Option<&str>,
) -> Result<()> {
    let content = read_text_file(Path::new(path))?;
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");
    let prompt = format!(
        "User asked to analyze this file and answer a concrete request.\n\
         Provide direct answer to user request first, then list supporting evidence from file.\n\
         Do not output shell commands unless user explicitly asks.\n\n\
         Original user request:\n{}\n\n\
         File: {}\n```{}\n{}\n```",
        user_request, path, ext, content
    );
    history.push(ChatMessage {
        role: "user".to_string(),
        content: prompt,
        attachments: Vec::new(),
    });
    maybe_compact_history(history, cfg);
    let system = build_system_prompt_with_skill(cfg, "review", active_skill)?;
    run_agent_turn_with_system(cfg, history, &system, None, render_markdown, false).await
}

fn has_followup_analysis_intent(input: &str, lower: &str) -> bool {
    lower.contains("then")
        || lower.contains("and tell")
        || lower.contains("and analyze")
        || input.contains("然后")
        || input.contains("并")
        || input.contains("后续")
        || input.contains("告诉我")
        || input.contains("分析")
        || input.contains("觉得")
}
fn push_tool_result(history: &mut Vec<ChatMessage>, user_input: &str, tool: &str, output: &str) {
    history.push(ChatMessage {
        role: "user".to_string(),
        content: user_input.to_string(),
        attachments: Vec::new(),
    });
    history.push(ChatMessage {
        role: "assistant".to_string(),
        content: format!("tool[{tool}] output:\n{output}"),
        attachments: Vec::new(),
    });
}

fn clip_output(text: &str, max_len: usize) -> String {
    truncate_with_suffix(text, max_len, "...\n[truncated]")
}

fn read_clipboard_image_attachment() -> Result<Option<ChatAttachment>> {
    #[cfg(target_os = "windows")]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
if (-not [System.Windows.Forms.Clipboard]::ContainsImage()) { exit 3 }
$img = [System.Windows.Forms.Clipboard]::GetImage()
$ms = New-Object System.IO.MemoryStream
$img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
$b64 = [Convert]::ToBase64String($ms.ToArray())
Write-Output ('data:image/png;base64,' + $b64)
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", script])
            .output()
            .context("failed to invoke powershell clipboard reader")?;
        match output.status.code() {
            Some(3) => return Ok(None),
            Some(0) => {}
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                bail!(
                    "clipboard image reader failed{}",
                    if stderr.is_empty() {
                        "".to_string()
                    } else {
                        format!(": {}", stderr)
                    }
                );
            }
        }
        let data_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if data_url.is_empty() {
            return Ok(None);
        }
        return Ok(Some(ChatAttachment {
            kind: "image".to_string(),
            media_type: "image/png".to_string(),
            data_url,
        }));
    }

    #[cfg(not(target_os = "windows"))]
    {
        bail!("clipboard image paste is currently implemented for Windows terminals only");
    }
}

fn maybe_compact_history(history: &mut Vec<ChatMessage>, cfg: &Config) {
    let max_messages = cfg.history_max_messages.max(4);
    let max_chars = cfg.history_max_chars.max(2000);
    let total_chars = history
        .iter()
        .map(|m| m.content.chars().count())
        .sum::<usize>();
    if history.len() <= max_messages && total_chars <= max_chars {
        return;
    }
    if history.len() < 8 {
        return;
    }

    let tail_keep = (max_messages / 2)
        .max(6)
        .min(history.len().saturating_sub(1));
    let split_at = history.len().saturating_sub(tail_keep);
    if split_at == 0 {
        return;
    }

    let older = &history[..split_at];
    let summary = summarize_history(older);
    let mut compacted = Vec::with_capacity(tail_keep + 1);
    compacted.push(ChatMessage {
        role: "assistant".to_string(),
        content: format!("[session-summary]\n{}", summary),
        attachments: Vec::new(),
    });
    compacted.extend_from_slice(&history[split_at..]);
    *history = compacted;
}

fn compact_native_messages(messages: &mut Vec<Value>, max_chars: usize) {
    let total: usize = messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(|s| s.len())
        .sum();
    if total <= max_chars {
        return;
    }
    const CLIP_TO: usize = 400;
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
            if let Some(content) = msg.get_mut("content") {
                if let Some(s) = content.as_str() {
                    if s.len() > CLIP_TO {
                        *content = json!(truncate_with_suffix(s, CLIP_TO, "...[truncated]"));
                    }
                }
            }
        }
    }
}

fn summarize_history(messages: &[ChatMessage]) -> String {
    let mut lines = Vec::new();
    for m in messages.iter().rev().take(20).rev() {
        let role = if m.role == "user" {
            "user"
        } else {
            "assistant"
        };
        let short = truncate_with_suffix(m.content.trim(), 220, "...");
        lines.push(format!("- {}: {}", role, short.replace('\n', " ")));
    }
    let mut out = String::new();
    out.push_str("Compressed earlier context:\n");
    out.push_str(&lines.join("\n"));
    truncate_with_suffix(&out, 4000, "...\n[summary truncated]")
}

fn is_read_request(input: &str, lower: &str) -> bool {
    lower.contains("read ")
        || lower.contains("read file")
        || lower.contains("open file")
        || lower.contains("cat ")
        || input.contains("\u{8bfb}\u{53d6}")
        || input.contains("\u{6253}\u{5f00}\u{6587}\u{4ef6}")
        || input.contains("\u{67e5}\u{770b}\u{6587}\u{4ef6}")
}

fn is_list_request(input: &str, lower: &str) -> bool {
    lower.contains("list files")
        || lower.contains("list dir")
        || lower.contains("show files")
        || lower.starts_with("ls")
        || input.contains("\u{5217}\u{51fa}\u{6587}\u{4ef6}")
        || input.contains("\u{6587}\u{4ef6}\u{5217}\u{8868}")
        || input.contains("\u{76ee}\u{5f55}\u{7ed3}\u{6784}")
        || input.contains("\u{770b}\u{770b}\u{76ee}\u{5f55}")
}

fn is_grep_request(input: &str, lower: &str) -> bool {
    lower.contains("grep ")
        || lower.contains("search ")
        || lower.contains("search for ")
        || lower.contains("find ")
        || lower.contains("find in ")
        || input.contains("\u{641c}\u{7d22}")
        || input.contains("\u{67e5}\u{627e}")
        || input.contains("\u{68c0}\u{7d22}")
}

fn is_prompt_list_request(input: &str, lower: &str) -> bool {
    lower.contains("list prompt")
        || lower.contains("show prompts")
        || lower.contains("list presets")
        || lower.contains("show preset prompts")
        || input.contains("\u{63d0}\u{793a}\u{8bcd}\u{5217}\u{8868}")
        || input.contains("\u{5217}\u{51fa}prompt")
}

fn is_config_show_request(input: &str, lower: &str) -> bool {
    lower.contains("show config")
        || lower.contains("current config")
        || input.contains("\u{67e5}\u{770b}\u{914d}\u{7f6e}")
        || input.contains("\u{5f53}\u{524d}\u{914d}\u{7f6e}")
}

fn is_model_list_request(input: &str, lower: &str) -> bool {
    lower.contains("list model")
        || lower.contains("show models")
        || input.contains("妯″瀷鍒楄〃")
        || input.contains("鍒楀嚭妯″瀷")
}

fn parse_model_use(input: &str, lower: &str) -> Option<String> {
    if let Some(idx) = lower.find("use model ") {
        let name = input[idx + "use model ".len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    if let Some(idx) = input.find("鍒囨崲妯″瀷") {
        let name = input[idx + "鍒囨崲妯″瀷".len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn parse_prompt_use(input: &str, lower: &str) -> Option<String> {
    if let Some(idx) = lower.find("use prompt ") {
        let name = input[idx + "use prompt ".len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    if let Some(idx) = lower.find("load prompt ") {
        let name = input[idx + "load prompt ".len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    if let Some(idx) = input.find("\u{5207}\u{6362}prompt") {
        let name = input[idx + "\u{5207}\u{6362}prompt".len()..].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn extract_search_pattern(input: &str) -> Option<String> {
    if let Some(q) = extract_quoted(input) {
        return Some(q);
    }
    if let Some(p) = extract_after_keyword(input, "grep ") {
        return Some(first_token(p));
    }
    if let Some(p) = extract_after_keyword(input, "search for ") {
        return Some(first_token(p));
    }
    if let Some(p) = extract_after_keyword(input, "search ") {
        return Some(first_token(p));
    }
    if let Some(p) = extract_after_keyword(input, "find in ") {
        return Some(first_token(p));
    }
    if let Some(p) = extract_after_keyword(input, "find ") {
        return Some(first_token(p));
    }
    if let Some(p) = extract_after_keyword(input, "\u{641c}\u{7d22}") {
        let p = p.trim().trim_start_matches(':').trim();
        if !p.is_empty() {
            return Some(first_token(p));
        }
    }
    None
}

fn extract_path(input: &str) -> Option<String> {
    for token in input.split_whitespace() {
        let t = token.trim_matches(|c| {
            matches!(
                c,
                '"' | '\'' | ',' | ';' | ':' | '\u{3002}' | '\u{ff0c}' | '\u{ff1a}' | '\u{ff1b}'
            )
        });
        if t.is_empty() {
            continue;
        }
        if t == "." || t == ".." || t.contains('/') || t.contains('\\') {
            return Some(t.to_string());
        }
        if t.contains('.') && !t.ends_with('.') {
            return Some(t.to_string());
        }
    }
    None
}

fn extract_existing_file_path(input: &str) -> Option<String> {
    if let Some(path) = extract_path(input)
        && Path::new(&path).exists()
    {
        return Some(path);
    }

    let mut cur = String::new();
    let mut candidates: Vec<String> = Vec::new();
    for ch in input.chars() {
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\');
        if ok {
            cur.push(ch);
        } else if !cur.is_empty() {
            candidates.push(cur.clone());
            cur.clear();
        }
    }
    if !cur.is_empty() {
        candidates.push(cur);
    }

    for c in candidates {
        if c.len() < 3 {
            continue;
        }
        if !(c.contains('.') || c.contains('/') || c.contains('\\')) {
            continue;
        }
        if Path::new(&c).exists() {
            return Some(c);
        }
    }
    None
}

fn should_auto_inspect_path(input: &str, lower: &str) -> bool {
    is_read_request(input, lower)
        || lower.contains("analyze ")
        || lower.contains("open ")
        || lower.contains("check ")
        || input.contains("分析")
        || input.contains("读取")
        || input.contains("查看")
        || input.contains("打开")
        || input.contains("检查")
        || input.contains("看看")
}

fn is_probably_binary_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "ico"
            | "pdf"
            | "zip"
            | "tar"
            | "gz"
            | "7z"
            | "dll"
            | "exe"
            | "so"
            | "bin"
    ) {
        return true;
    }
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    bytes.iter().take(512).any(|b| *b == 0)
}

fn describe_path_metadata(path: &Path) -> Result<String> {
    let meta = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata {}", path.display()))?;
    let kind = if meta.is_dir() { "directory" } else { "binary or non-text file" };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(format!(
        "{}: {}\nsize: {} bytes\nmodified_unix: {}",
        kind,
        path.display(),
        meta.len(),
        modified
    ))
}

fn extract_quoted(input: &str) -> Option<String> {
    let start = input.find('"').or_else(|| input.find('\''))?;
    let quote = input.chars().nth(start)?;
    let rest = &input[start + 1..];
    let end_rel = rest.find(quote)?;
    Some(rest[..end_rel].to_string())
}

fn extract_after_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let lower = input.to_lowercase();
    let idx = lower.find(keyword)?;
    let start = idx + keyword.len();
    Some(&input[start..])
}

fn first_token(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn build_system_prompt_with_skill(
    cfg: &Config,
    mode: &str,
    active_skill: Option<&str>,
) -> Result<String> {
    let mut prompt = build_system_prompt(cfg, mode);
    let Some(name) = active_skill.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(prompt);
    };
    if let Some(skill) = find_skill(name)? {
        prompt.push_str("\nActive skill:");
        prompt.push_str(&format!("\n- name: {}", skill.manifest.name));
        if !skill.manifest.description.trim().is_empty() {
            prompt.push_str(&format!("\n- description: {}", skill.manifest.description));
        }
        if !skill.manifest.allowed_tools.is_empty() {
            prompt.push_str(&format!(
                "\n- preferred_tools: {}",
                skill.manifest.allowed_tools.join(", ")
            ));
        }
        if !skill.manifest.trusted_commands.is_empty() {
            prompt.push_str(&format!(
                "\n- trusted_commands: {}",
                skill.manifest.trusted_commands.join(", ")
            ));
        }
        if !skill.prompt_text.trim().is_empty() {
            prompt.push_str("\nSkill instructions:\n");
            prompt.push_str(&skill.prompt_text);
        }
    } else {
        prompt.push_str(&format!(
            "\nRequested active skill '{}' was not found. Continue without it.",
            name
        ));
    }
    Ok(prompt)
}

async fn handle_chat_slash_command(
    input: &str,
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    active_session: &mut String,
    exec_mode: &mut ChatExecutionMode,
    render_markdown: &mut bool,
    active_skill: &mut Option<String>,
) -> Result<()> {
    let mut parts = input.split_whitespace();
    let Some(cmd) = parts.next() else {
        return Ok(());
    };

    match cmd {
        "/help" => {
            let c = |cmd: &str, desc: &str| {
                println!("  {}  {}", color_cyan(cmd), color_dim(desc));
            };
            println!(
                "{}",
                color_dim("─────────────────────────────────────────────")
            );
            c("/help", "show this message");
            c("/exit", "quit");
            c("/status", "show model/tool/error status");
            c("/render show|on|off", "toggle terminal markdown rendering");
            c("/new [name]", "start a new session");
            c("/clear", "clear current session history");
            c("/session list", "list saved sessions");
            c("/session use <name>", "switch session");
            c("/session rm <name>", "delete session");
            c("/mode show", "show current execution mode");
            c("/mode chat|agent-auto|agent-force", "switch execution mode");
            c("/read <file> [question]", "read a file into context");
            c("/askfile <file> <question>", "ask about a file");
            c("/list [path]", "list files");
            c("/grep <pattern> [path]", "search files");
            c("/paste [question]", "paste clipboard image into current turn");
            c("/prompt show", "show active prompt");
            c("/prompt list", "list prompts");
            c("/prompt use <name>", "switch prompt");
            c("/model list", "list available models");
            c("/model use <name>", "switch model");
            c("/skill list|show|use|clear", "manage session skill");
            println!(
                "{}",
                color_dim("─────────────────────────────────────────────")
            );
        }
        "/new" => {
            let next = parts.next();
            let new_session = if let Some(name) = next {
                resolve_session_name(name)?
            } else {
                fresh_session_name_for_workspace()?
            };
            history.clear();
            *active_session = new_session.clone();
            save_session(active_session, history)?;
            println!("Started new session: {}", new_session);
        }
        "/clear" => {
            history.clear();
            println!("Session history cleared.");
        }
        "/session" => {
            let Some(sub) = parts.next() else {
                println!("Usage: /session <list|use|rm>");
                return Ok(());
            };
            match sub {
                "list" => {
                    let sessions = list_saved_sessions()?;
                    if sessions.is_empty() {
                        println!("No saved sessions.");
                    } else {
                        println!("Saved sessions:");
                        for name in sessions {
                            if name == *active_session {
                                println!("* {name}");
                            } else {
                                println!("  {name}");
                            }
                        }
                    }
                }
                "use" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /session use <name>");
                        return Ok(());
                    };
                    let next_session = resolve_session_name(name)?;
                    let next_history = load_session_or_default(&next_session)?;
                    *history = next_history;
                    *active_session = next_session.clone();
                    *active_skill = load_active_skill_for_session(&next_session)?;
                    println!(
                        "Switched session: {} ({} messages)",
                        next_session,
                        history.len()
                    );
                }
                "rm" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /session rm <name>");
                        return Ok(());
                    };
                    let target = resolve_session_name(name)?;
                    if target == *active_session {
                        println!("Cannot remove current active session: {}", target);
                        return Ok(());
                    }
                    if remove_session_file(&target)? {
                        println!("Removed session: {}", target);
                    } else {
                        println!("Session not found: {}", target);
                    }
                }
                _ => {
                    println!("Usage: /session <list|use|rm>");
                }
            }
        }
        "/mode" => {
            let sub = parts.next().unwrap_or("show");
            if sub.eq_ignore_ascii_case("show") {
                println!("mode: {}", color_yellow(exec_mode.as_str()));
            } else if let Some(next_mode) = ChatExecutionMode::parse(sub) {
                *exec_mode = next_mode;
                println!("mode → {}", color_yellow(exec_mode.as_str()));
            } else {
                println!("Usage: /mode show|chat|agent-auto|agent-force");
            }
        }
        "/render" => {
            let sub = parts.next().unwrap_or("show");
            match sub {
                "show" => println!("render: {}", if *render_markdown { "on" } else { "off" }),
                "on" => {
                    *render_markdown = true;
                    println!("render → on");
                }
                "off" => {
                    *render_markdown = false;
                    println!("render → off");
                }
                _ => println!("Usage: /render show|on|off"),
            }
        }
        "/status" => {
            print_status(cfg)?;
            println!(
                "active_skill: {}",
                active_skill.as_deref().unwrap_or("(none)")
            );
        }
        "/paste" => {
            let attachment = match read_clipboard_image_attachment() {
                Ok(Some(v)) => v,
                Ok(None) => {
                    println!("No image found in clipboard.");
                    return Ok(());
                }
                Err(err) => {
                    println!("Failed to read clipboard image: {}", err);
                    return Ok(());
                }
            };
            let mut question = parts.collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                let inline = ask("image> ")?;
                question = inline.trim().to_string();
            }
            let prompt = if question.trim().is_empty() {
                "Please analyze this pasted image.".to_string()
            } else {
                question.trim().to_string()
            };
            let turn_skill = match active_skill.as_deref() {
                Some(name) => find_skill(name)?,
                None => pick_skill_for_input(&prompt)?,
            };
            let use_agent = should_use_agent_for_turn(cfg, history, &prompt, *exec_mode)
                .await
                .unwrap_or_else(|_| should_use_agent_for_input(&prompt, *exec_mode));
            let augmented_input = augment_user_input_with_workspace_context(&prompt)?;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: augmented_input,
                attachments: vec![attachment],
            });
            maybe_compact_history(history, cfg);
            if use_agent {
                run_agent_turn(
                    cfg,
                    history,
                    "chat",
                    Some(active_session),
                    *render_markdown,
                    turn_skill.as_ref().map(|s| s.manifest.name.as_str()),
                )
                .await?;
            } else {
                run_chat_turn(
                    cfg,
                    history,
                    "chat-lite",
                    *render_markdown,
                    turn_skill.as_ref().map(|s| s.manifest.name.as_str()),
                )
                .await?;
            }
        }
        "/read" => {
            let Some(file) = parts.next() else {
                println!("Usage: /read <file>");
                return Ok(());
            };
            let question = parts.collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                let content = read_text_file(Path::new(file))?;
                push_tool_result(history, input, "fs.read", &clip_output(&content, 8000));
                println!(
                    "Read {} (content hidden). Ask a follow-up question to analyze it.",
                    file
                );
            } else {
                submit_file_to_model(
                    cfg,
                    history,
                    &question,
                    file,
                    *render_markdown,
                    active_skill.as_deref(),
                )
                .await?;
            }
        }
        "/askfile" => {
            let Some(file) = parts.next() else {
                println!("Usage: /askfile <file> <question>");
                return Ok(());
            };
            let question = parts.collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                println!("Usage: /askfile <file> <question>");
                return Ok(());
            }
            submit_file_to_model(
                cfg,
                history,
                &question,
                file,
                *render_markdown,
                active_skill.as_deref(),
            )
            .await?;
        }
        "/list" => {
            let path = parts.next().unwrap_or(".");
            let path = Path::new(path);
            if !try_rg_files(path)? {
                list_files_recursive(path)?;
            }
        }
        "/grep" => {
            let Some(pattern) = parts.next() else {
                println!("Usage: /grep <pattern> [path]");
                return Ok(());
            };
            let path = parts.next().unwrap_or(".");
            let path = Path::new(path);
            if !try_rg_grep(path, pattern)? {
                grep_recursive(path, pattern)?;
            }
        }
        "/prompt" => {
            let Some(sub) = parts.next() else {
                println!("Usage: /prompt <show|list|use>");
                return Ok(());
            };
            match sub {
                "show" => {
                    println!("Active prompt: {}", cfg.active_prompt);
                    println!("{}", current_prompt_text(cfg));
                }
                "list" => {
                    println!("Active: {}", cfg.active_prompt);
                    for name in list_prompt_names().unwrap_or_default() {
                        if name == cfg.active_prompt {
                            println!(
                                "- {}: {}",
                                name,
                                truncate_preview(&current_prompt_text(cfg), 90)
                            );
                        } else {
                            println!("- {}: (stored)", name);
                        }
                    }
                }
                "use" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /prompt use <name>");
                        return Ok(());
                    };
                    if !list_prompt_names()
                        .unwrap_or_default()
                        .iter()
                        .any(|p| p == name)
                    {
                        println!("Prompt not found: {name}");
                        return Ok(());
                    }
                    cfg.active_prompt = name.to_string();
                    save_config(cfg)?;
                    println!("Active prompt switched to '{}'.", name);
                }
                _ => {
                    println!("Usage: /prompt <show|list|use>");
                }
            }
        }
        "/model" => {
            ensure_model_catalog(cfg);
            let Some(sub) = parts.next() else {
                println!("Usage: /model <list|use>");
                return Ok(());
            };
            match sub {
                "list" => {
                    println!("Current model: {}", cfg.model);
                    for m in &cfg.model_catalog {
                        let mark = if *m == cfg.model { "*" } else { " " };
                        println!("{mark} {m}");
                    }
                }
                "use" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /model use <name>");
                        return Ok(());
                    };
                    if !cfg.model_catalog.iter().any(|m| m == name) {
                        println!("Model not in catalog: {}", name);
                        return Ok(());
                    }
                    set_active_model(cfg, name);
                    save_config(cfg)?;
                    println!("Active model switched to '{}'.", name);
                }
                _ => println!("Usage: /model <list|use>"),
            }
        }
        "/skill" => {
            let Some(sub) = parts.next() else {
                println!("Usage: /skill <list|show|use|clear>");
                return Ok(());
            };
            match sub {
                "list" => {
                    let skills = load_skills()?;
                    if skills.is_empty() {
                        println!("No skills found.");
                    } else {
                        for skill in skills {
                            let mark = if active_skill
                                .as_deref()
                                .is_some_and(|v| v.eq_ignore_ascii_case(&skill.manifest.name))
                            {
                                "*"
                            } else {
                                " "
                            };
                            println!("{mark} {}", skill.manifest.name);
                        }
                    }
                }
                "show" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /skill show <name>");
                        return Ok(());
                    };
                    let Some(skill) = find_skill(name)? else {
                        println!("Skill not found: {}", name);
                        return Ok(());
                    };
                    println!("Skill: {}", skill.manifest.name);
                    println!("Description: {}", skill.manifest.description);
                    if !skill.prompt_text.trim().is_empty() {
                        println!("Prompt: {}", truncate_preview(&skill.prompt_text, 180));
                    }
                }
                "use" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /skill use <name>");
                        return Ok(());
                    };
                    let Some(skill) = find_skill(name)? else {
                        println!("Skill not found: {}", name);
                        return Ok(());
                    };
                    save_active_skill_for_session(active_session, Some(&skill.manifest.name))?;
                    *active_skill = Some(skill.manifest.name.clone());
                    println!("Active skill switched to '{}'.", skill.manifest.name);
                }
                "clear" => {
                    save_active_skill_for_session(active_session, None)?;
                    *active_skill = None;
                    println!("Active skill cleared.");
                }
                _ => println!("Usage: /skill <list|show|use|clear>"),
            }
        }
        _ => {
            println!("Unknown command: {}. Use /help.", cmd);
        }
    }
    Ok(())
}

struct ExecResult {
    executed_any: bool,
    had_blocks: bool,
    skipped_any: bool,
    invalid_format: bool,
    had_failures: bool,
    display_text: String,
    history_text: String,
}

#[derive(Debug, Clone)]
struct ToolCall {
    tool: String,
    command: String,
    args: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ToolResultRecord {
    tool: String,
    status: String,
    output: String,
    error: Option<String>,
    changed_files: Vec<String>,
}

#[derive(Debug, Clone)]
struct NativeToolExecution {
    call_id: String,
    output: String,
}

fn maybe_execute_assistant_commands(cfg: &mut Config, answer: &str) -> Result<ExecResult> {
    let calls = extract_tool_calls(answer);
    if calls.is_empty() {
        if contains_code_execution_hint(answer) {
            let msg = "Detected incompatible tool protocol: `code_execution`.\nThis model/gateway is not following dongshan JSON tool_calls schema, so execution is skipped.\nPlease switch to a model/provider that supports OpenAI function-calling or strict JSON tool_calls.\n";
            let records = vec![ToolResultRecord {
                tool: "tool_parser".to_string(),
                status: "error".to_string(),
                output: String::new(),
                error: Some(msg.trim().to_string()),
                changed_files: Vec::new(),
            }];
            return Ok(ExecResult {
                executed_any: false,
                had_blocks: true,
                skipped_any: true,
                invalid_format: true,
                had_failures: false,
                display_text: format!("\n{}", msg),
                history_text: format!(
                    "tool[native.exec] results:\n{}",
                    serialize_tool_results(&records)
                ),
            });
        }
        if contains_legacy_shell_block(answer) {
            let msg = "Detected legacy shell block. Skipped: use JSON tool_calls instead.\n";
            let records = vec![ToolResultRecord {
                tool: "tool_parser".to_string(),
                status: "skipped".to_string(),
                output: msg.trim().to_string(),
                error: None,
                changed_files: Vec::new(),
            }];
            return Ok(ExecResult {
                executed_any: false,
                had_blocks: true,
                skipped_any: true,
                invalid_format: true,
                had_failures: false,
                display_text: format!("\n{}", msg),
                history_text: format!(
                    "tool[native.exec] results:\n{}",
                    serialize_tool_results(&records)
                ),
            });
        }
        if contains_tool_call_hint(answer) {
            let msg = "Detected malformed or incomplete tool_calls JSON. Skipped; ask model to retry with valid JSON tool_calls.\n";
            let records = vec![ToolResultRecord {
                tool: "tool_parser".to_string(),
                status: "error".to_string(),
                output: String::new(),
                error: Some(msg.trim().to_string()),
                changed_files: Vec::new(),
            }];
            return Ok(ExecResult {
                executed_any: false,
                had_blocks: true,
                skipped_any: true,
                invalid_format: true,
                had_failures: false,
                display_text: format!("\n{}", msg),
                history_text: format!(
                    "tool[native.exec] results:\n{}",
                    serialize_tool_results(&records)
                ),
            });
        }
        return Ok(ExecResult {
            executed_any: false,
            had_blocks: false,
            skipped_any: false,
            invalid_format: false,
            had_failures: false,
            display_text: String::new(),
            history_text: String::new(),
        });
    }

    let mut display = String::new();
    let mut records: Vec<ToolResultRecord> = Vec::new();
    let mut executed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut seen_calls = 0usize;
    let mut failed_calls = 0usize;

    for call in calls {
        if seen_calls >= MAX_COMMANDS_PER_RESPONSE {
            let line = format!(
                "Stopped execution after {} tool calls to avoid noisy output.\n",
                MAX_COMMANDS_PER_RESPONSE
            );
            display.push_str(&line);
            records.push(ToolResultRecord {
                tool: "native.exec".to_string(),
                status: "skipped".to_string(),
                output: line.trim().to_string(),
                error: None,
                changed_files: Vec::new(),
            });
            break;
        }
        seen_calls += 1;

        let tool = call.tool.trim().to_ascii_lowercase();
        if tool.is_empty() {
            skipped_count += 1;
            continue;
        }

        let exec = execute_tool_call_with_progress(cfg, &call);
        let before_set = current_changed_file_set().unwrap_or_default();

        match exec {
            Ok(out) => {
                let status = if is_skipped_tool_output(&out) {
                    "skipped"
                } else {
                    "ok"
                };
                if status == "ok" {
                    executed_count += 1;
                } else {
                    skipped_count += 1;
                }
                if status == "ok" && looks_like_command_failure(&out) {
                    failed_calls += 1;
                }
                let after_set = current_changed_file_set().unwrap_or_default();
                let mut changed_files = changed_files_delta(&before_set, &after_set);
                if changed_files.is_empty() && status == "ok" {
                    changed_files = guessed_changed_files_for_call(&call);
                }
                display.push_str(&format!("tool[{}][{}]\n{}\n", call.tool, status, out));
                records.push(ToolResultRecord {
                    tool: call.tool.clone(),
                    status: status.to_string(),
                    output: out,
                    error: None,
                    changed_files,
                });
            }
            Err(err) => {
                let line = format!("Tool failed [{}]: {}\n", call.tool, err);
                display.push_str(&line);
                let after_set = current_changed_file_set().unwrap_or_default();
                let changed_files = changed_files_delta(&before_set, &after_set);
                records.push(ToolResultRecord {
                    tool: call.tool.clone(),
                    status: "error".to_string(),
                    output: String::new(),
                    error: Some(err.to_string()),
                    changed_files,
                });
                failed_calls += 1;
            }
        }

        if failed_calls >= MAX_FAILED_COMMANDS_PER_RESPONSE {
            let line = format!(
                "Stopped execution after {} failed tool calls.\n",
                MAX_FAILED_COMMANDS_PER_RESPONSE
            );
            display.push_str(&line);
            records.push(ToolResultRecord {
                tool: "native.exec".to_string(),
                status: "error".to_string(),
                output: String::new(),
                error: Some(line.trim().to_string()),
                changed_files: Vec::new(),
            });
            break;
        }
    }

    Ok(ExecResult {
        executed_any: executed_count > 0,
        had_blocks: true,
        skipped_any: skipped_count > 0,
        invalid_format: false,
        had_failures: failed_calls > 0,
        display_text: format!("\n{}", display),
        history_text: format!(
            "tool[native.exec] results:\n{}",
            serialize_tool_results(&records)
        ),
    })
}

fn native_tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "type":"function",
            "function":{
                "name":"fs_read_file",
                "description":"Read a UTF-8 text file from workspace",
                "parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_create_file",
                "description":"Create or overwrite a file with content",
                "parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"},"overwrite":{"type":"boolean"}},"required":["path","content"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_edit_file",
                "description":"Edit one file by replacing old_str with new_str",
                "parameters":{"type":"object","properties":{"path":{"type":"string"},"old_str":{"type":"string"},"new_str":{"type":"string"},"replace_all":{"type":"boolean"}},"required":["path","new_str"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_apply_patch",
                "description":"Apply one or more old/new text edits to a file",
                "parameters":{"type":"object","properties":{"path":{"type":"string"},"edits":{"type":"array","items":{"type":"object"}},"strict":{"type":"boolean"}},"required":["path","edits"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_list_files",
                "description":"List files under a path",
                "parameters":{"type":"object","properties":{"path":{"type":"string"}}}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_grep",
                "description":"Search pattern in files under path",
                "parameters":{"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_move",
                "description":"Move or rename a file",
                "parameters":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"fs_delete",
                "description":"Delete file or directory",
                "parameters":{"type":"object","properties":{"path":{"type":"string"},"recursive":{"type":"boolean"}},"required":["path"]}
            }
        }),
        json!({
            "type":"function",
            "function":{
                "name":"run_command",
                "description":"Run a shell command; use only when fs tools are insufficient",
                "parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}
            }
        }),
    ]
}

fn execute_native_function_calls(
    cfg: &mut Config,
    calls: &[NativeFunctionCall],
) -> Result<(ExecResult, Vec<NativeToolExecution>)> {
    if calls.is_empty() {
        return Ok((
            ExecResult {
                executed_any: false,
                had_blocks: false,
                skipped_any: false,
                invalid_format: false,
                had_failures: false,
                display_text: String::new(),
                history_text: String::new(),
            },
            Vec::new(),
        ));
    }

    let mut parsed: Vec<(String, ToolCall)> = Vec::new();
    for call in calls {
        let args = if call.arguments.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<Value>(&call.arguments)
                .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
        };
        parsed.push((
            call.id.clone(),
            ToolCall {
                tool: call.name.clone(),
                command: String::new(),
                args,
            },
        ));
    }

    let mut display = String::new();
    let mut tool_msgs: Vec<NativeToolExecution> = Vec::new();
    let mut executed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_calls = 0usize;

    for (call_id, call) in parsed {
        let before_set = current_changed_file_set().unwrap_or_default();
        let exec = execute_tool_call_with_progress(cfg, &call);

        match exec {
            Ok(out) => {
                let status = if is_skipped_tool_output(&out) {
                    "skipped"
                } else {
                    "ok"
                };
                if status == "ok" {
                    executed_count += 1;
                } else {
                    skipped_count += 1;
                }
                if status == "ok" && looks_like_command_failure(&out) {
                    failed_calls += 1;
                }
                let after_set = current_changed_file_set().unwrap_or_default();
                let mut changed_files = changed_files_delta(&before_set, &after_set);
                if changed_files.is_empty() && status == "ok" {
                    changed_files = guessed_changed_files_for_call(&call);
                }
                display.push_str(&format!("tool[{}][{}]\n{}\n", call.tool, status, out));
                if !changed_files.is_empty() {
                    display.push_str(&format!("changed_files: {}\n", changed_files.join(", ")));
                }
                tool_msgs.push(NativeToolExecution {
                    call_id,
                    output: out,
                });
            }
            Err(err) => {
                let line = format!("Tool failed [{}]: {}", call.tool, err);
                display.push_str(&format!("{line}\n"));
                tool_msgs.push(NativeToolExecution {
                    call_id,
                    output: line,
                });
                failed_calls += 1;
            }
        }
    }

    Ok((
        ExecResult {
            executed_any: executed_count > 0,
            had_blocks: true,
            skipped_any: skipped_count > 0,
            invalid_format: false,
            had_failures: failed_calls > 0,
            display_text: format!("\n{}", display),
            history_text: String::new(),
        },
        tool_msgs,
    ))
}

fn serialize_tool_results(results: &[ToolResultRecord]) -> String {
    serde_json::to_string_pretty(results).unwrap_or_else(|_| "[]".to_string())
}

fn is_skipped_tool_output(output: &str) -> bool {
    let lower = output.trim().to_ascii_lowercase();
    lower.starts_with("skipped") || lower.starts_with("skip ")
}

fn execute_tool_call_by_name(cfg: &mut Config, call: &ToolCall) -> Result<String> {
    let tool = call.tool.trim().to_ascii_lowercase();
    if let Some(skill) = runtime_active_skill()
        && !skill.manifest.allowed_tools.is_empty()
        && !skill
            .manifest
            .allowed_tools
            .iter()
            .any(|t| t.trim().eq_ignore_ascii_case(&tool))
    {
        return Ok(format!(
            "Skipped unsupported tool by active skill '{}': {}",
            skill.manifest.name, call.tool
        ));
    }
    match tool.as_str() {
        "shell" => execute_shell_tool_call(cfg, call),
        "fs.read_file" | "fs_read_file" => execute_native_fs_read(call),
        "fs.create_file" | "fs_create_file" => execute_native_fs_create(call),
        "fs.edit_file" | "fs_edit_file" => execute_native_fs_edit(call),
        "fs.list_files" | "fs_list_files" => execute_native_fs_list(call),
        "fs.grep" | "fs_grep" => execute_native_fs_grep(call),
        "fs.apply_patch" | "fs_apply_patch" => execute_native_fs_apply_patch(call),
        "fs.move" | "fs_move" => execute_native_fs_move(call),
        "fs.delete" | "fs_delete" => execute_native_fs_delete(call),
        "run_command" => execute_structured_run_command(cfg, call),
        _ => Ok(format!("Skipped unsupported tool: {}", call.tool)),
    }
}

fn tool_progress_label(call: &ToolCall) -> Option<String> {
    let tool = call.tool.trim().to_ascii_lowercase();
    match tool.as_str() {
        "fs.read_file" | "fs_read_file" => {
            let p = tool_arg_string(call, &["path", "file"])?;
            Some(format!("reading {}", p))
        }
        "fs.list_files" | "fs_list_files" => {
            let p = tool_arg_string(call, &["path"]).unwrap_or_else(|| ".".to_string());
            Some(format!("listing {}", p))
        }
        "fs.grep" | "fs_grep" => {
            let pattern = tool_arg_string(call, &["pattern", "query"])?;
            let p = tool_arg_string(call, &["path"]).unwrap_or_else(|| ".".to_string());
            Some(format!("searching '{}' in {}", pattern, p))
        }
        _ => None,
    }
}

fn execute_tool_call_with_progress(cfg: &mut Config, call: &ToolCall) -> Result<String> {
    let progress = tool_progress_label(call);
    let mut clear_width = 0usize;
    if let Some(label) = &progress {
        let line = format!("{} {}", color_dim("tool>"), label);
        clear_width = line.chars().count();
        print!("\r{}", line);
        let _ = io::stdout().flush();
    }
    let res = execute_tool_call_by_name(cfg, call);
    if clear_width > 0 {
        let width = clear_width.min(200);
        print!("\r{}\r", " ".repeat(width));
        let _ = io::stdout().flush();
    }
    res
}

fn execute_shell_tool_call(cfg: &mut Config, call: &ToolCall) -> Result<String> {
    let cmd_owned = if !call.command.trim().is_empty() {
        call.command.trim().to_string()
    } else {
        tool_arg_string(call, &["command", "cmd"]).unwrap_or_default()
    };
    let cmd = cmd_owned.trim();
    if cmd.is_empty() {
        bail!("shell tool missing command");
    }
    if let Some(reason) = precheck_command(cmd) {
        return Ok(format!("Skipped command: {} ({})", cmd, reason));
    }
    if !is_command_allowed(cfg, cmd) {
        return Ok(format!("Skipped unsafe command: {}", cmd));
    }
    if cfg.auto_confirm_exec && !is_trusted_command(cfg, cmd) {
        let prefix = command_prefix(cmd);
        let input = ask(&tagged_prompt(
            "exec-confirm",
            &format!(
                "Run command `{}` ? [y=yes]/[n=no]/[a=always `{}`]/[q=stop]: ",
                cmd, prefix
            ),
        ))?;
        let choice = input.trim().to_ascii_lowercase();
        if choice == "q" {
            return Ok("User stopped command execution.".to_string());
        }
        if choice == "a" {
            if !cfg
                .auto_exec_trusted
                .iter()
                .any(|x| x.eq_ignore_ascii_case(&prefix))
            {
                cfg.auto_exec_trusted.push(prefix.clone());
                let _ = save_config(cfg);
            }
        } else if choice != "y" {
            return Ok(format!("Skipped by user: {}", cmd));
        }
    }
    let out = run_shell_command(cmd)?;
    Ok(format!("$ {}\n{}", cmd, out))
}

fn execute_native_fs_read(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"])
        .ok_or_else(|| anyhow::anyhow!("fs.read_file requires args.path"))?;
    let path = resolve_native_path(&raw)?;
    let text = read_text_file(&path)?;
    Ok(format!(
        "Read: {}\n{}",
        path.display(),
        clip_output(&text, 12000)
    ))
}

fn execute_native_fs_create(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"])
        .ok_or_else(|| anyhow::anyhow!("fs.create_file requires args.path"))?;
    let content = tool_arg_string(call, &["content"])
        .ok_or_else(|| anyhow::anyhow!("fs.create_file requires args.content"))?;
    let overwrite = tool_arg_bool(call, &["overwrite"]).unwrap_or(true);
    let path = resolve_native_path(&raw)?;
    if path.exists() && !overwrite {
        bail!(
            "target already exists and overwrite=false: {}",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent dir {}", parent.display()))?;
    }
    fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("Created file: {}", path.display()))
}

fn execute_native_fs_edit(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"])
        .ok_or_else(|| anyhow::anyhow!("fs.edit_file requires args.path"))?;
    let old_str = tool_arg_string(call, &["old_str", "old"]).unwrap_or_default();
    let new_str = tool_arg_string(call, &["new_str", "new"])
        .ok_or_else(|| anyhow::anyhow!("fs.edit_file requires args.new_str"))?;
    let replace_all = tool_arg_bool(call, &["replace_all"]).unwrap_or(false);
    let path = resolve_native_path(&raw)?;
    let mut text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;

    if old_str.is_empty() {
        text = new_str;
    } else if replace_all {
        if !text.contains(&old_str) {
            bail!("old_str not found in {}", path.display());
        }
        text = text.replace(&old_str, &new_str);
    } else if let Some(idx) = text.find(&old_str) {
        text.replace_range(idx..idx + old_str.len(), &new_str);
    } else {
        bail!("old_str not found in {}", path.display());
    }

    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("Edited file: {}", path.display()))
}

fn execute_native_fs_list(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path"]).unwrap_or_else(|| ".".to_string());
    let path = resolve_native_path(&raw)?;
    let out = list_files_output(&path)?;
    Ok(format!(
        "List: {}\n{}",
        path.display(),
        clip_output(&out, 12000)
    ))
}

fn execute_native_fs_grep(call: &ToolCall) -> Result<String> {
    let pattern = tool_arg_string(call, &["pattern", "query"])
        .ok_or_else(|| anyhow::anyhow!("fs.grep requires args.pattern"))?;
    let raw = tool_arg_string(call, &["path"]).unwrap_or_else(|| ".".to_string());
    let path = resolve_native_path(&raw)?;
    let out = grep_output(&path, &pattern)?;
    if out.trim().is_empty() {
        return Ok(format!(
            "Grep: {} in {}\nNo matches found.",
            pattern,
            path.display()
        ));
    }
    Ok(format!(
        "Grep: {} in {}\n{}",
        pattern,
        path.display(),
        clip_output(&out, 12000)
    ))
}

fn execute_native_fs_apply_patch(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"])
        .ok_or_else(|| anyhow::anyhow!("fs.apply_patch requires args.path"))?;
    let path = resolve_native_path(&raw)?;
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;

    let edits = tool_arg_array(call, &["edits", "patches"])
        .ok_or_else(|| anyhow::anyhow!("fs.apply_patch requires args.edits[]"))?;
    if edits.is_empty() {
        bail!("fs.apply_patch requires at least one edit");
    }
    let strict = tool_arg_bool(call, &["strict"]).unwrap_or(true);
    let mut working = text;
    let mut hits: Vec<String> = Vec::new();
    let mut misses: Vec<String> = Vec::new();

    for (idx, e) in edits.iter().enumerate() {
        let Value::Object(obj) = e else {
            misses.push(format!("#{} invalid edit object", idx + 1));
            continue;
        };
        let old_s = obj
            .get("old")
            .or_else(|| obj.get("old_str"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_s = obj
            .get("new")
            .or_else(|| obj.get("new_str"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let replace_all = obj
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if old_s.is_empty() {
            misses.push(format!("#{} old/old_str is empty", idx + 1));
            continue;
        }
        if replace_all {
            let count = working.matches(old_s).count();
            if count == 0 {
                misses.push(format!("#{} old_str not found (replace_all=true)", idx + 1));
            } else {
                working = working.replace(old_s, new_s);
                hits.push(format!("#{} replace_all x{}", idx + 1, count));
            }
        } else if let Some(pos) = working.find(old_s) {
            working.replace_range(pos..pos + old_s.len(), new_s);
            hits.push(format!("#{} replaced once", idx + 1));
        } else {
            misses.push(format!("#{} old_str not found", idx + 1));
        }
    }

    if hits.is_empty() {
        let mut detail = format!("No edits applied to {}", path.display());
        if !misses.is_empty() {
            detail.push_str("\nmisses:");
            for m in misses.iter().take(20) {
                detail.push_str(&format!("\n- {}", m));
            }
        }
        bail!("{}", detail);
    }
    if strict && !misses.is_empty() {
        let mut detail = format!(
            "Patch aborted for {}: {} hit(s), {} miss(es), strict=true",
            path.display(),
            hits.len(),
            misses.len()
        );
        detail.push_str("\nhits:");
        for h in hits.iter().take(20) {
            detail.push_str(&format!("\n- {}", h));
        }
        detail.push_str("\nmisses:");
        for m in misses.iter().take(20) {
            detail.push_str(&format!("\n- {}", m));
        }
        bail!("{}", detail);
    }

    fs::write(&path, working).with_context(|| format!("Failed to write {}", path.display()))?;
    let mut report = format!(
        "Applied {}/{} edit(s) to {} (strict={})",
        hits.len(),
        edits.len(),
        path.display(),
        strict
    );
    report.push_str("\nhits:");
    for h in hits.iter().take(20) {
        report.push_str(&format!("\n- {}", h));
    }
    if !misses.is_empty() {
        report.push_str("\nmisses:");
        for m in misses.iter().take(20) {
            report.push_str(&format!("\n- {}", m));
        }
    }
    Ok(report)
}

fn execute_native_fs_move(call: &ToolCall) -> Result<String> {
    let from_raw = tool_arg_string(call, &["from", "src", "source"])
        .ok_or_else(|| anyhow::anyhow!("fs.move requires args.from"))?;
    let to_raw = tool_arg_string(call, &["to", "dst", "target"])
        .ok_or_else(|| anyhow::anyhow!("fs.move requires args.to"))?;
    let from = resolve_native_path(&from_raw)?;
    let to = resolve_native_path(&to_raw)?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent dir {}", parent.display()))?;
    }
    fs::rename(&from, &to)
        .with_context(|| format!("Failed to move {} -> {}", from.display(), to.display()))?;
    Ok(format!("Moved: {} -> {}", from.display(), to.display()))
}

fn execute_native_fs_delete(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file", "target"])
        .ok_or_else(|| anyhow::anyhow!("fs.delete requires args.path"))?;
    let recursive = tool_arg_bool(call, &["recursive", "r"]).unwrap_or(false);
    let p = resolve_native_path(&raw)?;
    if !p.exists() {
        return Ok(format!("Skip delete; not found: {}", p.display()));
    }
    if p.is_dir() {
        if recursive {
            fs::remove_dir_all(&p)
                .with_context(|| format!("Failed to remove dir {}", p.display()))?;
        } else {
            fs::remove_dir(&p).with_context(|| {
                format!("Failed to remove dir {} (set recursive=true)", p.display())
            })?;
        }
    } else {
        fs::remove_file(&p).with_context(|| format!("Failed to remove file {}", p.display()))?;
    }
    Ok(format!("Deleted: {}", p.display()))
}

fn execute_structured_run_command(cfg: &mut Config, call: &ToolCall) -> Result<String> {
    let cmd = tool_arg_string(call, &["command", "cmd"])
        .or_else(|| {
            if call.command.trim().is_empty() {
                None
            } else {
                Some(call.command.clone())
            }
        })
        .ok_or_else(|| anyhow::anyhow!("run_command requires args.command"))?;
    let shell_call = ToolCall {
        tool: "shell".to_string(),
        command: cmd,
        args: call.args.clone(),
    };
    execute_shell_tool_call(cfg, &shell_call)
}
fn resolve_native_path(raw: &str) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Failed to get current dir")?;
    let base = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        cwd.join(raw)
    };

    let normalized = if base.exists() {
        base.canonicalize()
            .with_context(|| format!("Failed to resolve path {}", base.display()))?
    } else if let Some(parent) = base.parent() {
        let p = parent
            .canonicalize()
            .with_context(|| format!("Failed to resolve parent path {}", parent.display()))?;
        if let Some(name) = base.file_name() {
            p.join(name)
        } else {
            p
        }
    } else {
        base.clone()
    };

    let cwd_norm = cwd
        .canonicalize()
        .with_context(|| format!("Failed to resolve cwd {}", cwd.display()))?;
    if !normalized.starts_with(&cwd_norm) {
        bail!("path outside workspace is not allowed: {}", raw);
    }
    Ok(normalized)
}

fn tool_arg_string(call: &ToolCall, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = &call.args else {
        return None;
    };
    for key in keys {
        if let Some(v) = map.get(*key)
            && let Some(s) = v.as_str()
        {
            return Some(s.to_string());
        }
    }
    None
}

fn tool_arg_bool(call: &ToolCall, keys: &[&str]) -> Option<bool> {
    let Value::Object(map) = &call.args else {
        return None;
    };
    for key in keys {
        if let Some(v) = map.get(*key)
            && let Some(b) = v.as_bool()
        {
            return Some(b);
        }
    }
    None
}

fn tool_arg_array(call: &ToolCall, keys: &[&str]) -> Option<Vec<Value>> {
    let Value::Object(map) = &call.args else {
        return None;
    };
    for key in keys {
        if let Some(v) = map.get(*key)
            && let Some(arr) = v.as_array()
        {
            return Some(arr.clone());
        }
    }
    None
}
fn precheck_command(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return Some("empty command".to_string());
    }
    let first = tokens[0].to_ascii_lowercase();
    let lower = cmd.to_ascii_lowercase();

    if (lower.contains("base64")
        || lower.contains("frombase64string")
        || lower.contains("[convert]::frombase64string"))
        && cmd.len() > 700
    {
        return Some("base64 payload too long; use small script file workflow instead".to_string());
    }

    if (first == "python" || first == "python3") && lower.contains(" -c ") {
        if cmd.contains('\n') || cmd.len() > 360 {
            return Some("python -c is too long/multiline; write .py file then run it".to_string());
        }
    }

    if first == "python" || first == "python3" {
        if tokens.len() >= 2 {
            let script = tokens[1].trim_matches('"').trim_matches('\'');
            if script.ends_with(".py") && !Path::new(script).exists() {
                return Some(format!("script not found: {}", script));
            }
        }
    }

    if first == "pip" && cmd.contains("-r") {
        for idx in 0..tokens.len() {
            if tokens[idx] == "-r" && idx + 1 < tokens.len() {
                let req = tokens[idx + 1].trim_matches('"').trim_matches('\'');
                if !Path::new(req).exists() {
                    return Some(format!("requirements file not found: {}", req));
                }
            }
        }
    }

    None
}
fn looks_like_command_failure(output: &str) -> bool {
    let s = output.to_ascii_lowercase();
    s.contains("commandnotfoundexception")
        || s.contains("can't open file")
        || s.contains("no such file")
        || s.contains("module not found")
        || s.contains("traceback")
        || s.contains("is not recognized")
}

async fn run_agent_turn(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    mode: &str,
    session: Option<&str>,
    render_markdown: bool,
    active_skill: Option<&str>,
) -> Result<()> {
    let runtime_skill = active_skill.and_then(|name| find_skill(name).ok().flatten());
    set_runtime_active_skill(runtime_skill)?;
    let system = build_system_prompt_with_skill(cfg, mode, active_skill)?;
    let result =
        run_agent_turn_with_system(cfg, history, &system, session, render_markdown, true).await;
    let _ = set_runtime_active_skill(None);
    result
}

async fn run_agent_turn_with_system(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
    session: Option<&str>,
    render_markdown: bool,
    allow_executor_fallback: bool,
) -> Result<()> {
    match active_effective_tool_mode(cfg) {
        ToolCallMode::Json => {
            run_agent_turn_with_system_legacy(
                cfg,
                history,
                system,
                session,
                render_markdown,
                allow_executor_fallback,
            )
            .await
        }
        _ => {
            match run_agent_turn_with_system_native(
                cfg,
                history,
                system,
                session,
                render_markdown,
                allow_executor_fallback,
            )
            .await
            {
                Ok(()) => Ok(()),
                Err(err) => {
                    cache_active_model_tool_mode(cfg, ToolCallMode::Json);
                    record_diagnostic(cfg, "native-request", &err.to_string(), session);
                    println!(
                        "\nassistant> Native function-calling unavailable, fallback to JSON tool_calls parser: {}\n",
                        truncate_with_suffix(&err.to_string(), 220, " ...")
                    );
                    run_agent_turn_with_system_legacy(
                        cfg,
                        history,
                        system,
                        session,
                        render_markdown,
                        allow_executor_fallback,
                    )
                    .await
                }
            }
        }
    }
}

async fn run_agent_turn_with_system_native(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
    session: Option<&str>,
    render_markdown: bool,
    allow_executor_fallback: bool,
) -> Result<()> {
    let tools = native_tool_schemas();
    let mut messages = build_openai_messages(system, history);
    let changed_baseline = current_changed_file_set().unwrap_or_default();
    let mut steps = 0usize;
    let mut unsafe_retries = 0usize;
    let mut invalid_format_retries = 0usize;
    let mut write_claim_retries = 0usize;
    let mut write_task_retries = 0usize;
    loop {
        compact_native_messages(&mut messages, cfg.history_max_chars.max(2000));
        println!(
            "{}",
            color_dim(&format!("(phase: reasoning step {})", steps + 1))
        );
        print!(
            "{}",
            color_rust(&format!(
                "● assistant[{}]({})> ",
                cfg.active_prompt, cfg.model
            ))
        );
        let resp = call_llm_with_messages_native_tools(cfg, &messages, &tools).await?;
        let answer = resp.content.trim().to_string();
        if !answer.is_empty() {
            println!("{}", render_markdown_terminal(&answer, render_markdown));
        }
        println!("\n");
        messages.push(resp.assistant_message);

        if resp.tool_calls.is_empty() {
            let exec_result = maybe_execute_assistant_commands(cfg, &answer)?;
            record_step_artifact_from_native(
                session,
                cfg,
                steps + 1,
                "reasoning",
                "chat",
                &messages,
                &answer,
                &[],
                Some(&exec_result),
                &changed_baseline,
            );
            if !exec_result.had_blocks {
                let changed_now = current_changed_file_set().unwrap_or_default();
                let changed_delta = changed_files_delta(&changed_baseline, &changed_now);
                let last_user = last_user_text_from_native_messages(&messages).unwrap_or_default();
                if let Some(msg) = evaluate_write_guard(
                    last_user,
                    &answer,
                    &changed_delta,
                    &mut write_task_retries,
                    &mut write_claim_retries,
                ) {
                    messages.push(json!({
                        "role":"user",
                        "content": format!("{msg} {}", STRICT_TOOL_CALL_INSTRUCTION)
                    }));
                    continue;
                }
                if changed_delta.is_empty() && looks_like_write_request(last_user) {
                    let err = "Write task failed: model did not produce executable tool_calls and no file changes were detected.";
                    if try_executor_model_fallback(
                        cfg,
                        history,
                        system,
                        session,
                        render_markdown,
                        err,
                        allow_executor_fallback,
                    )
                    .await?
                    {
                        return Ok(());
                    }
                    record_diagnostic(cfg, "write-hard-fail", err, session);
                    println!("assistant> {}", user_visible_write_fail());
                    return Ok(());
                }
                if looks_like_manual_action_answer(&answer)
                    && (looks_like_write_request(last_user) || looks_like_patch_request(last_user))
                {
                    let err = "Model attempted to delegate manual editing. This task requires executable tool_calls and real diffs.";
                    if try_executor_model_fallback(
                        cfg,
                        history,
                        system,
                        session,
                        render_markdown,
                        err,
                        allow_executor_fallback,
                    )
                    .await?
                    {
                        return Ok(());
                    }
                    record_diagnostic(cfg, "manual-delegation-blocked", err, session);
                    println!("assistant> {}", user_visible_manual_delegation());
                    return Ok(());
                }
                history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: answer,
                    attachments: Vec::new(),
                });
                return Ok(());
            }

            cache_active_model_tool_mode(cfg, ToolCallMode::Json);
            if exec_result.executed_any {
                let (verification, recovery_hint) = print_execution_and_verification(&exec_result)?;
                messages.push(json!({
                    "role":"user",
                    "content": format!(
                        "{}\n{}{}\nContinue based on tool outputs above. If more execution is needed, emit JSON tool_calls. If complete, give final answer directly with short summary, changed files, and verification result.",
                        exec_result.history_text,
                        verification,
                        recovery_hint
                    )
                }));
                steps += 1;
                if steps >= MAX_AUTO_TOOL_STEPS {
                    println!(
                        "assistant> Reached auto tool step limit ({}). Continue by describing next action.",
                        MAX_AUTO_TOOL_STEPS
                    );
                    return Ok(());
                }
                continue;
            }

            if exec_result.invalid_format && invalid_format_retries < MAX_INVALID_FORMAT_RETRIES {
                invalid_format_retries += 1;
                messages.push(json!({
                    "role":"user",
                    "content": format!("Your last response had invalid tool_calls format. {}",
                        STRICT_TOOL_CALL_INSTRUCTION)
                }));
                continue;
            }

            if exec_result.skipped_any && unsafe_retries < 1 {
                unsafe_retries += 1;
                messages.push(json!({
                    "role":"user",
                    "content": format!("Your last response used unsupported execution format or unsafe commands. {}",
                        STRICT_TOOL_CALL_INSTRUCTION)
                }));
                continue;
            }

            println!("assistant> Detected tool calls, but all were skipped or unsafe.\n");
            record_diagnostic(
                cfg,
                "tool-protocol",
                "tool calls detected but all were skipped or unsafe",
                session,
            );
            return Ok(());
        }
        let (exec_result, tool_msgs) = execute_native_function_calls(cfg, &resp.tool_calls)?;
        let native_calls = native_calls_to_values(&resp.tool_calls, cfg, session);
        record_step_artifact_from_native(
            session,
            cfg,
            steps + 1,
            "tool-exec",
            "chat",
            &messages,
            &answer,
            &native_calls,
            Some(&exec_result),
            &changed_baseline,
        );
        for m in tool_msgs {
            messages.push(json!({
                "role":"tool",
                "tool_call_id": m.call_id,
                "content": m.output
            }));
        }

        if exec_result.executed_any {
            let (verification, recovery_hint) = print_execution_and_verification(&exec_result)?;
            messages.push(json!({
                "role":"user",
                "content": format!(
                    "{}{}\nContinue based on tool outputs above. If more execution is needed, call functions directly. If complete, give final answer directly with short summary, changed files, and verification result.",
                    verification,
                    recovery_hint
                )
            }));
            steps += 1;
            if steps >= MAX_AUTO_TOOL_STEPS {
                println!(
                    "assistant> Reached auto tool step limit ({}). Continue by describing next action.",
                    MAX_AUTO_TOOL_STEPS
                );
                return Ok(());
            }
            continue;
        }

        if exec_result.skipped_any && unsafe_retries < 1 {
            unsafe_retries += 1;
            messages.push(json!({
                "role":"user",
                "content": format!("Your last tool calls were unsupported or unsafe. {}",
                    STRICT_TOOL_CALL_INSTRUCTION)
            }));
            continue;
        }

        println!("assistant> Detected tool calls, but all were skipped or unsafe.\n");
        record_diagnostic(
            cfg,
            "tool-protocol",
            "native tool calls detected but all were skipped or unsafe",
            session,
        );
        return Ok(());
    }
}

fn cache_active_model_tool_mode(cfg: &mut Config, mode: ToolCallMode) {
    let model = cfg.model.clone();
    let current = cfg.model_profiles.get(&model).map(|p| p.tool_mode);
    if current == Some(mode) {
        return;
    }
    set_model_tool_mode(cfg, &model, mode);
    let _ = save_config(cfg);
}

async fn run_agent_turn_with_system_legacy(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
    session: Option<&str>,
    render_markdown: bool,
    allow_executor_fallback: bool,
) -> Result<()> {
    let changed_baseline = current_changed_file_set().unwrap_or_default();
    let mut steps = 0usize;
    let mut unsafe_retries = 0usize;
    let mut invalid_format_retries = 0usize;
    let mut write_claim_retries = 0usize;
    let mut write_task_retries = 0usize;
    loop {
        maybe_compact_history(history, cfg);
        println!(
            "{}",
            color_dim(&format!("(phase: reasoning step {})", steps + 1))
        );
        print!(
            "{}",
            color_rust(&format!(
                "● assistant[{}]({})> ",
                cfg.active_prompt, cfg.model
            ))
        );
        let answer = match call_llm_with_history_stream_tools(cfg, system, history, &native_tool_schemas()).await {
            Ok(v) => v,
            Err(err) => {
                record_diagnostic(cfg, "legacy-request", &err.to_string(), session);
                println!("\n");
                println!(
                    "assistant> Request interrupted: {}",
                    truncate_with_suffix(&err.to_string(), 220, " ...")
                );
                println!("assistant> You can continue chatting and send the next message.\n");
                return Ok(());
            }
        };
        println!("\n");
        if !answer.trim().is_empty() {
            println!("{}", render_markdown_terminal(&answer, render_markdown));
            println!("\n");
        }
        let exec_result = maybe_execute_assistant_commands(cfg, &answer)?;
        let last_user = history
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let parsed_calls = tool_calls_from_text(&answer);
        record_step_artifact(
            session,
            cfg,
            steps + 1,
            "reasoning",
            "chat",
            &last_user,
            &answer,
            &parsed_calls,
            Some(&exec_result),
            &changed_baseline,
        );
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer.clone(),
            attachments: Vec::new(),
        });

        if !exec_result.had_blocks {
            let changed_now = current_changed_file_set().unwrap_or_default();
            let changed_delta = changed_files_delta(&changed_baseline, &changed_now);
            if let Some(msg) = evaluate_write_guard(
                &last_user,
                &answer,
                &changed_delta,
                &mut write_task_retries,
                &mut write_claim_retries,
            ) {
                history.push(ChatMessage {
                    role: "user".to_string(),
                    content: format!("{msg} {}", STRICT_TOOL_CALL_INSTRUCTION),
                    attachments: Vec::new(),
                });
                continue;
            }
            if changed_delta.is_empty() && looks_like_write_request(&last_user) {
                let err = "Write task failed: model did not produce executable tool_calls and no file changes were detected.";
                if try_executor_model_fallback(
                    cfg,
                    history,
                    system,
                    session,
                    render_markdown,
                    err,
                    allow_executor_fallback,
                )
                .await?
                {
                    return Ok(());
                }
                record_diagnostic(cfg, "write-hard-fail", err, session);
                println!("assistant> {}", user_visible_write_fail());
                return Ok(());
            }
            if looks_like_manual_action_answer(&answer)
                && (looks_like_write_request(&last_user) || looks_like_patch_request(&last_user))
            {
                let err = "Model attempted to delegate manual editing. This task requires executable tool_calls and real diffs.";
                if try_executor_model_fallback(
                    cfg,
                    history,
                    system,
                    session,
                    render_markdown,
                    err,
                    allow_executor_fallback,
                )
                .await?
                {
                    return Ok(());
                }
                record_diagnostic(cfg, "manual-delegation-blocked", err, session);
                println!("assistant> {}", user_visible_manual_delegation());
                return Ok(());
            }
            return Ok(());
        }

        if exec_result.executed_any {
            let (verification, recovery_hint) = print_execution_and_verification(&exec_result)?;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "{}\n{}{}\nContinue based on tool outputs above. If more execution is needed, emit JSON tool_calls. If complete, give final answer directly with short summary, changed files, and verification result.",
                    exec_result.history_text,
                    verification,
                    recovery_hint
                ),
                attachments: Vec::new(),
            });
            steps += 1;
            if steps >= MAX_AUTO_TOOL_STEPS {
                println!(
                    "assistant> Reached auto tool step limit ({}). Continue by describing next action.",
                    MAX_AUTO_TOOL_STEPS
                );
                return Ok(());
            }
            continue;
        }

        if exec_result.invalid_format && invalid_format_retries < MAX_INVALID_FORMAT_RETRIES {
            invalid_format_retries += 1;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Your last response had invalid tool_calls format. {}",
                    STRICT_TOOL_CALL_INSTRUCTION
                ),
                attachments: Vec::new(),
            });
            continue;
        }

        if exec_result.skipped_any && unsafe_retries < 1 {
            unsafe_retries += 1;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Your last response used unsupported execution format or unsafe commands. {}",
                    STRICT_TOOL_CALL_INSTRUCTION
                ),
                attachments: Vec::new(),
            });
            continue;
        }

        println!(
            "assistant> Detected tool calls, but skipped because commands are unsafe or unsupported.\n"
        );
        record_diagnostic(
            cfg,
            "tool-protocol",
            "legacy tool calls detected but skipped because unsafe/unsupported",
            session,
        );
        return Ok(());
    }
}

fn contains_legacy_shell_block(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("```bash")
        || t.contains("```sh")
        || t.contains("```powershell")
        || t.contains("```pwsh")
        || t.contains("```cmd")
}

fn contains_tool_call_hint(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("tool_calls")
        || t.contains("\"tool\"")
        || t.contains("code_execution")
        || t.contains("<think>")
        || t.contains("</think>")
        || t.contains("function_call")
        || t.contains("```json")
        || t.contains("``json")
        || t.contains("<|tool_call_argument_begin|>")
}

fn contains_code_execution_hint(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("code_execution")
        || t.contains("\"code_execution\"")
        || t.contains("<think>")
        || t.contains("</think>")
}

fn looks_like_file_write_claim(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let success = t.contains("created")
        || t.contains("written")
        || t.contains("saved")
        || t.contains("file created")
        || t.contains("analysis.md has been created")
        || t.contains("created in root")
        || t.contains("updated file");
    let zh_success = text.contains("已创建")
        || text.contains("已写入")
        || text.contains("已保存")
        || text.contains("创建了")
        || text.contains("写入了")
        || text.contains("文件已生成")
        || text.contains("已生成");
    let target_hint = has_file_target_hint(text);
    (success || zh_success) && target_hint
}

fn looks_like_write_request(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let action_en = t.contains("write ")
        || t.contains("create ")
        || t.contains("save ")
        || t.contains("generate ")
        || t.contains("modify ")
        || t.contains("edit ")
        || t.contains("update ")
        || t.contains("put in root");
    let action_zh = text.contains("写")
        || text.contains("创建")
        || text.contains("保存")
        || text.contains("生成")
        || text.contains("修改")
        || text.contains("编辑")
        || text.contains("更新")
        || text.contains("放在根目录");
    let negative = t.contains("do not write")
        || t.contains("don't write")
        || text.contains("不要写")
        || text.contains("不用写");
    !negative && (action_en || action_zh) && has_file_target_hint(text)
}

fn has_file_target_hint(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains(".md")
        || t.contains(".txt")
        || t.contains(".json")
        || t.contains(".toml")
        || t.contains(".yaml")
        || t.contains(".yml")
        || t.contains(".rs")
        || t.contains(".py")
        || t.contains(" file ")
        || t.contains("root directory")
        || t.contains("root/")
        || text.contains("文件")
        || text.contains("根目录")
}

fn evaluate_write_guard(
    last_user: &str,
    answer: &str,
    changed_delta: &[String],
    write_task_retries: &mut usize,
    write_claim_retries: &mut usize,
) -> Option<&'static str> {
    if changed_delta.is_empty()
        && looks_like_write_request(last_user)
        && *write_task_retries < MAX_WRITE_TASK_RETRIES
    {
        *write_task_retries += 1;
        return Some(WRITE_TASK_RETRY_MSG);
    }
    if changed_delta.is_empty()
        && looks_like_file_write_claim(answer)
        && *write_claim_retries < MAX_WRITE_CLAIM_RETRIES
    {
        *write_claim_retries += 1;
        return Some(WRITE_CLAIM_RETRY_MSG);
    }
    None
}

fn looks_like_patch_request(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("diff")
        || t.contains("patch")
        || t.contains("apply patch")
        || t.contains("implement")
        || t.contains("modify code")
        || t.contains("edit file")
        || text.contains("修改代码")
        || text.contains("改代码")
        || text.contains("补丁")
        || text.contains("实现")
}

fn looks_like_manual_action_answer(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("please manually")
        || t.contains("manually create")
        || t.contains("copy and paste")
        || t.contains("save as")
        || t.contains("动手改")
        || t.contains("手动")
        || t.contains("复制粘贴")
        || t.contains("你去改")
        || t.contains("请你自己改")
}

#[allow(dead_code)]
fn is_active_relay_provider(cfg: &Config) -> bool {
    cfg.model_profiles
        .get(&cfg.model)
        .map(|p| p.provider == ModelApiProvider::At)
        .unwrap_or(false)
}

#[allow(dead_code)]
fn should_force_tool_retry(cfg: &Config, answer: &str) -> bool {
    is_active_relay_provider(cfg)
        || active_effective_tool_mode(cfg) == ToolCallMode::Json
        || looks_like_manual_action_answer(answer)
}

async fn try_executor_model_fallback(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    _system: &str,
    session: Option<&str>,
    render_markdown: bool,
    reason: &str,
    allow_executor_fallback: bool,
) -> Result<bool> {
    if !allow_executor_fallback {
        return Ok(false);
    }
    let Some(executor_model) = cfg
        .executor_model
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
    else {
        return Ok(false);
    };
    if executor_model == cfg.model {
        return Ok(false);
    }

    ensure_model_catalog(cfg);
    if !cfg.model_catalog.iter().any(|m| m == &executor_model) {
        return Ok(false);
    }

    let before = current_changed_file_set().unwrap_or_default();
    let before_fp = snapshot_file_fingerprints(&before);
    let source_model = cfg.model.clone();
    record_diagnostic(
        cfg,
        "executor-fallback-start",
        &format!(
            "source_model={} executor_model={} reason={}",
            source_model, executor_model, reason
        ),
        session,
    );

    let mut exec_cfg = cfg.clone();
    set_active_model(&mut exec_cfg, &executor_model);
    let exec_system = build_system_prompt(&exec_cfg, "chat");
    let exec_res = Box::pin(run_agent_turn_with_system(
        &mut exec_cfg,
        history,
        &exec_system,
        session,
        render_markdown,
        false,
    ))
    .await;
    if let Err(err) = exec_res {
        let msg = format!("executor model request failed: {}", err);
        record_diagnostic(&exec_cfg, "executor-fallback-failed", &msg, session);
        return Ok(false);
    }

    let after = current_changed_file_set().unwrap_or_default();
    let after_fp = snapshot_file_fingerprints(&after);
    let delta = changed_files_delta(&before, &after);
    let content_changed = fingerprint_delta_exists(&before_fp, &after_fp);
    if delta.is_empty() && !content_changed {
        let msg = "executor fallback finished but still no detectable file changes were produced.";
        record_diagnostic(&exec_cfg, "executor-fallback-no-diff", msg, session);
        return Ok(false);
    }

    let msg = if !delta.is_empty() {
        format!(
            "executor fallback succeeded with {} changed file(s): {}",
            delta.len(),
            delta.join(", ")
        )
    } else {
        "executor fallback succeeded with content changes on existing changed files.".to_string()
    };
    record_diagnostic(&exec_cfg, "executor-fallback-succeeded", &msg, session);

    *cfg = exec_cfg;
    let _ = save_config(cfg);
    Ok(true)
}

fn last_user_text_from_native_messages(messages: &[Value]) -> Option<&str> {
    for m in messages.iter().rev() {
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or_default();
        if role != "user" {
            continue;
        }
        if let Some(s) = m.get("content").and_then(|v| v.as_str()) {
            return Some(s);
        }
    }
    None
}
fn extract_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut out = Vec::new();
    collect_tool_calls_from_fence(text, "```json", "```", false, &mut out);
    collect_tool_calls_from_inline_json(text, &mut out);
    collect_tool_calls_from_code_execution(text, &mut out);
    out
}
fn collect_tool_calls_from_fence(
    text: &str,
    open: &str,
    close: &str,
    skip_if_prev_backtick: bool,
    out: &mut Vec<ToolCall>,
) {
    let mut i = 0usize;
    while i < text.len() {
        let slice = &text[i..];
        let Some(pos) = slice.find(open) else {
            break;
        };
        let open_idx = i + pos;
        if skip_if_prev_backtick && open_idx > 0 && text.as_bytes()[open_idx - 1] == b'`' {
            i = open_idx + open.len();
            continue;
        }

        let mut json_start = open_idx + open.len();
        while json_start < text.len() {
            let b = text.as_bytes()[json_start];
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                json_start += 1;
            } else {
                break;
            }
        }
        if json_start >= text.len() {
            break;
        }

        let rest = &text[json_start..];
        let Some(end_rel) = rest.find(close) else {
            break;
        };

        let block = rest[..end_rel].trim();
        if !block.is_empty()
            && let Ok(value) = serde_json::from_str::<Value>(block)
        {
            collect_tool_calls_from_value(&value, out);
        }
        i = json_start + end_rel + close.len();
    }
}

async fn run_chat_turn(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    mode: &str,
    render_markdown: bool,
    active_skill: Option<&str>,
) -> Result<()> {
    let runtime_skill = active_skill.and_then(|name| find_skill(name).ok().flatten());
    set_runtime_active_skill(runtime_skill)?;
    let mut system = build_system_prompt_with_skill(cfg, mode, active_skill)?;
    maybe_compact_history(history, cfg);
    println!("{}", color_dim("(phase: response)"));
    print!(
        "{}",
        color_blue(&format!(
            "assistant[{}]({})> ",
            cfg.active_prompt, cfg.model
        ))
    );
    for attempt in 0..=1usize {
        let answer = match call_llm_with_history_stream_tools(cfg, &system, history, &native_tool_schemas()).await {
            Ok(v) => v,
            Err(err) => {
                record_diagnostic(cfg, "chat-lite-request", &err.to_string(), None);
                println!("\n");
                println!(
                    "assistant> Request interrupted: {}",
                    truncate_with_suffix(&err.to_string(), 220, " ...")
                );
                println!("assistant> You can continue chatting and send the next message.\n");
                return Ok(());
            }
        };
        println!("\n");

        let tool_calls = extract_tool_calls(&answer);
        if !tool_calls.is_empty() || contains_tool_call_hint(&answer) {
            if attempt == 0 {
                record_diagnostic(
                    cfg,
                    "chat-lite-tool-call",
                    "model returned tool_calls in chat-lite mode; retrying once with stricter no-tools instruction",
                    None,
                );
                system.push_str(
                    "\nStrict enforcement: Do not output JSON tool_calls, code_execution, <think>, or any execution plan. Reply in plain natural language only.",
                );
                continue;
            }
            let msg = "Model returned tool_calls in chat mode. Use /mode agent-force for execution tasks.";
            println!("assistant> {}\n", msg);
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: msg.to_string(),
                attachments: Vec::new(),
            });
            let _ = set_runtime_active_skill(None);
            return Ok(());
        }

        if !answer.trim().is_empty() {
            println!("{}", render_markdown_terminal(&answer, render_markdown));
            println!("\n");
        }
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer,
            attachments: Vec::new(),
        });
        let _ = set_runtime_active_skill(None);
        return Ok(());
    }
    let _ = set_runtime_active_skill(None);
    Ok(())
}

fn should_use_agent_for_input(input: &str, mode: ChatExecutionMode) -> bool {
    match mode {
        ChatExecutionMode::AgentForce => true,
        ChatExecutionMode::ChatOnly => false,
        ChatExecutionMode::AgentAuto => looks_like_agent_task(input),
    }
}

async fn should_use_agent_for_turn(
    cfg: &Config,
    history: &[ChatMessage],
    input: &str,
    mode: ChatExecutionMode,
) -> Result<bool> {
    match mode {
        ChatExecutionMode::AgentForce => Ok(true),
        ChatExecutionMode::ChatOnly => Ok(false),
        ChatExecutionMode::AgentAuto => {
            if looks_like_agent_task(input) {
                return Ok(true);
            }
            if let Some(v) = classify_mode_with_llm(cfg, history, input).await? {
                return Ok(v);
            }
            Ok(false)
        }
    }
}

async fn classify_mode_with_llm(
    cfg: &Config,
    history: &[ChatMessage],
    input: &str,
) -> Result<Option<bool>> {
    let mut router_history: Vec<ChatMessage> =
        history.iter().rev().take(4).rev().cloned().collect();
    router_history.push(ChatMessage {
        role: "user".to_string(),
        content: format!(
            "Route this request for terminal assistant mode.\n\
             Request: {input}\n\
             Output JSON only: {{\"mode\":\"agent\"|\"chat\",\"reason\":\"short\"}}"
        ),
        attachments: Vec::new(),
    });
    let system = "You are a strict mode router for coding assistant.\n\
Choose \"agent\" when task likely needs repo inspection, filesystem commands, file edits, test/build execution, or multi-step actions.\n\
Choose \"chat\" for explanation-only or conceptual Q&A.\n\
Output JSON only.";
    let out = call_llm_with_history(cfg, system, &router_history).await?;
    Ok(parse_router_mode(&out))
}

fn parse_router_mode(text: &str) -> Option<bool> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim())
        && let Some(mode) = v.get("mode").and_then(|m| m.as_str())
    {
        return Some(mode.eq_ignore_ascii_case("agent"));
    }

    if let Some(start) = text.find('{')
        && let Some(end) = find_matching_brace(text, start)
        && let Ok(v) = serde_json::from_str::<Value>(&text[start..=end])
        && let Some(mode) = v.get("mode").and_then(|m| m.as_str())
    {
        return Some(mode.eq_ignore_ascii_case("agent"));
    }

    let t = text.to_ascii_lowercase();
    if t.contains("\"mode\":\"agent\"") || t.contains("mode: agent") {
        return Some(true);
    }
    if t.contains("\"mode\":\"chat\"") || t.contains("mode: chat") {
        return Some(false);
    }
    None
}

fn looks_like_agent_task(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    let en_hit = [
        "fix ",
        "implement",
        "refactor",
        "analyze repo",
        "analyze project",
        "analyze this directory",
        "inspect repo",
        "inspect project",
        "edit ",
        "change ",
        "update ",
        "patch ",
        "apply ",
        "add feature",
        "write code",
        "run tests",
        "build ",
        "compile ",
        "create ",
        "write ",
        "modify ",
        "generate ",
        "save ",
    ]
    .iter()
    .any(|k| lower.contains(k));
    let zh_hit = [
        "修复",
        "实现",
        "重构",
        "分析目录",
        "分析项目",
        "看看目录",
        "检查仓库",
        "分析仓库",
        "修改",
        "编辑",
        "补丁",
        "写代码",
        "跑测试",
        "编译",
        "构建",
        "创建",
        "生成",
        "保存",
        "添加",
    ]
    .iter()
    .any(|k| input.contains(k));
    en_hit || zh_hit
}

fn print_execution_and_verification(exec_result: &ExecResult) -> Result<(String, String)> {
    println!("{}", color_dim("(phase: tool execution)"));
    let tool_calls = exec_result.display_text.matches("tool[").count();
    if exec_result.had_failures {
        println!("{} tool calls executed with failures.", tool_calls);
    } else {
        println!("{} tool calls executed.", tool_calls);
    }
    println!("{}", color_dim("(phase: verification)"));
    let verification = run_auto_verification()?;
    if !verification.trim().is_empty() && !verification.starts_with("verification: skipped") {
        println!("{} {}", color_dim("verify>"), verification);
    }
    let changed_now = current_changed_file_set().unwrap_or_default();
    let diff_preview = collect_diff_preview(&changed_now);
    if !diff_preview.trim().is_empty() {
        println!(
            "{} {}",
            color_dim("diff>"),
            diff_preview.lines().next().unwrap_or_default()
        );
    }
    let recovery_hint = if exec_result.had_failures {
        "\nSome commands failed. Prefer narrower retries: check file/path existence first, then rerun minimal commands.".to_string()
    } else {
        String::new()
    };
    let mut combined = verification;
    if !diff_preview.trim().is_empty() {
        combined.push('\n');
        combined.push_str(&diff_preview);
    }
    Ok((combined, recovery_hint))
}

fn run_auto_verification() -> Result<String> {
    let Some((label, cmd)) = pick_verification_command() else {
        return Ok("verification: skipped (no supported project checker detected)".to_string());
    };
    let out = run_shell_command(cmd)?;
    let status = if looks_like_command_failure(&out) {
        "failed"
    } else {
        "ok"
    };
    let clipped = clip_output(&out, 5000);
    Ok(format!(
        "verification[{label}] {status}\n$ {cmd}\n{clipped}"
    ))
}

fn collect_diff_preview(changed: &BTreeSet<String>) -> String {
    if changed.is_empty() {
        return "diff: no local changes".to_string();
    }

    let key = build_diff_cache_key(changed);
    let cache = DIFF_PREVIEW_CACHE.get_or_init(|| Mutex::new(DiffPreviewCache::default()));
    if let Ok(guard) = cache.lock()
        && guard.key == key
        && !guard.preview.is_empty()
    {
        return guard.preview.clone();
    }

    if !is_git_repo() {
        let preview = collect_fs_diff_preview(changed);
        if let Ok(mut guard) = cache.lock() {
            guard.key = key;
            guard.preview = preview.clone();
        }
        return preview;
    }

    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--no-color").arg("--");
    for p in changed.iter().take(MAX_DIFF_PREVIEW_FILES) {
        cmd.arg(p);
    }
    let output = cmd.output();
    let Ok(output) = output else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let mut preview = if text.trim().is_empty() {
        let untracked: Vec<String> = list_workspace_untracked_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| changed.contains(p))
            .take(MAX_DIFF_PREVIEW_FILES)
            .collect();
        if untracked.is_empty() {
            "diff: no local changes".to_string()
        } else {
            let mut out = String::from("diff preview:\n(untracked files)\n");
            for p in &untracked {
                out.push_str(&format!("+ {}\n", p));
            }
            if changed.len() > MAX_DIFF_PREVIEW_FILES {
                out.push_str(&format!(
                    "(diff limited to first {} changed files; total changed: {})",
                    MAX_DIFF_PREVIEW_FILES,
                    changed.len()
                ));
            }
            out
        }
    } else {
        let mut out = format!("diff preview:\n{}", clip_output(&text, 5000));
        if changed.len() > MAX_DIFF_PREVIEW_FILES {
            out.push_str(&format!(
                "\n(diff limited to first {} changed files; total changed: {})",
                MAX_DIFF_PREVIEW_FILES,
                changed.len()
            ));
        }
        out
    };
    if preview.is_empty() {
        preview = "diff: no local changes".to_string();
    }

    if let Ok(mut guard) = cache.lock() {
        guard.key = key;
        guard.preview = preview.clone();
    }
    preview
}

fn collect_fs_diff_preview(changed: &BTreeSet<String>) -> String {
    let root = match std::env::current_dir() {
        Ok(v) => v,
        Err(err) => return format!("diff: fs snapshot unavailable ({err})"),
    };
    let current = match collect_fs_snapshot(&root) {
        Ok(v) => v,
        Err(err) => return format!("diff: fs snapshot unavailable ({err})"),
    };
    let baseline_lock = FS_BASELINE_SNAPSHOT.get_or_init(|| Mutex::new(None));
    let baseline = match baseline_lock.lock() {
        Ok(guard) => guard.clone().unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    };

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for p in changed {
        match (baseline.get(p), current.get(p)) {
            (None, Some(cur)) => added.push((p.clone(), cur.clone())),
            (Some(old), Some(cur)) if old != cur => modified.push((p.clone(), old.clone(), cur.clone())),
            (Some(old), None) => deleted.push((p.clone(), old.clone())),
            _ => {}
        }
    }

    added.sort_by(|a, b| a.0.cmp(&b.0));
    modified.sort_by(|a, b| a.0.cmp(&b.0));
    deleted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("diff preview (filesystem):\n");
    out.push_str(&format!(
        "A:{} M:{} D:{} (total changed: {})\n",
        added.len(),
        modified.len(),
        deleted.len(),
        changed.len()
    ));

    if !added.is_empty() {
        out.push_str("\n[Added]\n");
        for (path, cur) in added.iter().take(MAX_DIFF_PREVIEW_FILES) {
            out.push_str(&format!(
                "+ {}  size={}  mtime={}  digest={}\n",
                path,
                cur.size,
                cur.mtime_unix,
                truncate_with_suffix(&cur.digest, 18, "")
            ));
            if let Some(preview) = preview_text_file(path) {
                out.push_str(&format!("  preview: {}\n", preview));
            }
        }
    }

    if !modified.is_empty() {
        out.push_str("\n[Modified]\n");
        for (path, old, cur) in modified.iter().take(MAX_DIFF_PREVIEW_FILES) {
            out.push_str(&format!(
                "~ {}  size:{}->{}  mtime:{}->{}  digest:{}->{}\n",
                path,
                old.size,
                cur.size,
                old.mtime_unix,
                cur.mtime_unix,
                truncate_with_suffix(&old.digest, 12, ""),
                truncate_with_suffix(&cur.digest, 12, "")
            ));
            if let Some(preview) = preview_text_file(path) {
                out.push_str(&format!("  head: {}\n", preview));
            }
        }
    }

    if !deleted.is_empty() {
        out.push_str("\n[Deleted]\n");
        for (path, old) in deleted.iter().take(MAX_DIFF_PREVIEW_FILES) {
            out.push_str(&format!(
                "- {}  size={}  mtime={}  digest={}\n",
                path,
                old.size,
                old.mtime_unix,
                truncate_with_suffix(&old.digest, 18, "")
            ));
        }
    }

    if added.len() + modified.len() + deleted.len() > MAX_DIFF_PREVIEW_FILES {
        out.push_str(&format!(
            "\n(diff limited to first {} paths per section)\n",
            MAX_DIFF_PREVIEW_FILES
        ));
    }
    out
}

fn preview_text_file(rel: &str) -> Option<String> {
    let p = Path::new(rel);
    if !looks_like_text_path(p) {
        return None;
    }
    let data = fs::read(p).ok()?;
    let text = String::from_utf8(data).ok()?;
    let compact = text.replace('\n', "\\n");
    Some(truncate_with_suffix(
        &compact,
        MAX_FS_PREVIEW_TEXT_BYTES,
        " ...",
    ))
}

fn looks_like_text_path(p: &Path) -> bool {
    let ext = p
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "rs"
            | "toml"
            | "md"
            | "txt"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "css"
            | "html"
            | "py"
            | "rpy"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "go"
            | "java"
            | "kt"
            | "swift"
    )
}

fn build_diff_cache_key(changed: &BTreeSet<String>) -> String {
    let mut out = changed
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    for p in changed {
        out.push_str(p);
        out.push(':');
        out.push_str(&fingerprint_for_workspace_path(p));
        out.push('\n');
    }
    out
}

fn pick_verification_command() -> Option<(&'static str, &'static str)> {
    if Path::new("Cargo.toml").exists() {
        return Some(("rust", "cargo check"));
    }
    if Path::new("pnpm-lock.yaml").exists() && Path::new("tsconfig.json").exists() {
        return Some(("typescript", "pnpm -s tsc --noEmit"));
    }
    if Path::new("package.json").exists() && Path::new("tsconfig.json").exists() {
        return Some(("typescript", "npm exec -y tsc --noEmit"));
    }
    if Path::new("pyproject.toml").exists() || Path::new("pytest.ini").exists() {
        return Some(("python", "pytest -q"));
    }
    None
}
fn collect_tool_calls_from_inline_json(text: &str, out: &mut Vec<ToolCall>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let Some(end) = find_matching_brace(text, i) else {
            i += 1;
            continue;
        };
        let candidate = &text[i..=end];
        if candidate.contains("\"tool_calls\"")
            && let Ok(value) = serde_json::from_str::<Value>(candidate)
        {
            collect_tool_calls_from_value(&value, out);
            i = end + 1;
            continue;
        }
        i += 1;
    }
}

fn collect_tool_calls_from_code_execution(text: &str, out: &mut Vec<ToolCall>) {
    let marker = "code_execution";
    let mut i = 0usize;
    while i < text.len() {
        let Some(pos_rel) = text[i..].find(marker) else {
            break;
        };
        let pos = i + pos_rel;
        let Some(obj_start_rel) = text[pos..].find('{') else {
            i = pos + marker.len();
            continue;
        };
        let obj_start = pos + obj_start_rel;
        let Some(obj_end) = find_matching_brace(text, obj_start) else {
            i = obj_start + 1;
            continue;
        };
        let candidate = &text[obj_start..=obj_end];
        if let Ok(value) = serde_json::from_str::<Value>(candidate)
            && let Some(code) = value.get("code").and_then(|v| v.as_str())
        {
            collect_tool_calls_from_python_code(code, out);
        }
        i = obj_end + 1;
    }
}

fn collect_tool_calls_from_python_code(code: &str, out: &mut Vec<ToolCall>) {
    if let Some((path, content)) = parse_python_write_file(code) {
        out.push(ToolCall {
            tool: "fs_create_file".to_string(),
            command: String::new(),
            args: json!({
                "path": path,
                "content": content,
                "overwrite": true
            }),
        });
        return;
    }
    if let Some(path) = parse_python_read_file(code) {
        out.push(ToolCall {
            tool: "fs_read_file".to_string(),
            command: String::new(),
            args: json!({ "path": path }),
        });
        return;
    }
    if let Some(path) = parse_python_listdir(code) {
        out.push(ToolCall {
            tool: "fs_list_files".to_string(),
            command: String::new(),
            args: json!({ "path": path }),
        });
    }
}

fn parse_python_write_file(code: &str) -> Option<(String, String)> {
    let open_pos = code.find("open(")?;
    let open_args = &code[open_pos + "open(".len()..];
    let (path, _) = parse_python_string_literal(open_args)?;
    let open_close = open_args.find(')')?;
    let mode_seg = &open_args[..open_close].to_ascii_lowercase();
    if !(mode_seg.contains("'w'") || mode_seg.contains("\"w\"")) {
        return None;
    }

    let write_pos = code.find(".write(")?;
    let write_args = &code[write_pos + ".write(".len()..];
    let (content, _) = parse_python_string_literal(write_args)?;
    Some((path, content))
}

fn parse_python_read_file(code: &str) -> Option<String> {
    let open_pos = code.find("open(")?;
    let open_args = &code[open_pos + "open(".len()..];
    let (path, _) = parse_python_string_literal(open_args)?;
    let open_close = open_args.find(')')?;
    let mode_seg = &open_args[..open_close].to_ascii_lowercase();
    if mode_seg.contains("'w'") || mode_seg.contains("\"w\"") {
        return None;
    }
    Some(path)
}

fn parse_python_listdir(code: &str) -> Option<String> {
    let pos = code.find("os.listdir(")?;
    let args = &code[pos + "os.listdir(".len()..];
    let trimmed = args.trim_start();
    if trimmed.starts_with(')') {
        return Some(".".to_string());
    }
    let (path, _) = parse_python_string_literal(trimmed)?;
    Some(path)
}

fn parse_python_string_literal(input: &str) -> Option<(String, usize)> {
    let s = input.trim_start();
    let skipped = input.len() - s.len();
    if s.len() < 2 {
        return None;
    }

    for quote in ["\"\"\"", "'''"] {
        if let Some(body) = s.strip_prefix(quote)
            && let Some(end) = body.find(quote)
        {
            let content = body[..end].to_string();
            return Some((content, skipped + quote.len() + end + quote.len()));
        }
    }

    let first = s.chars().next()?;
    if first != '"' && first != '\'' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    let mut end_idx = None;
    for (idx, ch) in s.char_indices().skip(1) {
        if escaped {
            let decoded = match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                '"' => '"',
                '\'' => '\'',
                other => other,
            };
            out.push(decoded);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == first {
            end_idx = Some(idx);
            break;
        }
        out.push(ch);
    }
    let end = end_idx?;
    Some((out, skipped + end + 1))
}

fn find_matching_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(start).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
fn collect_tool_calls_from_value(value: &Value, out: &mut Vec<ToolCall>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_tool_calls_from_value(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(calls) = map.get("tool_calls") {
                collect_tool_calls_from_value(calls, out);
                return;
            }
            let tool = map
                .get("tool")
                .or_else(|| map.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let command = map
                .get("command")
                .or_else(|| map.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let args = if let Some(v) = map.get("args") {
                v.clone()
            } else {
                let mut m = serde_json::Map::new();
                for (k, v) in map {
                    if k == "tool"
                        || k == "type"
                        || k == "command"
                        || k == "cmd"
                        || k == "tool_calls"
                    {
                        continue;
                    }
                    m.insert(k.clone(), v.clone());
                }
                Value::Object(m)
            };

            let has_args = matches!(&args, Value::Object(m) if !m.is_empty());
            if !tool.trim().is_empty() && (!command.trim().is_empty() || has_args) {
                out.push(ToolCall {
                    tool,
                    command,
                    args,
                });
            }
        }
        _ => {}
    }
}
fn is_command_allowed(cfg: &Config, cmd: &str) -> bool {
    if matches_list(&cfg.auto_exec_deny, cmd) {
        return false;
    }
    match cfg.auto_exec_mode {
        AutoExecMode::All => true,
        AutoExecMode::Safe => is_safe_auto_exec_command(cmd),
        AutoExecMode::Custom => matches_list(&cfg.auto_exec_allow, cmd),
    }
}

fn matches_list(list: &[String], cmd: &str) -> bool {
    let cmd_l = cmd.to_ascii_lowercase();
    list.iter().any(|item| {
        let s = item.trim().to_ascii_lowercase();
        !s.is_empty() && cmd_l.starts_with(&s)
    })
}

fn is_trusted_command(cfg: &Config, cmd: &str) -> bool {
    if matches_list(&cfg.auto_exec_trusted, cmd) {
        return true;
    }
    if let Some(skill) = runtime_active_skill() {
        return matches_list(&skill.manifest.trusted_commands, cmd);
    }
    false
}

fn command_prefix(cmd: &str) -> String {
    let mut it = cmd.split_whitespace();
    let first = it.next().unwrap_or("").to_string();
    if first.eq_ignore_ascii_case("git")
        && let Some(second) = it.next()
    {
        return format!("git {}", second);
    }
    first
}

fn is_safe_auto_exec_command(cmd: &str) -> bool {
    let mut parts = cmd.split_whitespace();
    let Some(first) = parts.next() else {
        return false;
    };
    let f = first.to_ascii_lowercase();
    if matches!(
        f.as_str(),
        "ls" | "dir" | "pwd" | "cat" | "type" | "rg" | "grep" | "findstr" | "tree" | "find"
    ) {
        return true;
    }
    if f == "get-childitem" || f == "get-content" || f == "get-location" {
        return true;
    }
    if f == "git"
        && let Some(second) = parts.next()
    {
        let s = second.to_ascii_lowercase();
        return matches!(s.as_str(), "status" | "diff" | "log" | "show" | "branch");
    }
    false
}

fn run_shell_command(cmd: &str) -> Result<String> {
    let short = if cmd.chars().count() > 48 {
        format!("exec {}...", prefix_chars(cmd, 48))
    } else {
        format!("exec {}", cmd)
    };
    let working = WorkingStatus::start(short);

    if let Some(v) = run_translated_safe_command(cmd)? {
        working.finish();
        return Ok(v);
    }

    let output = if cfg!(target_os = "windows") {
        let normalized = normalize_windows_shell_command(cmd);
        let wrapped = format!(
            "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {}",
            normalized
        );
        Command::new("powershell")
            .args(["-NoProfile", "-Command", &wrapped])
            .output()
            .with_context(|| format!("Failed to run command: {cmd}"))?
    } else {
        Command::new("sh")
            .args(["-lc", cmd])
            .output()
            .with_context(|| format!("Failed to run command: {cmd}"))?
    };

    let stdout = decode_command_output(&output.stdout);
    let stderr = decode_command_output(&output.stderr);
    let mut out = String::new();
    if !stdout.trim().is_empty() {
        out.push_str(&stdout);
    }
    if !stderr.trim().is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&stderr);
    }
    if out.trim().is_empty() {
        out = "(no output)".to_string();
    }
    working.finish();
    Ok(out)
}

fn normalize_windows_shell_command(cmd: &str) -> String {
    // Windows PowerShell 5.1 does not support "&&"; convert to sequential separator.
    // This keeps common model-generated commands like `cd path && ls -la` runnable.
    let mut out = String::with_capacity(cmd.len());
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '&' if !in_single && !in_double && chars.peek() == Some(&'&') => {
                let _ = chars.next();
                out.push_str("; ");
            }
            _ => out.push(ch),
        }
    }
    out
}
fn run_translated_safe_command(cmd: &str) -> Result<Option<String>> {
    if !cfg!(target_os = "windows") {
        return Ok(None);
    }
    let trimmed = cmd.trim();
    if trimmed.starts_with("grep ") {
        return Ok(Some(run_windows_grep_translation(trimmed)?));
    }
    if trimmed.starts_with("find ") {
        return Ok(Some(run_windows_find_translation(trimmed)?));
    }
    Ok(None)
}

fn run_windows_grep_translation(cmd: &str) -> Result<String> {
    let pattern = extract_quoted(cmd).unwrap_or_else(|| ".*".to_string());
    let pattern = pattern.replace("\\|", "|");
    let glob = parse_flag_value(cmd, "--include=").unwrap_or_else(|| "*.txt".to_string());
    let path = if cmd.contains(" . ") || cmd.ends_with(" .") {
        ".".to_string()
    } else {
        ".".to_string()
    };
    let limit = parse_head_limit(cmd).unwrap_or(30);

    let out = Command::new("rg")
        .args(["-n", "-g", &glob, &pattern, &path])
        .output();
    let Ok(out) = out else {
        return Ok("rg not found; cannot translate grep on Windows.".to_string());
    };
    let txt = decode_command_output(&out.stdout);
    Ok(limit_lines(&txt, limit))
}

fn run_windows_find_translation(cmd: &str) -> Result<String> {
    let path = cmd.split_whitespace().nth(1).unwrap_or(".");
    let glob = parse_name_glob(cmd).unwrap_or_else(|| "*".to_string());
    let limit = parse_head_limit(cmd).unwrap_or(20);

    let out = Command::new("rg")
        .args(["--files", "-g", &glob, path])
        .output();
    let Ok(out) = out else {
        return Ok("rg not found; cannot translate find on Windows.".to_string());
    };
    let txt = decode_command_output(&out.stdout);
    Ok(limit_lines(&txt, limit))
}

fn parse_flag_value(cmd: &str, prefix: &str) -> Option<String> {
    for token in cmd.split_whitespace() {
        if let Some(v) = token.strip_prefix(prefix) {
            return Some(trim_quotes(v).to_string());
        }
    }
    None
}

fn parse_name_glob(cmd: &str) -> Option<String> {
    let marker = "-name";
    let idx = cmd.find(marker)?;
    let rest = cmd[idx + marker.len()..].trim();
    let tok = rest.split_whitespace().next()?;
    Some(trim_quotes(tok).to_string())
}

fn parse_head_limit(cmd: &str) -> Option<usize> {
    let marker = "head -";
    let idx = cmd.find(marker)?;
    let rest = &cmd[idx + marker.len()..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse::<usize>().ok()
}

fn trim_quotes(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'')
}

fn limit_lines(s: &str, n: usize) -> String {
    s.lines().take(n).collect::<Vec<_>>().join("\n")
}

fn session_path(session: &str) -> Result<std::path::PathBuf> {
    Ok(config_dir()?
        .join("sessions")
        .join(format!("{session}.json")))
}

fn sessions_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("sessions"))
}

fn resolve_session_name(requested: &str) -> Result<String> {
    let name = if requested == "default" || requested == "auto" {
        workspace_session_base()?
    } else {
        requested.to_string()
    };
    Ok(sanitize_session_name(&name))
}

fn fresh_session_name_for_workspace() -> Result<String> {
    let base = workspace_session_base()?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(format!("{}-{}", sanitize_session_name(&base), ts))
}

fn workspace_session_base() -> Result<String> {
    let cwd = std::env::current_dir()?;
    let cwd = cwd.to_string_lossy().to_string();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hasher);
    let hash = hasher.finish();
    let leaf = Path::new(&cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("workspace");
    Ok(format!("ws-{}-{:x}", leaf, hash))
}

fn sanitize_session_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "session".to_string()
    } else {
        s
    }
}

fn load_session_or_default(session: &str) -> Result<Vec<ChatMessage>> {
    let path = session_path(session)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: Vec<ChatMessage> = serde_json::from_str(&text)
        .with_context(|| format!("Invalid session JSON: {}", path.display()))?;
    let repaired = parsed
        .into_iter()
        .map(|mut m| {
            m.content = fix_mojibake_if_needed(&m.content);
            m
        })
        .collect();
    Ok(repaired)
}

fn save_session(session: &str, messages: &[ChatMessage]) -> Result<()> {
    let path = session_path(session)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create session dir {}", parent.display()))?;
    }
    let normalized: Vec<ChatMessage> = messages
        .iter()
        .cloned()
        .map(|mut m| {
            m.content = fix_mojibake_if_needed(&m.content);
            m
        })
        .collect();
    let text = serde_json::to_string_pretty(&normalized)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub async fn run_agent_task(mut cfg: Config, session: &str, task: &str) -> Result<()> {
    let active_session = resolve_session_name(session)?;
    println!("== dongshan agent ({active_session}) ==");
    let mut history = load_session_or_default(&active_session)?;
    let augmented_input = augment_user_input_with_workspace_context(task)?;
    history.push(ChatMessage {
        role: "user".to_string(),
        content: augmented_input,
        attachments: Vec::new(),
    });

    maybe_compact_history(&mut history, &cfg);
    let active_skill = load_active_skill_for_session(&active_session)?;
    run_agent_turn(
        &mut cfg,
        &mut history,
        "chat",
        Some(&active_session),
        true,
        active_skill.as_deref(),
    )
    .await?;
    save_session(&active_session, &history)?;

    let changed = list_workspace_changed_files()?;
    if changed.is_empty() {
        println!("agent> no tracked workspace changes detected.");
    } else {
        println!("agent> changed files:");
        for file in changed {
            println!("- {}", file);
        }
    }
    Ok(())
}

fn print_status(cfg: &Config) -> Result<()> {
    let provider = cfg
        .model_profiles
        .get(&cfg.model)
        .map(|p| format!("{:?}", p.provider))
        .unwrap_or_else(|| "unknown".to_string());
    let tool_mode = format!("{:?}", active_effective_tool_mode(cfg));
    println!("model: {}", cfg.model);
    println!("provider: {}", provider);
    println!("tool_mode: {}", tool_mode);
    println!(
        "executor_model: {}",
        cfg.executor_model.as_deref().unwrap_or("(none)")
    );
    println!(
        "fallback_models: {}",
        if cfg.fallback_models.is_empty() {
            "(none)".to_string()
        } else {
            cfg.fallback_models.join(", ")
        }
    );
    let changed = list_workspace_changed_files()?;
    println!("changed_files: {}", changed.len());
    for p in changed.iter().take(8) {
        println!("- {}", p);
    }
    if let Some(diag) = read_last_diagnostic() {
        println!(
            "last_error: [{}] {}",
            diag.phase,
            truncate_with_suffix(&diag.message, 180, " ...")
        );
    } else {
        println!("last_error: (none)");
    }
    Ok(())
}

fn record_diagnostic(cfg: &Config, phase: &str, message: &str, session: Option<&str>) {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    let diag = LastDiagnostic {
        timestamp_unix: now_unix_ts(),
        model: cfg.model.clone(),
        phase: phase.to_string(),
        message: message.to_string(),
        session: session.map(|s| s.to_string()),
        cwd,
    };
    let _ = write_last_diagnostic(&diag);
}

fn record_step_artifact_from_native(
    session: Option<&str>,
    cfg: &Config,
    step: usize,
    phase: &str,
    prompt_mode: &str,
    messages: &[Value],
    response: &str,
    tool_calls: &[Value],
    exec_result: Option<&ExecResult>,
    changed_baseline: &BTreeSet<String>,
) {
    let request = last_user_text_from_native_messages(messages)
        .unwrap_or_default()
        .to_string();
    record_step_artifact(
        session,
        cfg,
        step,
        phase,
        prompt_mode,
        &request,
        response,
        tool_calls,
        exec_result,
        changed_baseline,
    );
}

fn record_step_artifact(
    session: Option<&str>,
    cfg: &Config,
    step: usize,
    phase: &str,
    prompt_mode: &str,
    request: &str,
    response: &str,
    tool_calls: &[Value],
    exec_result: Option<&ExecResult>,
    changed_baseline: &BTreeSet<String>,
) {
    let Some(session) = session else {
        return;
    };
    let changed_now = current_changed_file_set().unwrap_or_default();
    let changed_files = changed_files_delta(changed_baseline, &changed_now);
    let artifact = TurnArtifact {
        timestamp_unix: now_unix_ts(),
        session: session.to_string(),
        model: cfg.model.clone(),
        step,
        phase: phase.to_string(),
        prompt_mode: prompt_mode.to_string(),
        request: truncate_with_suffix(request, 8000, "..."),
        response: truncate_with_suffix(response, 12000, "..."),
        tool_calls: tool_calls.to_vec(),
        executed_any: exec_result.map(|e| e.executed_any).unwrap_or(false),
        had_failures: exec_result.map(|e| e.had_failures).unwrap_or(false),
        changed_files,
    };
    let _ = write_turn_artifact(session, &artifact);
}

fn native_calls_to_values(
    calls: &[NativeFunctionCall],
    cfg: &Config,
    session: Option<&str>,
) -> Vec<Value> {
    calls
        .iter()
        .map(|c| {
            let args = match serde_json::from_str::<Value>(&c.arguments) {
                Ok(v) => v,
                Err(e) => {
                    record_diagnostic(
                        cfg,
                        "native-call-args-parse",
                        &format!("tool={} id={} parse_error={}", c.name, c.id, e),
                        session,
                    );
                    json!({ "raw_arguments": c.arguments })
                }
            };
            json!({
                "id": c.id,
                "name": c.name,
                "args": args
            })
        })
        .collect()
}

fn tool_calls_from_text(text: &str) -> Vec<Value> {
    extract_tool_calls(text)
        .into_iter()
        .map(|c| {
            json!({
                "tool": c.tool,
                "command": c.command,
                "args": c.args
            })
        })
        .collect()
}

fn list_saved_sessions() -> Result<Vec<String>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("Failed to read session dir {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        names.push(stem.to_string());
    }
    names.sort();
    Ok(names)
}

fn remove_session_file(session: &str) -> Result<bool> {
    let path = session_path(session)?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("Failed to remove {}", path.display()))?;
    Ok(true)
}

fn list_workspace_changed_files() -> Result<Vec<String>> {
    if !is_git_repo() {
        return list_workspace_changed_files_fs();
    }
    let entries = read_git_status_entries()?;
    let mut files: Vec<String> = entries.into_iter().map(|(_, p)| p).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

fn list_workspace_untracked_files() -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (status, path) in read_git_status_entries()? {
        if status == "??" {
            out.push(path);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn read_git_status_entries() -> Result<Vec<(String, String)>> {
    let out = Command::new("git").args(["status", "--porcelain"]).output();
    let Ok(out) = out else {
        return Ok(vec![]);
    };
    if !out.status.success() {
        return Ok(vec![]);
    }
    let text = decode_command_output(&out.stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = line[..2].to_string();
        let path = line[3..].trim();
        if !path.is_empty() {
            entries.push((status, path.to_string()));
        }
    }
    Ok(entries)
}

fn is_git_repo() -> bool {
    let out = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    decode_command_output(&out.stdout).trim() == "true"
}

fn list_workspace_changed_files_fs() -> Result<Vec<String>> {
    let current = collect_fs_snapshot(std::env::current_dir()?.as_path())?;
    let baseline_lock = FS_BASELINE_SNAPSHOT.get_or_init(|| Mutex::new(None));
    let mut guard = baseline_lock
        .lock()
        .map_err(|_| anyhow::anyhow!("failed to lock fs baseline snapshot"))?;
    if guard.is_none() {
        *guard = Some(current);
        return Ok(Vec::new());
    }
    let baseline = guard.clone().unwrap_or_default();
    drop(guard);
    let mut changed = BTreeSet::new();
    for (p, c) in &current {
        match baseline.get(p) {
            None => {
                changed.insert(p.clone());
            }
            Some(b) if b != c => {
                changed.insert(p.clone());
            }
            _ => {}
        }
    }
    for p in baseline.keys() {
        if !current.contains_key(p) {
            changed.insert(p.clone());
        }
    }
    Ok(changed.into_iter().collect())
}

fn collect_fs_snapshot(root: &Path) -> Result<BTreeMap<String, FsEntry>> {
    let mut out = BTreeMap::new();
    collect_fs_snapshot_walk(root, root, &mut out)?;
    Ok(out)
}

fn collect_fs_snapshot_walk(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, FsEntry>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let rel_str = normalize_rel_path(rel);
        if should_skip_fs_path(&rel_str) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime_unix = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if meta.is_dir() {
            out.insert(
                rel_str.clone(),
                FsEntry {
                    is_dir: true,
                    size: 0,
                    mtime_unix,
                    digest: "dir".to_string(),
                },
            );
            collect_fs_snapshot_walk(root, &path, out)?;
        } else {
            out.insert(
                rel_str.clone(),
                FsEntry {
                    is_dir: false,
                    size: meta.len(),
                    mtime_unix,
                    digest: compute_fs_digest(&path, meta.len(), mtime_unix),
                },
            );
        }
    }
    Ok(())
}

fn normalize_rel_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn should_skip_fs_path(rel: &str) -> bool {
    rel.starts_with(".git/")
        || rel == ".git"
        || rel.starts_with("target/")
        || rel == "target"
        || rel.starts_with("node_modules/")
        || rel == "node_modules"
        || rel.starts_with(".dongshan/")
        || rel == ".dongshan"
}

fn compute_fs_digest(path: &Path, size: u64, mtime_unix: i64) -> String {
    if size == 0 {
        return "empty".to_string();
    }
    if size > MAX_FS_SNAPSHOT_HASH_BYTES {
        return format!("meta:{size}:{mtime_unix}");
    }
    let Ok(bytes) = fs::read(path) else {
        return format!("meta:{size}:{mtime_unix}:read-error");
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("h:{:x}", hasher.finish())
}

fn current_changed_file_set() -> Result<BTreeSet<String>> {
    Ok(list_workspace_changed_files()?.into_iter().collect())
}

fn print_changed_files_delta(before: &BTreeSet<String>) -> Result<()> {
    let after = current_changed_file_set()?;
    if &after == before {
        return Ok(());
    }

    println!("{}", color_dim("changed files:"));
    for p in after.iter().filter(|p| !before.contains(*p)) {
        println!("{}", color_green(&format!("+ {}", p)));
    }
    for p in after.iter().filter(|p| before.contains(*p)) {
        println!("{}", color_yellow(&format!("~ {}", p)));
    }
    for p in before.iter().filter(|p| !after.contains(*p)) {
        println!("{}", color_red(&format!("- {}", p)));
    }
    Ok(())
}

fn changed_files_delta(before: &BTreeSet<String>, after: &BTreeSet<String>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for p in after.iter().filter(|p| !before.contains(*p)) {
        out.insert(p.to_string());
    }
    for p in before.iter().filter(|p| !after.contains(*p)) {
        out.insert(p.to_string());
    }
    out.into_iter().collect()
}

fn snapshot_file_fingerprints(paths: &BTreeSet<String>) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| (p.clone(), fingerprint_for_workspace_path(p)))
        .collect()
}

fn fingerprint_delta_exists(before: &[(String, String)], after: &[(String, String)]) -> bool {
    let mut b = std::collections::BTreeMap::new();
    let mut a = std::collections::BTreeMap::new();
    for (k, v) in before {
        b.insert(k.clone(), v.clone());
    }
    for (k, v) in after {
        a.insert(k.clone(), v.clone());
    }
    let keys: BTreeSet<String> = b.keys().chain(a.keys()).cloned().collect();
    for k in keys {
        if b.get(&k) != a.get(&k) {
            return true;
        }
    }
    false
}

fn fingerprint_for_workspace_path(rel: &str) -> String {
    let Ok(cwd) = std::env::current_dir() else {
        return "cwd-error".to_string();
    };
    let p = cwd.join(rel);
    let Ok(meta) = fs::metadata(&p) else {
        return "missing".to_string();
    };
    if meta.is_dir() {
        return format!("dir:{}", meta.len());
    }
    let Ok(bytes) = fs::read(&p) else {
        return format!("file:{}:read-error", meta.len());
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("file:{}:{:x}", bytes.len(), hasher.finish())
}

fn guessed_changed_files_for_call(call: &ToolCall) -> Vec<String> {
    let tool = call.tool.trim().to_ascii_lowercase();
    let mut out = BTreeSet::new();
    match tool.as_str() {
        "fs.create_file" | "fs.edit_file" | "fs.apply_patch" | "fs.delete" => {
            if let Some(path) = tool_arg_string(call, &["path", "file", "target"]) {
                out.insert(path);
            }
        }
        "fs.move" => {
            if let Some(path) = tool_arg_string(call, &["from", "src", "source"]) {
                out.insert(path);
            }
            if let Some(path) = tool_arg_string(call, &["to", "dst", "target"]) {
                out.insert(path);
            }
        }
        _ => {}
    }
    out.into_iter().collect()
}

fn decode_command_output(bytes: &[u8]) -> String {
    if let Ok(utf8) = std::str::from_utf8(bytes) {
        return fix_mojibake_if_needed(utf8);
    }
    let (decoded, _, _) = GBK.decode(bytes);
    let gbk_text = decoded.into_owned();
    let repaired = fix_mojibake_if_needed(&gbk_text);
    if repaired.trim().is_empty() {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        repaired
    }
}

fn fix_mojibake_if_needed(input: &str) -> String {
    if !looks_like_utf8_as_gbk_mojibake(input) {
        return input.to_string();
    }
    let (gbk_bytes, _, _) = GBK.encode(input);
    match String::from_utf8(gbk_bytes.into_owned()) {
        Ok(candidate) if looks_more_readable_chinese(&candidate, input) => candidate,
        _ => input.to_string(),
    }
}

fn looks_like_utf8_as_gbk_mojibake(s: &str) -> bool {
    if !s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)) {
        return false;
    }
    let suspicious = s
        .chars()
        .filter(|c| "鍙鍑鍦鍧鍚鍛鏄鏃鏂鏁鏍鐨鍏ュ彛鎴".contains(*c))
        .count();
    let common = s
        .chars()
        .filter(|c| "的是了在和有我你他她它中为就不也很函数程序入口文件模型提示".contains(*c))
        .count();
    suspicious >= 2 && suspicious > common
}

fn looks_more_readable_chinese(candidate: &str, original: &str) -> bool {
    fn score(x: &str) -> isize {
        let common = x
            .chars()
            .filter(|c| "的是了在和有我你他她它中为就不也很函数程序入口文件模型提示".contains(*c))
            .count() as isize;
        let weird = x
            .chars()
            .filter(|c| "鍙鍑鍦鍧鍚鍛鏄鏃鏂鏁鏍鐨鍏".contains(*c))
            .count() as isize;
        let replacement = x.matches('\u{FFFD}').count() as isize;
        common * 2 - weird * 2 - replacement * 3
    }
    score(candidate) > score(original)
}

