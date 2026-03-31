use anyhow::Result;

use crate::llm::ChatMessage;
use crate::services::sessions;

pub fn load(session: &str) -> Result<Vec<ChatMessage>> {
    sessions::load(session)
}

pub fn save(session: &str, messages: &[ChatMessage]) -> Result<()> {
    sessions::save(session, messages)
}

pub fn transcript_lines(messages: &[ChatMessage]) -> Vec<String> {
    sessions::transcript_lines(messages)
}
