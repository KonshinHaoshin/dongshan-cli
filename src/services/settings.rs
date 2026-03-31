#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;

use crate::config::{Config, config_dir, config_path, load_config_or_default, save_config};

pub fn load() -> Result<Config> {
    load_config_or_default()
}

pub fn save(cfg: &Config) -> Result<()> {
    save_config(cfg)
}

pub fn path() -> Result<PathBuf> {
    config_path()
}

pub fn root_dir() -> Result<PathBuf> {
    config_dir()
}

pub fn initialize() -> Result<Config> {
    let cfg = Config::default();
    save(&cfg)?;
    Ok(cfg)
}
