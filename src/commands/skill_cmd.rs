use anyhow::{Result, bail};

use crate::cli::SkillCommand;
use crate::skills::{
    find_skill, load_active_skill_for_session, load_skills, resolve_session_name,
    save_active_skill_for_session,
};
use crate::util::truncate_preview;

pub fn handle_skill(command: SkillCommand) -> Result<()> {
    match command {
        SkillCommand::List => {
            let skills = load_skills()?;
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
        SkillCommand::Show { name } => {
            let Some(skill) = find_skill(&name)? else {
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
        SkillCommand::Use { name, session } => {
            let Some(skill) = find_skill(&name)? else {
                bail!("Skill not found: {}", name);
            };
            let session = resolve_session_name(&session)?;
            save_active_skill_for_session(&session, Some(&skill.manifest.name))?;
            println!(
                "Active skill for session '{}' set to '{}'.",
                session, skill.manifest.name
            );
        }
        SkillCommand::Clear { session } => {
            let session = resolve_session_name(&session)?;
            save_active_skill_for_session(&session, None)?;
            println!("Active skill cleared for session '{}'.", session);
        }
        SkillCommand::Current { session } => {
            let session = resolve_session_name(&session)?;
            match load_active_skill_for_session(&session)? {
                Some(name) => println!("Active skill for session '{}': {}", session, name),
                None => println!("Active skill for session '{}': (none)", session),
            }
        }
    }
    Ok(())
}
