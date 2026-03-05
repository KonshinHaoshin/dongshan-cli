use std::fs;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use encoding_rs::GBK;
use serde::Serialize;
use serde_json::{Value, json};

use crate::chat_context::augment_user_input_with_workspace_context;
use crate::config::{
    AutoExecMode, Config, build_system_prompt, config_dir, current_prompt_text, ensure_model_catalog,
    save_config, set_active_model,
};
use crate::fs_tools::{
    grep_output, grep_recursive, list_files_output, list_files_recursive, read_text_file,
    try_rg_files, try_rg_grep,
};
use crate::llm::{
    ChatMessage, NativeFunctionCall, build_openai_messages, call_llm_with_history,
    call_llm_with_history_stream, call_llm_with_messages_native_tools,
};
use crate::prompt_store::list_prompt_names;
use crate::util::{
    WorkingStatus, ask, ask_or_eof, color_blue, color_cyan, color_dim, color_green, color_red,
    color_yellow, prefix_chars, print_startup_banner, tagged_prompt, truncate_preview,
    truncate_with_suffix,
};
const MAX_AUTO_TOOL_STEPS: usize = 3;
const MAX_COMMANDS_PER_RESPONSE: usize = 8;
const MAX_FAILED_COMMANDS_PER_RESPONSE: usize = 2;
const MAX_INVALID_FORMAT_RETRIES: usize = 2;

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
    print_startup_banner(&active_session, &cfg.model, exec_mode.as_str());
    let mut history = load_session_or_default(&active_session)?;
    loop {
        let Some(input) = ask_or_eof(&color_green("you> "))? else {
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
            ).await?;
            save_session(&active_session, &history)?;
            print_changed_files_delta(&changed_before)?;
            continue;
        }

        if handle_natural_language_tool_command(input.trim(), &mut cfg, &mut history).await? {
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
        });

        maybe_compact_history(&mut history, &cfg);
        if use_agent {
            run_agent_turn(&mut cfg, &mut history, "chat").await?;
        } else {
            run_chat_turn(&mut cfg, &mut history, "chat-lite").await?;
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
            out.push_str(&format!("- {}: {}\n", name, preview));
        }
        println!("{out}");
        push_tool_result(history, input, "prompt.list", &out);
        return Ok(true);
    }

    if let Some(name) = parse_prompt_use(input, &lower) {
        if !list_prompt_names().unwrap_or_default().iter().any(|p| p == &name) {
            println!("Prompt not found: {name}");
            return Ok(true);
        }
        cfg.active_prompt = name.clone();
        save_config(cfg)?;
        let out = format!("Active prompt switched to '{}'.", name);
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
        push_tool_result(history, input, "model.list", &format!("current={}", cfg.model));
        return Ok(true);
    }

    if let Some(name) = parse_model_use(input, &lower) {
        ensure_model_catalog(cfg);
        if !cfg.model_catalog.iter().any(|m| m == &name) {
            println!("Model not found in catalog: {}", name);
            return Ok(true);
        }
        set_active_model(cfg, &name);
        save_config(cfg)?;
        let out = format!("Active model switched to '{}'.", name);
        println!("{out}");
        push_tool_result(history, input, "model.use", &out);
        return Ok(true);
    }

    if let Some(path) = extract_existing_file_path(input) {
        if !is_read_request(input, &lower) && !is_list_request(input, &lower) && !is_grep_request(input, &lower) {
            submit_file_to_model(cfg, history, input, &path).await?;
            return Ok(true);
        }
    }

    if is_read_request(input, &lower) {
        if let Some(path) = extract_path(input) {
            if has_followup_analysis_intent(input, &lower) {
                submit_file_to_model(cfg, history, input, &path).await?;
            } else {
                let content = read_text_file(Path::new(&path))?;
                push_tool_result(history, input, "fs.read", &clip_output(&content, 8000));
                println!("Read {} (content hidden). Ask a follow-up question to analyze it.", path);
            }
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

async fn submit_file_to_model(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    user_request: &str,
    path: &str,
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
    });
    maybe_compact_history(history, cfg);
    let system = build_system_prompt(cfg, "review");
    run_agent_turn_with_system(cfg, history, &system).await
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
    });
    history.push(ChatMessage {
        role: "assistant".to_string(),
        content: format!("tool[{tool}] output:\n{output}"),
    });
}

