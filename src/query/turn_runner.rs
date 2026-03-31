use anyhow::Result;

use crate::chat::run_agent_task;
use crate::chat_context::augment_user_input_with_workspace_context;
use crate::config::{Config, build_system_prompt};
use crate::llm::{ChatMessage, call_llm_with_history};
use crate::query::history;
use crate::services::{settings, skills};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Chat,
    AgentAuto,
    AgentForce,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionMode::Chat => "chat",
            ExecutionMode::AgentAuto => "agent-auto",
            ExecutionMode::AgentForce => "agent-force",
        }
    }
}

pub async fn run_turn(
    cfg: &mut Config,
    session: &str,
    input: &str,
    mode: ExecutionMode,
) -> Result<String> {
    match mode {
        ExecutionMode::Chat => run_chat_turn(cfg, session, input).await,
        ExecutionMode::AgentAuto | ExecutionMode::AgentForce => {
            run_agent_task(cfg.clone(), session, input).await?;
            let messages = history::load(session)?;
            Ok(messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.content.clone())
                .unwrap_or_else(|| "Agent run completed.".to_string()))
        }
    }
}

async fn run_chat_turn(cfg: &mut Config, session: &str, input: &str) -> Result<String> {
    let mut messages = history::load(session)?;
    let _ = skills::pick_for_input(input)?;
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: augment_user_input_with_workspace_context(input)?,
        attachments: Vec::new(),
    });
    let answer = call_llm_with_history(cfg, &build_system_prompt(cfg, "chat-lite"), &messages).await?;
    messages.push(ChatMessage {
        role: "assistant".to_string(),
        content: answer.clone(),
        attachments: Vec::new(),
    });
    history::save(session, &messages)?;
    settings::save(cfg)?;
    Ok(answer)
}
