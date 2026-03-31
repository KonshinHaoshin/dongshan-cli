use anyhow::Result;

use crate::entrypoints::tui::run_terminal_app;
use crate::query::turn_runner::ExecutionMode;
use crate::services::skills;

pub async fn run_chat_app(session: &str) -> Result<()> {
    let session = skills::resolve_session_name(session)?;
    run_terminal_app(session, ExecutionMode::AgentAuto, None).await
}