fn clip_output(text: &str, max_len: usize) -> String {
    truncate_with_suffix(text, max_len, "...\n[truncated]")
}

fn maybe_compact_history(history: &mut Vec<ChatMessage>, cfg: &Config) {
    let max_messages = cfg.history_max_messages.max(4);
    let max_chars = cfg.history_max_chars.max(2000);
    let total_chars = history.iter().map(|m| m.content.chars().count()).sum::<usize>();
    if history.len() <= max_messages && total_chars <= max_chars {
        return;
    }
    if history.len() < 8 {
        return;
    }

    let tail_keep = (max_messages / 2).max(6).min(history.len().saturating_sub(1));
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
        let role = if m.role == "user" { "user" } else { "assistant" };
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
        || lower.contains("search ") || lower.contains("search for ") || lower.contains("find ") || lower.contains("find in ")
        || input.contains("\u{641c}\u{7d22}")
        || input.contains("\u{67e5}\u{627e}")
        || input.contains("\u{68c0}\u{7d22}")
}

fn is_prompt_list_request(input: &str, lower: &str) -> bool {
    lower.contains("list prompt") || lower.contains("show prompts") || lower.contains("list presets") || lower.contains("show preset prompts")
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

async fn handle_chat_slash_command(
    input: &str,
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    active_session: &mut String,
    exec_mode: &mut ChatExecutionMode,
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
            println!("{}", color_dim("─────────────────────────────────────────────"));
            c("/help",                       "show this message");
            c("/exit",                       "quit");
            c("/new [name]",                 "start a new session");
            c("/clear",                      "clear current session history");
            c("/session list",               "list saved sessions");
            c("/session use <name>",         "switch session");
            c("/session rm <name>",          "delete session");
            c("/mode show",                  "show current execution mode");
            c("/mode chat|agent-auto|agent-force", "switch execution mode");
            c("/read <file> [question]",     "read a file into context");
            c("/askfile <file> <question>",  "ask about a file");
            c("/list [path]",                "list files");
            c("/grep <pattern> [path]",      "search files");
            c("/prompt show",                "show active prompt");
            c("/prompt list",                "list prompts");
            c("/prompt use <name>",          "switch prompt");
            c("/model list",                 "list available models");
            c("/model use <name>",           "switch model");
            println!("{}", color_dim("─────────────────────────────────────────────"));
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
        "/read" => {
            let Some(file) = parts.next() else {
                println!("Usage: /read <file>");
                return Ok(());
            };
            let question = parts.collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                let content = read_text_file(Path::new(file))?;
                push_tool_result(history, input, "fs.read", &clip_output(&content, 8000));
                println!("Read {} (content hidden). Ask a follow-up question to analyze it.", file);
            } else {
                submit_file_to_model(cfg, history, &question, file).await?;
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
            submit_file_to_model(cfg, history, &question, file).await?;
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
                            println!("- {}: {}", name, truncate_preview(&current_prompt_text(cfg), 90));
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
                    if !list_prompt_names().unwrap_or_default().iter().any(|p| p == name) {
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

        let exec = execute_tool_call_by_name(cfg, &call);
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
        let exec = execute_tool_call_by_name(cfg, &call);

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
            if !cfg.auto_exec_trusted.iter().any(|x| x.eq_ignore_ascii_case(&prefix)) {
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
    let raw = tool_arg_string(call, &["path", "file"]).ok_or_else(|| anyhow::anyhow!("fs.read_file requires args.path"))?;
    let path = resolve_native_path(&raw)?;
    let text = read_text_file(&path)?;
    Ok(format!("Read: {}\n{}", path.display(), clip_output(&text, 12000)))
}

fn execute_native_fs_create(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"]).ok_or_else(|| anyhow::anyhow!("fs.create_file requires args.path"))?;
    let content = tool_arg_string(call, &["content"]).ok_or_else(|| anyhow::anyhow!("fs.create_file requires args.content"))?;
    let overwrite = tool_arg_bool(call, &["overwrite"]).unwrap_or(true);
    let path = resolve_native_path(&raw)?;
    if path.exists() && !overwrite {
        bail!("target already exists and overwrite=false: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Failed to create parent dir {}", parent.display()))?;
    }
    fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("Created file: {}", path.display()))
}

fn execute_native_fs_edit(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"]).ok_or_else(|| anyhow::anyhow!("fs.edit_file requires args.path"))?;
    let old_str = tool_arg_string(call, &["old_str", "old"]).unwrap_or_default();
    let new_str = tool_arg_string(call, &["new_str", "new"]).ok_or_else(|| anyhow::anyhow!("fs.edit_file requires args.new_str"))?;
    let replace_all = tool_arg_bool(call, &["replace_all"]).unwrap_or(false);
    let path = resolve_native_path(&raw)?;
    let mut text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;

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
    Ok(format!("List: {}\n{}", path.display(), clip_output(&out, 12000)))
}

fn execute_native_fs_grep(call: &ToolCall) -> Result<String> {
    let pattern = tool_arg_string(call, &["pattern", "query"]).ok_or_else(|| anyhow::anyhow!("fs.grep requires args.pattern"))?;
    let raw = tool_arg_string(call, &["path"]).unwrap_or_else(|| ".".to_string());
    let path = resolve_native_path(&raw)?;
    let out = grep_output(&path, &pattern)?;
    if out.trim().is_empty() {
        return Ok(format!("Grep: {} in {}\nNo matches found.", pattern, path.display()));
    }
    Ok(format!("Grep: {} in {}\n{}", pattern, path.display(), clip_output(&out, 12000)))
}


fn execute_native_fs_apply_patch(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file"]).ok_or_else(|| anyhow::anyhow!("fs.apply_patch requires args.path"))?;
    let path = resolve_native_path(&raw)?;
    let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;

    let edits = tool_arg_array(call, &["edits", "patches"]).ok_or_else(|| anyhow::anyhow!("fs.apply_patch requires args.edits[]"))?;
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
        let replace_all = obj.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
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
    let from_raw = tool_arg_string(call, &["from", "src", "source"]).ok_or_else(|| anyhow::anyhow!("fs.move requires args.from"))?;
    let to_raw = tool_arg_string(call, &["to", "dst", "target"]).ok_or_else(|| anyhow::anyhow!("fs.move requires args.to"))?;
    let from = resolve_native_path(&from_raw)?;
    let to = resolve_native_path(&to_raw)?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Failed to create parent dir {}", parent.display()))?;
    }
    fs::rename(&from, &to).with_context(|| format!("Failed to move {} -> {}", from.display(), to.display()))?;
    Ok(format!("Moved: {} -> {}", from.display(), to.display()))
}

fn execute_native_fs_delete(call: &ToolCall) -> Result<String> {
    let raw = tool_arg_string(call, &["path", "file", "target"]).ok_or_else(|| anyhow::anyhow!("fs.delete requires args.path"))?;
    let recursive = tool_arg_bool(call, &["recursive", "r"]).unwrap_or(false);
    let p = resolve_native_path(&raw)?;
    if !p.exists() {
        return Ok(format!("Skip delete; not found: {}", p.display()));
    }
    if p.is_dir() {
        if recursive {
            fs::remove_dir_all(&p).with_context(|| format!("Failed to remove dir {}", p.display()))?;
        } else {
            fs::remove_dir(&p).with_context(|| format!("Failed to remove dir {} (set recursive=true)", p.display()))?;
        }
    } else {
        fs::remove_file(&p).with_context(|| format!("Failed to remove file {}", p.display()))?;
    }
    Ok(format!("Deleted: {}", p.display()))
}

fn execute_structured_run_command(cfg: &mut Config, call: &ToolCall) -> Result<String> {
    let cmd = tool_arg_string(call, &["command", "cmd"]).or_else(|| if call.command.trim().is_empty() { None } else { Some(call.command.clone()) }).ok_or_else(|| anyhow::anyhow!("run_command requires args.command"))?;
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
        base.canonicalize().with_context(|| format!("Failed to resolve path {}", base.display()))?
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

    let cwd_norm = cwd.canonicalize().with_context(|| format!("Failed to resolve cwd {}", cwd.display()))?;
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

    if (lower.contains("base64") || lower.contains("frombase64string") || lower.contains("[convert]::frombase64string"))
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

async fn run_agent_turn(cfg: &mut Config, history: &mut Vec<ChatMessage>, mode: &str) -> Result<()> {
    let system = build_system_prompt(cfg, mode);
    run_agent_turn_with_system(cfg, history, &system).await
}

async fn run_agent_turn_with_system(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
) -> Result<()> {
    match run_agent_turn_with_system_native(cfg, history, system).await {
        Ok(()) => Ok(()),
        Err(err) => {
            println!(
                "\nassistant> Native function-calling unavailable, fallback to JSON tool_calls parser: {}\n",
                truncate_with_suffix(&err.to_string(), 220, " ...")
            );
            run_agent_turn_with_system_legacy(cfg, history, system).await
        }
    }
}

async fn run_agent_turn_with_system_native(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
) -> Result<()> {
    let tools = native_tool_schemas();
    let mut messages = build_openai_messages(system, history);
    let mut steps = 0usize;
    let mut unsafe_retries = 0usize;
    loop {
        compact_native_messages(&mut messages, cfg.history_max_chars.max(2000));
        println!("{}", color_dim(&format!("(phase: reasoning step {})", steps + 1)));
        print!("{}", color_blue(&format!("assistant[{}]({})> ", cfg.active_prompt, cfg.model)));
        let resp = call_llm_with_messages_native_tools(cfg, &messages, &tools).await?;
        let answer = resp.content.trim().to_string();
        if !answer.is_empty() {
            println!("{}", answer);
        }
        println!("\n");
        messages.push(resp.assistant_message);

        if resp.tool_calls.is_empty() {
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: answer,
            });
            return Ok(());
        }

        let (exec_result, tool_msgs) = execute_native_function_calls(cfg, &resp.tool_calls)?;
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
                "content":"Your last tool calls were unsupported or unsafe. Use supported functions only, and only when needed. Otherwise provide direct final analysis/result."
            }));
            continue;
        }

        println!("assistant> Detected tool calls, but all were skipped or unsafe.\n");
        return Ok(());
    }
}

