//! Persistent settings in `%APPDATA%\wowmod\config\config.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Color for key names in the status bar, dialog footers and help. A touch
/// lighter than the usual terminal dark gray, and an RGB value so terminal
/// themes (Solarized maps dark gray to its background) cannot hide it.
pub const DEFAULT_KEY_COLOR: &str = "#8a8a8a";

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
    /// Color for key names in hints, as a ratatui color string: a name such
    /// as `gray`, `light blue` or `yellow`, a hex value such as `#8a8a8a`,
    /// or a 0-255 palette index.
    pub key_color: String,
    /// Install new wowmod releases from GitHub on startup.
    pub auto_update: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wow_roots: Vec::new(),
            browse_dir: None,
            last_flavor: None,
            backups: true,
            filter_browser: true,
            key_color: DEFAULT_KEY_COLOR.into(),
            auto_update: true,
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "wowmod")?;
        Some(dirs.config_dir().join("config.toml"))
    }

    pub fn load() -> Config {
        match Config::path() {
            Some(p) => Config::load_from(&p),
            None => Config::default(),
        }
    }

    /// Read the file, filling in defaults for anything it lacks. If the file
    /// exists but is missing a setting, it is rewritten so the user can see
    /// and edit every entry.
    pub fn load_from(path: &Path) -> Config {
        let Ok(text) = fs::read_to_string(path) else {
            return Config::default();
        };
        let config: Config = toml::from_str(&text).unwrap_or_default();
        let complete = toml::from_str::<toml::Table>(&text)
            .map(|t| Config::field_names().iter().all(|f| t.contains_key(*f)))
            .unwrap_or(false);
        if !complete {
            let _ = config.save_to(path);
        }
        config
    }

    /// Settings a complete file always contains. `browse_dir` and
    /// `last_flavor` are optional and simply absent when unset.
    fn field_names() -> &'static [&'static str] {
        &[
            "wow_roots",
            "backups",
            "filter_browser",
            "key_color",
            "auto_update",
        ]
    }

    pub fn save(&self) -> Result<()> {
        let p = Config::path().context("no config dir")?;
        self.save_to(&p)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_entries_are_added_to_the_file() {
        let temp = crate::package::TempDir::new().unwrap();
        let path = temp.path.join("config.toml");
        fs::write(&path, "backups = false\n").unwrap();
        let config = Config::load_from(&path);
        assert!(!config.backups);
        assert_eq!(config.key_color, DEFAULT_KEY_COLOR);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("key_color = \"#8a8a8a\""), "{text}");
        assert!(text.contains("backups = false"), "{text}");

        // A complete file is left alone.
        fs::write(&path, format!("{text}# note\n")).unwrap();
        let config = Config::load_from(&path);
        assert_eq!(config.key_color, DEFAULT_KEY_COLOR);
        assert!(fs::read_to_string(&path).unwrap().contains("# note"));
    }
}
