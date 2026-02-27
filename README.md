# dongshan cli

`dongshan` is a Rust-based AI coding CLI tool inspired by Claude CLI / Codex workflows.

- 中文文档: [README.zh-CN.md](README.zh-CN.md)

## Features

- Provider presets (`openai`, `deepseek`, `openrouter`, `xai`, `nvidia`)
- `dongshan onboard` interactive setup
- Onboard model picker by provider (numbered choices + custom input)
- Onboard can fetch online model list (fallback to built-in options if unavailable)
- Prompt profiles + variables
- Chat with natural-language tool routing
- Slash commands in chat (`/new`, `/read`, `/grep`, etc.)
- Agent loop: auto-run commands based on your policy (`safe`/`all`/`custom`)
- Auto update check from GitHub on startup
- Real-time progress line in terminal (e.g. `(working model gpt-4o-mini 8s)`)

## Build

```powershell
cargo build --release
```

## Global Install

```powershell
cargo install --path .
dongshan --help
```

If command is not found, add Cargo bin to PATH:

```powershell
$cargoBin = "$HOME\\.cargo\\bin"
[Environment]::SetEnvironmentVariable("Path", $env:Path + ";" + $cargoBin, "User")
```

## Quick Start

```powershell
dongshan config init
dongshan onboard
dongshan config set --auto-check-update true
dongshan chat
```

## Chat

Slash commands:

- `/help`
- `/new [name]`
- `/read <file>`
- `/list [path]`
- `/grep <pattern> [path]`
- `/prompt show|list|use <name>`
- `/clear`
- `/exit`

Natural language examples:

- `帮我读取 src/main.rs`
- `列出当前目录文件`
- `搜索 "run_chat" 在 src`
- `查看当前配置`
- `切换prompt reviewer`

Session files:

- `~/.dongshan/sessions/*.json`
- Default `dongshan chat` session is isolated by current workspace path.

## Auto Exec Policy

You can choose how command blocks are executed in chat:

- `safe` (default): built-in safe read-only commands
- `all`: allow all commands (LLM decides what to run)
- `custom`: only commands in your allowlist; denylist always blocks

Examples:

```powershell
# 全部放行（谨慎）
dongshan config set --auto-exec-mode all

# 恢复内置安全策略
dongshan config set --auto-exec-mode safe

# 自定义：只允许 rg/ls/git status，且禁止 rm/del
dongshan config set --auto-exec-mode custom --auto-exec-allow "rg,ls,git status" --auto-exec-deny "rm,del"
```

## Auto Update Check

- Source repo: `https://github.com/KonshinHaoshin/dongshan-cli`
- Checks about every 24 hours by default.
- On newer version, it prints:

```powershell
cargo install --git https://github.com/KonshinHaoshin/dongshan-cli --force
```

Disable:

```powershell
dongshan config set --auto-check-update false
```

## Config

Config file:

- `~/.dongshan/config.toml`

Example:

```toml
base_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
api_key = ""
active_prompt = "default"
allow_nsfw = true
auto_check_update = true
auto_exec_mode = "safe"
auto_exec_allow = ["rg", "ls"]
auto_exec_deny = ["rm", "del"]

[prompts]
default = "You are a pragmatic senior software engineer."
review = "Focus on correctness, regressions, risks, and missing tests."
edit = "Keep changes minimal and preserve behavior unless requested."

[prompt_vars]
tone = "strict"
```

