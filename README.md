# dongshan cli

`dongshan` is a Rust-based AI coding CLI tool inspired by Claude CLI / Codex workflows.

- 中文文档: [README.zh-CN.md](README.zh-CN.md)

## Features

- Provider presets (`openai`, `deepseek`, `openrouter`, `xai`, `nvidia`)
- `dongshan onboard` interactive setup
- Onboard model picker by provider (numbered choices + custom input)
- Onboard can fetch online model list (fallback to built-in options if unavailable)
- Prompt profiles + variables
- Local web console for managing config/models/prompts/policy (`dongshan web`)
- Chat with natural-language tool routing
- Slash commands in chat (`/new`, `/read`, `/grep`, etc.)
- Agent loop: auto-run commands based on your policy (`safe`/`all`/`custom`)
- Only `bash/sh/powershell/pwsh/cmd` fenced blocks are considered for execution
- Ask before running non-trusted commands, with "always trust this prefix" option
- Auto update check from GitHub on startup
- Real-time progress line in terminal (e.g. `(working model gpt-4o-mini 8s)`)
- Per-model profile (`base_url` / `api_key_env` / `api_key`) for custom OpenAI-compatible endpoints
- `dongshan doctor` health check for current model profile
- Structured JSON tool-call execution (no legacy shell block auto-exec)
- Automatic chat history compaction (message and character budget)

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

## Windows Setup EXE

For normal users, use the installer from GitHub Releases:

- `dongshan-setup-windows-x86_64.exe` (recommended)
- `dongshan-windows-x86_64.zip` (portable)

After installation, open a new PowerShell and run:

```powershell
dongshan --help
```

Uninstall behavior:

- Start Menu shortcuts are removed by uninstaller
- Installer-added `{app}` PATH entry is automatically cleaned on uninstall

## Release Packaging

This repo includes automated packaging for Windows setup:

- Workflow: `.github/workflows/release.yml`
- Inno script: `packaging/windows/dongshan.iss`
- Local build script: `scripts/build-installer.ps1`

Publish a new release by pushing a tag:

```powershell
git tag v0.1.4
git push origin v0.1.4
```

GitHub Actions will build and upload:

- `dongshan-setup-windows-x86_64.exe`
- `dongshan-windows-x86_64.zip`
- `SHA256SUMS.txt`

## Quick Start

```powershell
dongshan config init
dongshan onboard
dongshan config set --auto-check-update true
dongshan chat
dongshan web --port 3721
```

## Web Console

Start local console:

```powershell
dongshan web --port 3721
```

Open in browser:

- `http://127.0.0.1:3721`

Usage:

```powershell
# 1) Start server
dongshan web --port 3721

# 2) Keep this terminal running, then open browser
http://127.0.0.1:3721
```

You can manage:

- Provider/API fields (`base_url`, `api_key_env`, `api_key`, `allow_nsfw`)
- Model catalog (add/use/remove)
- Per-model connection profile: each model has its own `base_url` / `api_key_env` / `api_key`
- Prompt files (save/use/delete)
- Auto exec policy (`safe/all/custom`, allow/deny/trusted, confirm flag)
- Frontend is split into static files: `web/index.html`, `web/app.css`, `web/app.js`
- UI uses Vue component architecture (loaded from CDN in `index.html`)

## Chat

Slash commands:

- `/help`
- `/new [name]`
- `/read <file>`
- `/list [path]`
- `/grep <pattern> [path]`
- `/prompt show|list|use <name>`
- `/model list`
- `/model use <name>`
- `/clear`
- `/exit`

Natural language examples:

- `帮我读取 src/main.rs`
- `列出当前目录文件`
- `搜索 "run_chat" 在 src`
- `查看当前配置`
- `切换prompt reviewer`
- `列出模型`
- `切换模型 grok-code-fast-1`

Session files:

- `~/.dongshan/sessions/*.json`
- Default `dongshan chat` session is isolated by current workspace path.

## Prompt Profiles

Create and switch multiple prompts:

```powershell
dongshan prompt list
dongshan prompt save reviewer "You are a strict code reviewer. Focus on bugs and missing tests."
dongshan prompt save architect "You are a software architect. Focus on modularity and long-term maintainability."
dongshan prompt use reviewer
dongshan prompt show
```

Set template variables:

```powershell
dongshan prompt var-set tone strict
dongshan prompt var-list
```

Switch prompt inside chat:

```text
/prompt list
/prompt use architect
```

Natural language in chat:

```text
切换prompt reviewer
```

Prompt storage location:

- `~/.dongshan/prompts/*.json`
- Each prompt is a separate JSON file, for example:

```json
{
  "name": "reviewer",
  "content": "You are a strict code reviewer. Focus on bugs and missing tests."
}
```

## Auto Exec Policy

You can choose how command blocks are executed in chat:

- `safe` (default): built-in safe read-only commands
- `all`: allow all commands (LLM decides what to run)
- `custom`: only commands in your allowlist; denylist always blocks
- `auto_confirm_exec=true`: ask before executing non-trusted commands
- `auto_exec_trusted`: trusted command prefixes (e.g. `rg`, `grep`, `git status`)

Examples:

```powershell
# 全部放行（谨慎）
dongshan config set --auto-exec-mode all

# 恢复内置安全策略
dongshan config set --auto-exec-mode safe

# 自定义：只允许 rg/ls/git status，且禁止 rm/del
dongshan config set --auto-exec-mode custom --auto-exec-allow "rg,ls,git status" --auto-exec-deny "rm,del"

# 开启逐条确认执行（默认 true）
dongshan config set --auto-confirm-exec true

# 设置默认信任前缀（这些命令不再询问）
dongshan config set --auto-exec-trusted "rg,grep,git status"
```

## Auto Update Check

- Source repo: `https://github.com/KonshinHaoshin/dongshan-cli`
- Checks about every 24 hours by default.
- On newer version, it prints release page and asset hints.

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
auto_confirm_exec = true
auto_exec_trusted = ["rg", "grep", "git status"]

[prompts]
default = "You are a pragmatic senior software engineer."
review = "Focus on correctness, regressions, risks, and missing tests."
edit = "Keep changes minimal and preserve behavior unless requested."

[prompt_vars]
tone = "strict"
```

## Models Command

```powershell
dongshan models list
dongshan models add grok-code-fast-1
dongshan models use grok-code-fast-1
dongshan models remove old-model-name
dongshan models show
dongshan models show grok-code-fast-1
dongshan models set-profile grok-code-fast-1 --base-url "https://api.x.ai/v1/chat/completions" --api-key-env "XAI_API_KEY"
```

Custom model with custom endpoint/key:

```powershell
dongshan models add my-openai-compatible \
  --base-url "https://your-gateway.example.com/v1/chat/completions" \
  --api-key-env "MY_GATEWAY_KEY"
dongshan models use my-openai-compatible
```

## Doctor

```powershell
dongshan doctor
```

Checks:

- Current model profile exists
- `base_url` validity
- API key resolution
- `/models` endpoint reachability (warning-only if unsupported)
- Real chat completion request health

## Chat Execution Protocol

- Auto execution only parses JSON tool-call blocks:

```json
{"tool_calls":[{"tool":"shell","command":"rg --files"}]}
```

- Legacy `bash/powershell` blocks are ignored for auto execution.

## Session Compaction

Tune chat memory budget:

```powershell
dongshan config set --history-max-messages 24 --history-max-chars 50000
```

