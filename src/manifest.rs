//! What wowmod installed from CurseForge, so `update` knows what to check.
//! Lives next to the config in `%APPDATA%\wowmod\config\manifest.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::wow::Install;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub mods: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Flavor folder the mod was installed into.
    pub install: PathBuf,
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    /// Folders it put in `Interface\AddOns`.
    pub folders: Vec<String>,
    pub installed_at: String,
}

impl Manifest {
    fn path() -> Option<PathBuf> {
        crate::config::Config::path().map(|p| p.with_file_name("manifest.toml"))
    }

    pub fn load() -> Manifest {
        Manifest::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Manifest::path().context("no config dir")?;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, install: &Path, mod_id: u64) -> Option<&Entry> {
        self.mods
            .iter()
            .find(|e| e.install == install && e.mod_id == mod_id)
    }

    pub fn set(&mut self, entry: Entry) {
        self.mods
            .retain(|e| !(e.install == entry.install && e.mod_id == entry.mod_id));
        self.mods.push(entry);
    }

    /// Mod ids recorded for one install, dropping any whose folders are all
    /// gone (uninstalled since) so `update` does not bring them back.
    pub fn ids_for(&mut self, install: &Install) -> Vec<u64> {
        let addons = install.addons_dir();
        self.mods.retain(|e| {
            e.install != install.path || e.folders.iter().any(|f| addons.join(f).is_dir())
        });
        self.mods
            .iter()
            .filter(|e| e.install == install.path)
            .map(|e| e.mod_id)
            .collect()
    }
}
