use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::config_dir;

static RUNTIME_ACTIVE_SKILL: OnceLock<Mutex<Option<LoadedSkill>>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default = "default_prompt_file")]
    pub prompt_file: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub trusted_commands: Vec<String>,
    #[serde(default = "default_entry_mode")]
    pub entry_mode: String,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub manifest: SkillManifest,
    pub root_dir: PathBuf,
    pub prompt_text: String,
}

fn default_prompt_file() -> String {
    "PROMPT.md".to_string()
}

fn default_entry_mode() -> String {
    "augment".to_string()
}

pub fn load_skills() -> Result<Vec<LoadedSkill>> {
    let mut out = Vec::new();
    for dir in candidate_skill_dirs()? {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(skill) = load_skill_dir(&path)? else {
                continue;
            };
            if let Some(idx) = out
                .iter()
                .position(|v: &LoadedSkill| v.manifest.name.eq_ignore_ascii_case(&skill.manifest.name))
            {
                out[idx] = skill;
            } else {
                out.push(skill);
            }
        }
    }
    out.sort_by(|a, b| {
        b.manifest
            .priority
            .cmp(&a.manifest.priority)
            .then_with(|| a.manifest.name.cmp(&b.manifest.name))
    });
    Ok(out)
}

pub fn find_skill(name: &str) -> Result<Option<LoadedSkill>> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    Ok(load_skills()?
        .into_iter()
        .find(|s| s.manifest.name.eq_ignore_ascii_case(wanted)))
}

pub fn pick_skill_for_input(input: &str) -> Result<Option<LoadedSkill>> {
    let text = input.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let lower = text.to_ascii_lowercase();
    for skill in load_skills()? {
        if skill
            .manifest
            .triggers
            .iter()
            .filter(|t| !t.trim().is_empty())
            .any(|t| {
                let trigger = t.trim();
                lower.contains(&trigger.to_ascii_lowercase()) || text.contains(trigger)
            })
        {
            return Ok(Some(skill));
        }
    }
    Ok(None)
}

pub fn set_runtime_active_skill(skill: Option<LoadedSkill>) -> Result<()> {
    let lock = RUNTIME_ACTIVE_SKILL.get_or_init(|| Mutex::new(None));
    let mut guard = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("failed to lock runtime skill context"))?;
    *guard = skill;
    Ok(())
}

pub fn runtime_active_skill() -> Option<LoadedSkill> {
    let lock = RUNTIME_ACTIVE_SKILL.get_or_init(|| Mutex::new(None));
    lock.lock().ok().and_then(|g| g.clone())
}

pub fn load_active_skill_for_session(session: &str) -> Result<Option<String>> {
    let path = session_skill_path(session)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read session skill {}", path.display()))?;
    let name = text.trim();
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(name.to_string()))
}

pub fn save_active_skill_for_session(session: &str, skill: Option<&str>) -> Result<()> {
    let path = session_skill_path(session)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create session dir {}", parent.display()))?;
    }
    match skill.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => fs::write(&path, format!("{name}\n"))
            .with_context(|| format!("Failed to write {}", path.display()))?,
        None => {
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

pub fn resolve_session_name(requested: &str) -> Result<String> {
    let name = if requested == "default" || requested == "auto" {
        workspace_session_base()?
    } else {
        requested.to_string()
    };
    Ok(sanitize_session_name(&name))
}

fn candidate_skill_dirs() -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    dirs.push(config_dir()?.join("skills"));
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".dongshan").join("skills"));
    }
    Ok(dirs)
}

fn load_skill_dir(dir: &Path) -> Result<Option<LoadedSkill>> {
    let manifest_path = dir.join("SKILL.toml");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: SkillManifest = toml::from_str(&text)
        .with_context(|| format!("Invalid skill manifest {}", manifest_path.display()))?;
    let prompt_path = dir.join(&manifest.prompt_file);
    let prompt_text = if prompt_path.exists() {
        fs::read_to_string(&prompt_path)
            .with_context(|| format!("Failed to read {}", prompt_path.display()))?
    } else {
        String::new()
    };
    Ok(Some(LoadedSkill {
        manifest,
        root_dir: dir.to_path_buf(),
        prompt_text,
    }))
}

fn session_skill_path(session: &str) -> Result<PathBuf> {
    Ok(config_dir()?
        .join("sessions")
        .join(format!("{session}.skill")))
}

fn workspace_session_base() -> Result<String> {
    let cwd = std::env::current_dir()?;
    let cwd = cwd.to_string_lossy().to_string();
    let mut hasher = DefaultHasher::new();
    cwd.hash(&mut hasher);
    let hash = hasher.finish();
    let leaf = Path::new(&cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("workspace");
    Ok(format!("ws-{}-{:x}", leaf, hash))
}

fn sanitize_session_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("session-{ts}")
    } else {
        s
    }
}
