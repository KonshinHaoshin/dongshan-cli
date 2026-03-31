# AGENTS.md

## Project Overview

`dongshan` is a Rust-based AI coding CLI with:

- core CLI logic in `src/`
- subcommand handlers in `src/commands/`
- a local web console served from `web/`
- npm global-install wrapper files in `npm/`
- release packaging assets in `packaging/` and `scripts/`

This repository is primarily a Rust project. Treat the web and npm parts as support layers around the Rust binary, not as the main implementation surface.

## Working Rules

- Keep changes tightly scoped to the user's request.
- Prefer the smallest correct diff over cleanup or refactor work.
- Preserve existing behavior unless the user explicitly asks to change it.
- Do not rewrite adjacent modules just because they could be improved.
- Avoid formatting churn, renames, or file moves unless they are required.
- If a request touches CLI behavior, also consider whether docs, packaging, or the web console need a matching update.

## Repo Map

- `src/main.rs`: CLI entrypoint and top-level command dispatch
- `src/chat.rs`: main chat and agent loop behavior
- `src/cli.rs`: clap CLI definitions
- `src/config.rs`: config loading and persistence
- `src/llm.rs`: provider/model request logic
- `src/webui.rs`: local web server
- `src/commands/`: command-specific logic
- `web/`: static frontend assets for `dongshan web`
- `npm/`: npm package wrapper and install downloader
- `.github/workflows/ci.yml`: canonical CI verification steps
- `.github/workflows/release.yml`: release packaging flow

## Implementation Guidance

### Rust

- Follow the existing module structure instead of introducing new layers prematurely.
- Prefer explicit, readable control flow over clever abstractions.
- Reuse existing config, diagnostics, and utility helpers before adding new ones.
- Keep error handling consistent with the current `anyhow::Result` style.
- When adding CLI flags or subcommands, update the clap definitions and the dispatcher together.

### Web Console

- The web UI is plain static files in `web/` (`index.html`, `app.css`, `app.js`).
- Do not introduce a build system unless the user explicitly asks for one.
- Keep browser-side changes compatible with the current no-build setup.

### npm Wrapper / Packaging

- Treat `npm/` as a distribution wrapper around release binaries, not an alternative runtime.
- If package versioning, install behavior, or release asset names change, verify consistency across `package.json`, `npm/`, docs, and release workflow files.
- Be conservative with packaging changes because they affect install and upgrade paths.

## Verification

Changed code is not done until it is verified with the most relevant checks available.

Default verification order for this repo:

1. `cargo check`
2. `cargo clippy -- -D warnings`
3. `cargo test`

Use more targeted verification when appropriate:

- `cargo build --release` for packaging or release-related changes
- manual smoke checks for `dongshan web` when editing `web/` or `src/webui.rs`
- manual CLI invocation for changed subcommands when practical

If you cannot run a relevant check, say so explicitly. Do not claim something is fixed without verification evidence.

## Change Boundaries

- For command behavior changes, inspect both `src/cli.rs` and the affected handler in `src/commands/` or `src/main.rs`.
- For config-related changes, inspect persistence and user-facing docs/examples.
- For agent/chat behavior changes, inspect surrounding loop and verification behavior before patching.
- For release/install changes, inspect `.github/workflows/`, `scripts/`, `packaging/`, and `npm/` together.
- Do not edit generated artifacts or `target/` outputs unless the user explicitly asks.

## Documentation Expectations

- Update `README.md` when behavior visible to users changes.
- Update `README.zh-CN.md` too when the English README change affects usage, commands, install, packaging, or user-facing behavior.
- Keep docs aligned with the actual command names and current release asset names.

## Review Priorities

When reviewing or modifying code in this repo, prioritize:

1. correctness of command behavior
2. regressions in config and session persistence
3. unsafe command execution or permission-policy mistakes
4. install/release breakage across Rust, npm, and packaged binaries
5. missing verification for CLI-facing changes

## Output Expectations

In final responses:

- summarize what changed
- summarize what was verified
- call out anything not verified or still uncertain
- keep it concise and evidence-based
