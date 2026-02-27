# dongshan cli（中文文档）

`dongshan` 是一个使用 Rust 编写的 AI 终端工具，目标体验接近 Claude CLI / Codex / Cursor。

## 功能

- Provider 预设（`openai` / `deepseek` / `openrouter` / `xai` / `nvidia`）
- `dongshan onboard` 交互初始化
- onboard 按 provider 提供模型候选（编号选择 + 自定义输入）
- onboard 会尝试在线拉取模型列表（失败自动回退内置候选）
- Prompt 保存、切换、变量模板
- 本地 Web 控制台（`dongshan web`）管理配置/模型/Prompt/执行策略
- `chat` 自然语言工具路由
- `/new`、`/read`、`/grep` 等斜杠命令
- Agent 循环：根据策略自动执行命令块并回喂模型
- 仅对 `bash/sh/powershell/pwsh/cmd` 代码块做命令执行解析
- 对“非默认信任命令”执行前询问，可一键设为默认信任前缀
- 启动自动检查更新（GitHub）
- 按模型独立 profile（`base_url` / `api_key_env` / `api_key`），支持自定义 OpenAI 兼容网关
- `dongshan doctor` 当前模型健康检查
- 结构化 JSON tool-call 自动执行（不再自动执行传统 shell 代码块）
- 聊天历史自动压缩（按消息数和字符预算）

## 构建

```powershell
cargo build --release
```

## 全局安装

```powershell
cargo install --path .
dongshan --help
```

## Windows 安装包（Setup EXE）

给普通用户推荐直接用 Release 安装包：

- `dongshan-setup-windows-x86_64.exe`（推荐）
- `dongshan-windows-x86_64.zip`（便携版）

安装后重新打开 PowerShell，执行：

```powershell
dongshan --help
```

## Release 自动打包

仓库已内置 Windows 自动打包发布：

- Workflow：`.github/workflows/release.yml`
- Inno 安装脚本：`packaging/windows/dongshan.iss`
- 本地打包脚本：`scripts/build-installer.ps1`

发布新版本（打 tag）：

```powershell
git tag v0.1.4
git push origin v0.1.4
```

GitHub Actions 会自动上传：

- `dongshan-setup-windows-x86_64.exe`
- `dongshan-windows-x86_64.zip`
- `SHA256SUMS.txt`

## 快速开始

```powershell
dongshan config init
dongshan onboard
dongshan config set --auto-check-update true
dongshan chat
dongshan web --port 3721
```

## Web 控制台

启动：

```powershell
dongshan web --port 3721
```

浏览器打开：

- `http://127.0.0.1:3721`

使用方法：

```powershell
# 1) 启动 Web 服务
dongshan web --port 3721

# 2) 保持这个终端窗口运行，然后在浏览器访问
http://127.0.0.1:3721
```

可管理内容：

- Provider/API 配置（`base_url`、`api_key_env`、`api_key`、`allow_nsfw`）
- 模型目录（新增/切换/删除）
- 按模型独立配置连接信息：每个模型各自维护 `base_url` / `api_key_env` / `api_key`
- Prompt 文件（保存/切换/删除）
- 自动执行策略（`safe/all/custom`、allow/deny/trusted、确认开关）

说明：

- `onboard` 现在只保存你最终选择的模型，不会把候选列表全部加入模型目录。

## Chat

斜杠命令：

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

自然语言示例：

- `帮我读取 src/main.rs`
- `列出当前目录文件`
- `搜索 "run_chat" 在 src`
- `查看当前配置`
- `切换prompt reviewer`
- `列出模型`
- `切换模型 grok-code-fast-1`

会话文件：

- `~/.dongshan/sessions/*.json`
- 默认 `dongshan chat` 会按当前路径隔离记忆。

## Prompt 多模板编写与切换

可创建多个 prompt 并随时切换：

```powershell
dongshan prompt list
dongshan prompt save reviewer "你是严格代码审查员，优先找 bug 和缺失测试。"
dongshan prompt save architect "你是架构顾问，优先关注模块边界和长期可维护性。"
dongshan prompt use reviewer
dongshan prompt show
```

设置 Prompt 变量模板：

```powershell
dongshan prompt var-set tone strict
dongshan prompt var-list
```

在 chat 内切换：

```text
/prompt list
/prompt use architect
```

也支持自然语言：

```text
切换prompt reviewer
```

Prompt 存储位置：

- `~/.dongshan/prompts/*.json`
- 每个 prompt 一个 JSON 文件，例如：

```json
{
  "name": "reviewer",
  "content": "你是严格代码审查员，优先找 bug 和缺失测试。"
}
```

## 命令自动执行策略

你可以自己决定哪些命令安全：

- `safe`：默认内置安全白名单
- `all`：全部放行（由 LLM 自行选择要执行的命令，谨慎）
- `custom`：只允许你配置的 allow 列表；deny 永远优先拦截
- `auto_confirm_exec=true`：对非信任命令执行前询问
- `auto_exec_trusted`：默认信任命令前缀（如 `rg`、`grep`、`git status`）

示例：

```powershell
# 全部放行
dongshan config set --auto-exec-mode all

# 恢复默认安全策略
dongshan config set --auto-exec-mode safe

# 自定义策略
dongshan config set --auto-exec-mode custom --auto-exec-allow "rg,ls,git status" --auto-exec-deny "rm,del"

# 开启逐条确认执行（默认 true）
dongshan config set --auto-confirm-exec true

# 配置默认信任前缀
dongshan config set --auto-exec-trusted "rg,grep,git status"
```

## 自动更新检查

- 更新源：`https://github.com/KonshinHaoshin/dongshan-cli`
- 默认每 24 小时检查一次
- 检测到新版本会提示 Release 页面和对应安装包资产名称。

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
auto_confirm_exec = true
auto_exec_trusted = ["rg", "grep", "git status"]
```

## 模型命令

```powershell
dongshan models list
dongshan models add grok-code-fast-1
dongshan models use grok-code-fast-1
dongshan models remove old-model-name
dongshan models show
dongshan models show grok-code-fast-1
dongshan models set-profile grok-code-fast-1 --base-url "https://api.x.ai/v1/chat/completions" --api-key-env "XAI_API_KEY"
```

自定义模型（自定义 API 地址和 Key 环境变量）：

```powershell
dongshan models add my-openai-compatible `
  --base-url "https://your-gateway.example.com/v1/chat/completions" `
  --api-key-env "MY_GATEWAY_KEY"
dongshan models use my-openai-compatible
```

## Doctor 健康检查

```powershell
dongshan doctor
```

检查内容：

- 当前模型 profile 是否存在
- `base_url` 是否有效
- API key 是否可解析
- `/models` 是否可达（不支持时给 warning）
- 实际 chat completion 连通性

## Chat 执行协议

- 自动执行只解析 JSON tool-call：

```json
{"tool_calls":[{"tool":"shell","command":"rg --files"}]}
```

- 传统 `bash/powershell` 代码块不会再被自动执行。

## 会话压缩参数

```powershell
dongshan config set --history-max-messages 24 --history-max-chars 50000
```
