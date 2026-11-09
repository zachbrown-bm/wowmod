//! Persistent settings in `%APPDATA%\wowmod\config\config.toml`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Extra WoW roots or flavor folders to scan in addition to auto-detection.
    pub wow_roots: Vec<PathBuf>,
    /// Folder the browser opens in. Defaults to Downloads.
    pub browse_dir: Option<PathBuf>,
    /// Flavor code (e.g. `wowf`) selected last time.
    pub last_flavor: Option<String>,
    /// Move replaced folders to a backup dir instead of deleting them.
    pub backups: bool,
    /// Hide everything but folders and .zip files in the browser.
    pub filter_browser: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wow_roots: Vec::new(),
            browse_dir: None,
            last_flavor: None,
            backups: true,
            filter_browser: true,
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "wowmod")?;
        Some(dirs.config_dir().join("config.toml"))
    }

    pub fn load() -> Config {
        let Some(p) = Config::path() else {
            return Config::default();
        };
        fs::read_to_string(&p)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Config::path().context("no config dir")?;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn start_dir(&self) -> PathBuf {
        if let Some(d) = &self.browse_dir
            && d.is_dir()
        {
            return d.clone();
        }
        directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.to_path_buf()))
            .filter(|d| d.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
