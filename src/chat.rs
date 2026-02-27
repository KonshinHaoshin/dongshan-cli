use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::chat_context::augment_user_input_with_workspace_context;
use crate::config::{
    AutoExecMode, Config, build_system_prompt, config_dir, current_prompt_text, save_config,
};
use crate::fs_tools::{
    grep_output, grep_recursive, list_files_output, list_files_recursive, read_text_file,
    try_rg_files, try_rg_grep,
};
use crate::llm::{ChatMessage, call_llm_with_history};
use crate::util::{WorkingStatus, ask, truncate_preview};
const MAX_AUTO_TOOL_STEPS: usize = 3;
const MAX_COMMANDS_PER_RESPONSE: usize = 8;
const MAX_FAILED_COMMANDS_PER_RESPONSE: usize = 2;

pub async fn run_chat(mut cfg: Config, session: &str) -> Result<()> {
    let mut active_session = resolve_session_name(session)?;
    println!("== dongshan chat ({active_session}) ==");
    println!("Type /help for slash commands. Type /exit to quit.");
    let mut history = load_session_or_default(&active_session)?;
    loop {
        let input = ask("you> ")?;
        if input.trim().eq_ignore_ascii_case("/exit") {
            break;
        }
        if input.trim().is_empty() {
            continue;
        }

        if input.trim_start().starts_with('/') {
            handle_chat_slash_command(
                input.trim(),
                &mut cfg,
                &mut history,
                &mut active_session,
            )?;
            save_session(&active_session, &history)?;
            continue;
        }

        if handle_natural_language_tool_command(input.trim(), &mut cfg, &mut history).await? {
            save_session(&active_session, &history)?;
            continue;
        }

        let augmented_input = augment_user_input_with_workspace_context(&input)?;
        history.push(ChatMessage {
            role: "user".to_string(),
            content: augmented_input,
        });

        run_agent_turn(&cfg, &mut history, "chat").await?;
        save_session(&active_session, &history)?;
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
        for (name, text) in &cfg.prompts {
            out.push_str(&format!("- {}: {}\n", name, truncate_preview(text, 90)));
        }
        println!("{out}");
        push_tool_result(history, input, "prompt.list", &out);
        return Ok(true);
    }

    if let Some(name) = parse_prompt_use(input, &lower) {
        if !cfg.prompts.contains_key(&name) {
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

    if let Some(path) = extract_existing_file_path(input) {
        if !is_read_request(input, &lower) && !is_list_request(input, &lower) && !is_grep_request(input, &lower) {
            let content = read_text_file(Path::new(&path))?;
            let ext = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("txt");
            let prompt = format!(
                "User asked to analyze or improve this file.\n\
                 Provide concrete improvements: correctness, maintainability, tests, and readability.\n\
                 Do not output shell commands; provide direct analysis.\n\n\
                 Original user request:\n{}\n\n\
                 File: {}\n```{}\n{}\n```",
                input, path, ext, content
            );
            history.push(ChatMessage {
                role: "user".to_string(),
                content: prompt,
            });
            let system = build_system_prompt(cfg, "review");
            run_agent_turn_with_system(cfg, history, &system).await?;
            return Ok(true);
        }
    }

    if is_read_request(input, &lower) {
        if let Some(path) = extract_path(input) {
            let content = read_text_file(Path::new(&path))?;
            println!("{content}");
            push_tool_result(history, input, "fs.read", &clip_output(&content, 8000));
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
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...\n[truncated]", &text[..max_len])
    }
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
        || lower.contains("find ")
        || input.contains("\u{641c}\u{7d22}")
        || input.contains("\u{67e5}\u{627e}")
        || input.contains("\u{68c0}\u{7d22}")
}

fn is_prompt_list_request(input: &str, lower: &str) -> bool {
    lower.contains("list prompt")
        || lower.contains("show prompts")
        || input.contains("\u{63d0}\u{793a}\u{8bcd}\u{5217}\u{8868}")
        || input.contains("\u{5217}\u{51fa}prompt")
}

fn is_config_show_request(input: &str, lower: &str) -> bool {
    lower.contains("show config")
        || lower.contains("current config")
        || input.contains("\u{67e5}\u{770b}\u{914d}\u{7f6e}")
        || input.contains("\u{5f53}\u{524d}\u{914d}\u{7f6e}")
}

fn parse_prompt_use(input: &str, lower: &str) -> Option<String> {
    if let Some(idx) = lower.find("use prompt ") {
        let name = input[idx + "use prompt ".len()..].trim();
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
    if let Some(p) = extract_after_keyword(input, "search ") {
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

fn handle_chat_slash_command(
    input: &str,
    cfg: &mut Config,
    history: &mut Vec<ChatMessage>,
    active_session: &mut String,
) -> Result<()> {
    let mut parts = input.split_whitespace();
    let Some(cmd) = parts.next() else {
        return Ok(());
    };

    match cmd {
        "/help" => {
            println!("Slash commands:");
            println!("/help");
            println!("/exit");
            println!("/new [name]");
            println!("/clear");
            println!("/read <file>");
            println!("/list [path]");
            println!("/grep <pattern> [path]");
            println!("/prompt show");
            println!("/prompt list");
            println!("/prompt use <name>");
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
        "/read" => {
            let Some(file) = parts.next() else {
                println!("Usage: /read <file>");
                return Ok(());
            };
            let content = read_text_file(Path::new(file))?;
            println!("{content}");
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
                    for (name, text) in &cfg.prompts {
                        println!("- {}: {}", name, truncate_preview(text, 90));
                    }
                }
                "use" => {
                    let Some(name) = parts.next() else {
                        println!("Usage: /prompt use <name>");
                        return Ok(());
                    };
                    if !cfg.prompts.contains_key(name) {
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
    display_text: String,
    history_text: String,
}

fn maybe_execute_assistant_commands(cfg: &Config, answer: &str) -> Result<ExecResult> {
    let blocks = extract_command_blocks(answer);
    if blocks.is_empty() {
        return Ok(ExecResult {
            executed_any: false,
            had_blocks: false,
            skipped_any: false,
            display_text: String::new(),
            history_text: String::new(),
        });
    }

    let mut display = String::new();
    let mut history = String::new();
    let mut executed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut seen_commands = 0usize;
    let mut failed_commands = 0usize;
    for block in blocks {
        for raw in block.lines() {
            let cmd = raw.trim();
            if cmd.is_empty() {
                continue;
            }
            if cmd.starts_with('#')
                || cmd.starts_with("//")
                || cmd.starts_with('-')
                || cmd.starts_with('*')
                || cmd.starts_with("```")
            {
                continue;
            }
            if seen_commands >= MAX_COMMANDS_PER_RESPONSE {
                let line = format!(
                    "Stopped auto exec after {} commands to avoid noisy output.\n",
                    MAX_COMMANDS_PER_RESPONSE
                );
                display.push_str(&line);
                history.push_str(&line);
                break;
            }
            seen_commands += 1;
            if let Some(reason) = precheck_command(cmd) {
                let line = format!("Skipped command: {} ({})\n", cmd, reason);
                display.push_str(&line);
                history.push_str(&line);
                skipped_count += 1;
                continue;
            }
            if !is_command_allowed(cfg, cmd) {
                let line = format!("Skipped unsafe command: {}\n", cmd);
                display.push_str(&line);
                history.push_str(&line);
                skipped_count += 1;
                continue;
            }
            let out = run_shell_command(cmd)?;
            display.push_str(&format!("$ {}\n{}\n", cmd, out));
            history.push_str(&format!("Executed: {}\nOutput:\n{}\n", cmd, out));
            executed_count += 1;
            if looks_like_command_failure(&out) {
                failed_commands += 1;
                if failed_commands >= MAX_FAILED_COMMANDS_PER_RESPONSE {
                    let line = format!(
                        "Stopped auto exec after {} failed commands.\n",
                        MAX_FAILED_COMMANDS_PER_RESPONSE
                    );
                    display.push_str(&line);
                    history.push_str(&line);
                    break;
                }
            }
        }
    }

    Ok(ExecResult {
        executed_any: executed_count > 0,
        had_blocks: true,
        skipped_any: skipped_count > 0,
        display_text: format!("\n{}", display),
        history_text: format!("tool[shell.auto_exec] output:\n{}", history),
    })
}

fn precheck_command(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return Some("empty command".to_string());
    }
    let first = tokens[0].to_ascii_lowercase();

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

async fn run_agent_turn(cfg: &Config, history: &mut Vec<ChatMessage>, mode: &str) -> Result<()> {
    let system = build_system_prompt(cfg, mode);
    run_agent_turn_with_system(cfg, history, &system).await
}

async fn run_agent_turn_with_system(
    cfg: &Config,
    history: &mut Vec<ChatMessage>,
    system: &str,
) -> Result<()> {
    let mut steps = 0usize;
    let mut unsafe_retries = 0usize;
    loop {
        let answer = call_llm_with_history(cfg, system, history).await?;
        let exec_result = maybe_execute_assistant_commands(cfg, &answer)?;
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer.clone(),
        });

        if !exec_result.had_blocks {
            println!("assistant> {}\n", answer);
            return Ok(());
        }

        if exec_result.executed_any {
            println!("assistant> {}", exec_result.display_text);
            history.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "{}\nContinue based on command outputs above. If complete, give final answer without command blocks.",
                    exec_result.history_text
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

        if exec_result.skipped_any && unsafe_retries < 1 {
            unsafe_retries += 1;
            history.push(ChatMessage {
                role: "user".to_string(),
                content: "Your last response contained unsafe or unsupported shell commands in this environment. Do not output command blocks. Give a direct final analysis/result in plain text based on available context.".to_string(),
            });
            continue;
        }

        println!("assistant> Detected command block(s), but skipped because commands are unsafe.\n");
        return Ok(());
    }
}

fn extract_command_blocks(text: &str) -> Vec<String> {
    let tags = ["```bash", "```sh", "```powershell", "```pwsh", "```cmd"];
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let slice = &text[i..];
        let mut found: Option<(usize, &str)> = None;
        for tag in &tags {
            if let Some(pos) = slice.find(tag) {
                match found {
                    Some((best, _)) if pos >= best => {}
                    _ => found = Some((pos, tag)),
                }
            }
        }
        let Some((pos, tag)) = found else {
            break;
        };
        let start = i + pos + tag.len();
        let after_tag = &text[start..];
        let line_skip = if let Some(nl) = after_tag.find('\n') {
            nl + 1
        } else {
            break;
        };
        let cmd_start = start + line_skip;
        let rest = &text[cmd_start..];
        let Some(end_rel) = rest.find("```") else {
            break;
        };
        let cmd = rest[..end_rel].trim().to_string();
        if !cmd.is_empty() {
            out.push(cmd);
        }
        i = cmd_start + end_rel + 3;
    }
    out
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
    let short = if cmd.len() > 48 {
        format!("exec {}...", &cmd[..48])
    } else {
        format!("exec {}", cmd)
    };
    let working = WorkingStatus::start(short);

    if let Some(v) = run_translated_safe_command(cmd)? {
        working.finish();
        return Ok(v);
    }

    let output = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .output()
            .with_context(|| format!("Failed to run command: {cmd}"))?
    } else {
        Command::new("sh")
            .args(["-lc", cmd])
            .output()
            .with_context(|| format!("Failed to run command: {cmd}"))?
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
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
    let txt = String::from_utf8_lossy(&out.stdout).to_string();
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
    let txt = String::from_utf8_lossy(&out.stdout).to_string();
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
    Ok(parsed)
}

fn save_session(session: &str, messages: &[ChatMessage]) -> Result<()> {
    let path = session_path(session)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create session dir {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(messages)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

