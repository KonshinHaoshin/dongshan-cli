use anyhow::Result;

use crate::skills::{
    LoadedSkill, find_skill, load_active_skill_for_session, load_skills, pick_skill_for_input,
    resolve_session_name as legacy_resolve_session_name, save_active_skill_for_session,
};

pub fn load_all() -> Result<Vec<LoadedSkill>> {
    load_skills()
}

pub fn find(name: &str) -> Result<Option<LoadedSkill>> {
    find_skill(name)
}

pub fn pick_for_input(input: &str) -> Result<Option<LoadedSkill>> {
    pick_skill_for_input(input)
}

pub fn resolve_session_name(session: &str) -> Result<String> {
    legacy_resolve_session_name(session)
}

pub fn load_active_for_session(session: &str) -> Result<Option<String>> {
    load_active_skill_for_session(session)
}

pub fn save_active_for_session(session: &str, skill: Option<&str>) -> Result<()> {
    save_active_skill_for_session(session, skill)
}
