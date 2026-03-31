use anyhow::{Result, bail};
use serde_json::json;

use crate::query::input::{ParsedInput, SlashCommand, parse_input};
use crate::query::turn_runner::{ExecutionMode, run_turn};
use crate::services::{diagnostics, settings, skills};
use crate::state::runtime::RuntimeState;
use crate::tools::registry::ToolRegistry;

pub struct EngineReply {
    pub exit: bool,
}

pub struct QueryEngine {
    pub runtime: RuntimeState,
    registry: ToolRegistry,
}

impl QueryEngine {
    pub fn new(session: String, mode: ExecutionMode) -> Self {
        Self {
            runtime: RuntimeState::new(session, mode.as_str()),
            registry: ToolRegistry::claude_style(),
        }
    }

    pub async fn handle_input(
        &mut self,
        input: &str,
        mode: ExecutionMode,
    ) -> Result<EngineReply> {
        match parse_input(input) {
            ParsedInput::Exit => Ok(EngineReply {
                exit: true,
            }),
            ParsedInput::Help => {
                self.runtime.set_output(help_text());
                self.runtime.set_status("ready");
                Ok(EngineReply {
                    exit: false,
                })
            }
            ParsedInput::Slash(command) => {
                let output = self.handle_slash(command)?;
                self.runtime.set_output(output);
                self.runtime.set_status("ready");
                Ok(EngineReply {
                    exit: false,
                })
            }
            ParsedInput::Prompt(prompt) => {
                let mut cfg = settings::load()?;
                self.runtime.set_status(format!("running {}", mode.as_str()));
                let output = run_turn(&mut cfg, &self.runtime.session, &prompt, mode).await?;
                self.runtime.set_output(output);
                self.runtime.set_status("ready");
                Ok(EngineReply {
                    exit: false,
                })
            }
        }
    }

    fn handle_slash(&mut self, command: SlashCommand) -> Result<String> {
        match command {
            SlashCommand::ModelList => {
                self.registry.execute("model", &json!({"action":"list"}))
            }
            SlashCommand::ModelUse(name) => {
                if name.trim().is_empty() {
                    bail!("Usage: /model use <name>");
                }
                self.registry
                    .execute("model", &json!({"action":"use","name":name}))
            }
            SlashCommand::SkillsList => {
                self.registry.execute("skills", &json!({"action":"list"}))
            }
            SlashCommand::SkillsShow(name) => {
                let Some(skill) = skills::find(name.trim())? else {
                    bail!("Skill not found: {}", name.trim());
                };
                Ok(format!(
                    "Skill: {}\nDescription: {}\nTools: {}",
                    skill.manifest.name,
                    skill.manifest.description,
                    skill.manifest.allowed_tools.join(", ")
                ))
            }
            SlashCommand::SkillsUse(name) => {
                let Some(skill) = skills::find(name.trim())? else {
                    bail!("Skill not found: {}", name.trim());
                };
                skills::save_active_for_session(&self.runtime.session, Some(&skill.manifest.name))?;
                Ok(format!("Active skill set to {}", skill.manifest.name))
            }
            SlashCommand::SkillsClear => {
                skills::save_active_for_session(&self.runtime.session, None)?;
                Ok("Active skill cleared.".to_string())
            }
            SlashCommand::FilesRead(path) => self
                .registry
                .execute("fs_read_file", &json!({"path": path.trim()})),
            SlashCommand::FilesList(path) => self.registry.execute(
                "fs_list_files",
                &json!({"path": path.unwrap_or_else(|| ".".to_string())}),
            ),
            SlashCommand::FilesGrep { pattern, path } => self.registry.execute(
                "fs_grep",
                &json!({"pattern": pattern, "path": path.unwrap_or_else(|| ".".to_string())}),
            ),
            SlashCommand::ConfigShow => self.registry.execute("config", &json!({"action":"show"})),
            SlashCommand::Doctor => diagnostics::render_last_error(),
            SlashCommand::Diff => render_diff(),
            SlashCommand::Tasks => Ok(not_implemented("tasks")),
            SlashCommand::Permissions => Ok(not_implemented("permissions")),
            SlashCommand::Plan => Ok(not_implemented("plan")),
            SlashCommand::Unknown(raw) => Ok(format!("Unknown command: {}", raw)),
        }
    }
}

fn help_text() -> String {
    [
        "/model list|use <name>",
        "/skills list|show <name>|use <name>|clear",
        "/files read <path>",
        "/files list [path]",
        "/files grep <pattern> [path]",
        "/config",
        "/doctor",
        "/diff",
        "/tasks",
        "/permissions",
        "/plan",
        "/exit",
    ]
    .join("\n")
}

fn render_diff() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["status", "--short"])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn not_implemented(area: &str) -> String {
    format!(
        "`{}` is registered in the Claude-style command surface, but the subsystem is not implemented yet.",
        area
    )
}
