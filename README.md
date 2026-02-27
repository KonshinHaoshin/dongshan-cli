# dongshan cli

`dongshan` is a Rust-based AI coding CLI tool inspired by Claude CLI / Codex style workflows.

## Features

- Select API provider presets (`openai`, `deepseek`, `openrouter`)
- Interactive onboarding (`dongshan onboard`) for provider/model/api key/prompt
- Customize API endpoint/model/env key
- Save API key in local config (or use env var)
- Manage multiple local prompt profiles and switch active prompt
- Support prompt template variables (e.g. `{{tone}}`)
- File tools like Codex-style basics: read/list/grep
- `rg` (ripgrep) priority for fast grep/list, with internal fallback
- Multi-turn chat mode with local session persistence
- Local NSFW filtering can be disabled (`allow_nsfw = true`)
- Review code files with AI
- Edit code files with AI instructions (dry-run or `--apply`)
- Global install support (`cargo install --path .`)

## Build

```powershell
cargo build --release
```

## Global Install (PowerShell)

From this project root:

```powershell
cargo install --path .
```

Then you can run:

```powershell
dongshan --help
```

If command is not found, add Cargo bin to PATH:

```powershell
$cargoBin = "$HOME\\.cargo\\bin"
[Environment]::SetEnvironmentVariable("Path", $env:Path + ";" + $cargoBin, "User")
```

Open a new terminal and run `dongshan`.

## Quick Start

1. Initialize config:

```powershell
dongshan config init
```

2. Run onboarding:

```powershell
dongshan onboard
```

3. Or set by commands:

```powershell
$env:OPENAI_API_KEY = "your_api_key_here"
dongshan config use openai
dongshan config set --model gpt-4o-mini
dongshan config set --allow-nsfw true
```

4. File tools:

```powershell
dongshan fs read .\src\main.rs
dongshan fs list .
dongshan fs grep "run_edit" .
```

5. Prompt profiles:

```powershell
dongshan prompt list
dongshan prompt save reviewer "Focus on bugs and missing tests"
dongshan prompt use reviewer
dongshan prompt show
dongshan prompt var-set tone strict
dongshan prompt var-list
```

6. Review a file:

```powershell
dongshan review .\src\main.rs
```

7. Edit a file (preview):

```powershell
dongshan edit .\src\main.rs --instruction "refactor duplicated logic"
```

8. Apply edits:

```powershell
dongshan edit .\src\main.rs --instruction "refactor duplicated logic" --apply
```

9. Chat mode:

```powershell
dongshan chat --session work
```

Type `/exit` to quit chat. Session history is saved to:

- `~/.dongshan/sessions/work.json`

If you omit `--session` (default mode), chat memory is automatically isolated by current workspace path.

Chat slash commands:

- `/help`
- `/new [name]` (start a new session; no name = auto-generated new session in current workspace)
- `/read <file>`
- `/list [path]`
- `/grep <pattern> [path]`
- `/prompt show`
- `/prompt list`
- `/prompt use <name>`
- `/clear`

Natural language routing in chat (no slash required):

- "帮我读取 src/main.rs"
- "列出当前目录文件"
- "搜索 \"run_chat\" �?src"
- "查看当前配置"
- "列出prompt"
- "切换prompt reviewer"

Assistant command auto-exec in chat:

- If the model returns fenced shell commands (e.g. ```bash ... ```), dongshan will auto-run safe read-only commands (`ls`, `dir`, `cat`, `rg`, `grep`, `git status`...) and print output.
- Unknown/unsafe commands are skipped.
- Agent loop: after executing safe commands, dongshan feeds outputs back to the model and continues automatically (up to 3 steps) until it returns a final answer.

## API Key Priority

`dongshan` resolves API key in this order:

1. Environment variable from `api_key_env`
2. `api_key` field in `~/.dongshan/config.toml`

## NSFW Behavior

- `dongshan` itself can run with `allow_nsfw = true` and will not add extra NSFW filtering in local prompt flow.
- Provider APIs (OpenAI/DeepSeek/OpenRouter etc.) may still enforce their own policy checks server-side.

## Config

Config file location:

- `~/.dongshan/config.toml`

Example:

```toml
base_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
api_key = ""
active_prompt = "default"
allow_nsfw = true

[prompts]
default = "You are a pragmatic senior software engineer."
review = "Focus on correctness, regressions, risks, and missing tests."
edit = "Keep changes minimal and preserve behavior unless requested."

[prompt_vars]
tone = "strict"
```

You can still set API key via env:

```powershell
$env:OPENAI_API_KEY = "your_api_key_here"
```