async fn run_agent_turn_with_system_legacy(
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
) -> Result<()> {
    let mut steps = 0usize;
    let mut unsafe_retries = 0usize;
    let mut invalid_format_retries = 0usize;
    loop {
        maybe_compact_history(history, cfg);
        println!("{}", color_dim(&format!("(phase: reasoning step {})", steps + 1)));
        print!("{}", color_blue(&format!("assistant[{}]({})> ", cfg.active_prompt, cfg.model)));
        let answer = match call_llm_with_history_stream(cfg, system, history).await {
            Ok(v) => v,
            Err(err) => {
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
        let exec_result = maybe_execute_assistant_commands(cfg, &answer)?;
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer.clone(),
        });

        if !exec_result.had_blocks {
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
                content: "Your last response had invalid tool_calls format. Use strict JSON like ```json {\"tool_calls\":[{\"tool\":\"fs.read_file\",\"args\":{\"path\":\"src/main.rs\"}}]} ``` or ```json {\"tool_calls\":[{\"tool\":\"shell\",\"command\":\"rg --files\"}]} ```.".to_string(),
            });
            continue;
        }

        if exec_result.skipped_any && unsafe_retries < 1 {
            unsafe_retries += 1;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: "Your last response used unsupported execution format or unsafe commands. Use JSON tool_calls only, and only when needed. Otherwise provide direct final analysis/result.".to_string(),
            });
            continue;
        }

        println!("assistant> Detected tool calls, but skipped because commands are unsafe or unsupported.\n");
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
        || t.contains("```json")
        || t.contains("``json")
        || t.contains("<|tool_call_argument_begin|>")
}
fn extract_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut out = Vec::new();
    collect_tool_calls_from_fence(text, "```json", "```", false, &mut out);
    collect_tool_calls_from_inline_json(text, &mut out);
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

