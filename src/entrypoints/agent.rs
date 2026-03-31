use anyhow::Result;

use crate::entrypoints::tui::run_terminal_app;
use crate::query::turn_runner::ExecutionMode;
use crate::services::skills;

pub async fn run_agent_app(session: &str, task: &str) -> Result<()> {
    let session = skills::resolve_session_name(session)?;
    run_terminal_app(session, ExecutionMode::AgentForce, Some(task.to_string())).await
}
