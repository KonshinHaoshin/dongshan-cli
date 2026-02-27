# dongshan cli（中文文档）

`dongshan` 是一个使用 Rust 编写的 AI 终端工具，目标体验接近 Claude CLI / Codex / Cursor。

## 功能

- Provider 预设（`openai` / `deepseek` / `openrouter` / `xai` / `nvidia`）
- `dongshan onboard` 交互初始化
- onboard 按 provider 提供模型候选（编号选择 + 自定义输入）
- onboard 会尝试在线拉取模型列表（失败自动回退内置候选）
- Prompt 保存、切换、变量模板
- `chat` 自然语言工具路由
- `/new`、`/read`、`/grep` 等斜杠命令
- Agent 循环：根据策略自动执行命令块并回喂模型
- 启动自动检查更新（GitHub）

## 构建

```powershell
cargo build --release
```

## 全局安装

```powershell
cargo install --path .
dongshan --help
```

## 快速开始

```powershell
dongshan config init
dongshan onboard
dongshan config set --auto-check-update true
dongshan chat
```

## Chat

斜杠命令：

- `/help`
- `/new [name]`
- `/read <file>`
- `/list [path]`
- `/grep <pattern> [path]`
- `/prompt show|list|use <name>`
- `/clear`
- `/exit`

自然语言示例：

- `帮我读取 src/main.rs`
- `列出当前目录文件`
- `搜索 "run_chat" 在 src`
- `查看当前配置`
- `切换prompt reviewer`

会话文件：

- `~/.dongshan/sessions/*.json`
- 默认 `dongshan chat` 会按当前路径隔离记忆。

## 命令自动执行策略

你可以自己决定哪些命令安全：

- `safe`：默认内置安全白名单
- `all`：全部放行（由 LLM 自行选择要执行的命令，谨慎）
- `custom`：只允许你配置的 allow 列表；deny 永远优先拦截

示例：

```powershell
# 全部放行
dongshan config set --auto-exec-mode all

# 恢复默认安全策略
dongshan config set --auto-exec-mode safe

# 自定义策略
dongshan config set --auto-exec-mode custom --auto-exec-allow "rg,ls,git status" --auto-exec-deny "rm,del"
```

## 自动更新检查

- 更新源：`https://github.com/KonshinHaoshin/dongshan-cli`
- 默认每 24 小时检查一次
- 检测到新版本会提示：

```powershell
cargo install --git https://github.com/KonshinHaoshin/dongshan-cli --force
```

关闭自动检查：

```powershell
dongshan config set --auto-check-update false
```

## 配置文件

路径：`~/.dongshan/config.toml`

示例：

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
```
