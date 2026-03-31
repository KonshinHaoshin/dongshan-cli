use anyhow::{Result, bail};

use crate::cli::SkillsCommand;
use crate::services::skills as skills_service;
use crate::util::truncate_preview;

pub fn handle_skills(command: SkillsCommand) -> Result<()> {
    match command {
        SkillsCommand::List => {
            let skills = skills_service::load_all()?;
            if skills.is_empty() {
                println!("No skills found.");
                return Ok(());
            }
            println!("Skills:");
            for skill in skills {
                let desc = if skill.manifest.description.trim().is_empty() {
                    "(no description)".to_string()
                } else {
                    truncate_preview(&skill.manifest.description, 80)
                };
                println!("- {}: {}", skill.manifest.name, desc);
            }
        }
        SkillsCommand::Show { name } => {
            let Some(skill) = skills_service::find(&name)? else {
                bail!("Skill not found: {}", name);
            };
            println!("Skill: {}", skill.manifest.name);
            println!("  description: {}", skill.manifest.description);
            println!("  version: {}", skill.manifest.version);
            println!("  root: {}", skill.root_dir.display());
            println!("  entry_mode: {}", skill.manifest.entry_mode);
            println!("  priority: {}", skill.manifest.priority);
            println!("  triggers: {}", skill.manifest.triggers.join(", "));
            println!("  allowed_tools: {}", skill.manifest.allowed_tools.join(", "));
            println!(
                "  trusted_commands: {}",
                skill.manifest.trusted_commands.join(", ")
            );
            if !skill.prompt_text.trim().is_empty() {
                println!("  prompt: {}", truncate_preview(&skill.prompt_text, 240));
            }
        }
        SkillsCommand::Use { name, session } => {
            let Some(skill) = skills_service::find(&name)? else {
                bail!("Skill not found: {}", name);
            };
            let session = skills_service::resolve_session_name(&session)?;
            skills_service::save_active_for_session(&session, Some(&skill.manifest.name))?;
            println!(
                "Active skill for session '{}' set to '{}'.",
                session, skill.manifest.name
            );
        }
        SkillsCommand::Clear { session } => {
            let session = skills_service::resolve_session_name(&session)?;
            skills_service::save_active_for_session(&session, None)?;
            println!("Active skill cleared for session '{}'.", session);
        }
        SkillsCommand::Current { session } => {
            let session = skills_service::resolve_session_name(&session)?;
            match skills_service::load_active_for_session(&session)? {
                Some(name) => println!("Active skill for session '{}': {}", session, name),
                None => println!("Active skill for session '{}': (none)", session),
            }
        }
    }
    Ok(())
}
