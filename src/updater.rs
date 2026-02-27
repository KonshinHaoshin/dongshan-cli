use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, config_dir};

const REPO_OWNER: &str = "KonshinHaoshin";
const REPO_NAME: &str = "dongshan-cli";
const CHECK_INTERVAL_SECS: u64 = 60 * 60 * 24;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UpdateState {
    last_check_unix: u64,
    last_seen_remote: Option<String>,
}

pub async fn maybe_check_update(cfg: &Config) -> Result<()> {
    if !cfg.auto_check_update {
        return Ok(());
    }

    let mut state = load_state().unwrap_or_default();
    let now = now_unix();
    if now.saturating_sub(state.last_check_unix) < CHECK_INTERVAL_SECS {
        return Ok(());
    }

    let current = env!("CARGO_PKG_VERSION");
    let latest = match fetch_latest_version().await {
        Ok(v) => v,
        Err(_) => {
            state.last_check_unix = now;
            save_state(&state)?;
            return Ok(());
        }
    };

    state.last_check_unix = now;
    state.last_seen_remote = Some(latest.clone());
    save_state(&state)?;

    if is_remote_newer(current, &latest) {
        println!(
            "Update available: {} -> {}\nRun: cargo install --git https://github.com/{}/{} --force",
            current, latest, REPO_OWNER, REPO_NAME
        );
    }

    Ok(())
}

async fn fetch_latest_version() -> Result<String> {
    let client = Client::new();
    let latest_release_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        REPO_OWNER, REPO_NAME
    );
    let latest_tag_url = format!(
        "https://api.github.com/repos/{}/{}/tags?per_page=1",
        REPO_OWNER, REPO_NAME
    );

    if let Ok(v) = fetch_release_latest(&client, &latest_release_url).await {
        return Ok(v);
    }
    fetch_tag_latest(&client, &latest_tag_url).await
}

async fn fetch_release_latest(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .header("User-Agent", "dongshan-cli-update-checker")
        .send()
        .await
        .context("request latest release failed")?;
    if !resp.status().is_success() {
        anyhow::bail!("latest release status {}", resp.status());
    }
    let v: Value = resp.json().await.context("invalid latest release json")?;
    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim();
    if tag.is_empty() {
        anyhow::bail!("missing tag_name");
    }
    Ok(normalize_version(tag))
}

async fn fetch_tag_latest(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .header("User-Agent", "dongshan-cli-update-checker")
        .send()
        .await
        .context("request tags failed")?;
    if !resp.status().is_success() {
        anyhow::bail!("tags status {}", resp.status());
    }
    let v: Value = resp.json().await.context("invalid tags json")?;
    let tag = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|item| item.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .trim();
    if tag.is_empty() {
        anyhow::bail!("missing tag name");
    }
    Ok(normalize_version(tag))
}

fn normalize_version(v: &str) -> String {
    v.trim_start_matches('v').to_string()
}

fn parse_version(v: &str) -> (u64, u64, u64) {
    let core = v.split('-').next().unwrap_or(v);
    let mut it = core.split('.');
    let major = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let minor = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let patch = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

fn is_remote_newer(current: &str, remote: &str) -> bool {
    parse_version(remote) > parse_version(current)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn update_state_path() -> Result<std::path::PathBuf> {
    Ok(config_dir()?.join("update_state.json"))
}

fn load_state() -> Result<UpdateState> {
    let path = update_state_path()?;
    if !path.exists() {
        return Ok(UpdateState::default());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let s: UpdateState =
        serde_json::from_str(&text).with_context(|| format!("Invalid {}", path.display()))?;
    Ok(s)
}

fn save_state(state: &UpdateState) -> Result<()> {
    let path = update_state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(state)?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}
