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

    /// Entries in `install` that own any of `folders`.
    pub fn owners<'a>(
        &'a self,
        install: &'a Path,
        folders: &'a [String],
    ) -> impl Iterator<Item = &'a Entry> {
        self.mods
            .iter()
            .filter(move |e| e.install == install && e.folders.iter().any(|f| folders.contains(f)))
    }

    /// Stop tracking mods whose folders were replaced by a manual install.
    pub fn forget(&mut self, install: &Path, folders: &[String]) {
        self.mods
            .retain(|e| e.install != install || !e.folders.iter().any(|f| folders.contains(f)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_install_forgets_mods_sharing_a_folder() {
        let entry = |install: &str, id, folders: &[&str]| Entry {
            install: install.into(),
            mod_id: id,
            file_id: 1,
            name: format!("mod {id}"),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            installed_at: String::new(),
        };
        let mut m = Manifest {
            mods: vec![
                entry("retail", 1, &["Spoken", "Spoken_Data"]),
                entry("retail", 2, &["Other"]),
                entry("classic", 3, &["Spoken"]),
            ],
        };
        let zip = vec!["Spoken_Data".to_string()];
        let retail = Path::new("retail");
        assert_eq!(
            m.owners(retail, &zip).map(|e| e.mod_id).collect::<Vec<_>>(),
            [1]
        );
        m.forget(retail, &zip);
        assert_eq!(m.mods.iter().map(|e| e.mod_id).collect::<Vec<_>>(), [2, 3]);
    }
}