async fn run_chat_turn(cfg: &mut Config, history: &mut Vec<ChatMessage>, mode: &str) -> Result<()> {
    let system = build_system_prompt(cfg, mode);
    maybe_compact_history(history, cfg);
    println!("{}", color_dim("(phase: response)"));
    print!("{}", color_blue(&format!("assistant[{}]({})> ", cfg.active_prompt, cfg.model)));
    let answer = match call_llm_with_history_stream(cfg, &system, history).await {
        Ok(v) => v,
        Err(err) => {
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
    history.push(ChatMessage {
        role: "assistant".to_string(),
        content: answer,
    });
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
    let mut router_history: Vec<ChatMessage> = history
        .iter()
        .rev()
        .take(4)
        .rev()
        .cloned()
        .collect();
    router_history.push(ChatMessage {
        role: "user".to_string(),
        content: format!(
            "Route this request for terminal assistant mode.\n\
             Request: {input}\n\
             Output JSON only: {{\"mode\":\"agent\"|\"chat\",\"reason\":\"short\"}}"
        ),
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
    ]
    .iter()
    .any(|k| input.contains(k));
    en_hit || zh_hit
}

fn print_execution_and_verification(exec_result: &ExecResult) -> Result<(String, String)> {
    println!("{}", color_dim("(phase: tool execution)"));
    let tool_calls = exec_result.display_text.matches("tool[").count();
    if exec_result.had_failures {
        println!(
            "{} tool calls executed with failures (details hidden)",
            tool_calls
        );
    } else {
        println!("{} tool calls executed (details hidden)", tool_calls);
    }
    println!("{}", color_dim("(phase: verification)"));
    let verification = run_auto_verification()?;
    if !verification.trim().is_empty() {
        println!("{} {}", color_dim("verify>"), verification);
    }
    let recovery_hint = if exec_result.had_failures {
        "\nSome commands failed. Prefer narrower retries: check file/path existence first, then rerun minimal commands.".to_string()
    } else {
        String::new()
    };
    Ok((verification, recovery_hint))
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
                    if k == "tool" || k == "type" || k == "command" || k == "cmd" || k == "tool_calls" {
                        continue;
                    }
                    m.insert(k.clone(), v.clone());
                }
                Value::Object(m)
            };

            let has_args = matches!(&args, Value::Object(m) if !m.is_empty());
            if !tool.trim().is_empty() && (!command.trim().is_empty() || has_args) {
                out.push(ToolCall { tool, command, args });
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
    matches_list(&cfg.auto_exec_trusted, cmd)
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
    let pattern = extract_quoted(cmd).unwrap_or_else(|| "TODO".to_string());
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
    Ok(config_dir()?.join("sessions").join(format!("{session}.json")))
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
    if s.is_empty() { "session".to_string() } else { s }
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
    });

    maybe_compact_history(&mut history, &cfg);
    run_agent_turn(&mut cfg, &mut history, "chat").await?;
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

fn list_saved_sessions() -> Result<Vec<String>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in
        fs::read_dir(&dir).with_context(|| format!("Failed to read session dir {}", dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
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
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .output();
    let Ok(out) = out else {
        return Ok(Vec::new());
    };
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let text = decode_command_output(&out.stdout);
    let mut files = Vec::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        if !path.is_empty() {
            files.push(path.to_string());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
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








