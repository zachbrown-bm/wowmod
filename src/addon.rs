//! Listing what is installed in an `Interface\AddOns` folder.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::flavor::Flavor;
use crate::toc::{self, Toc};
use crate::wow::Install;

/// One folder inside AddOns.
#[derive(Debug, Clone)]
pub struct InstalledFolder {
    pub name: String,
    pub path: PathBuf,
    /// `None` when the folder has no TOC at all (leftover junk).
    pub toc: Option<Toc>,
    pub enabled: EnableState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableState {
    /// Enabled on every character we found.
    Enabled,
    /// Disabled on every character we found.
    Disabled,
    /// Differs between characters.
    Mixed,
    /// Not mentioned in any AddOns.txt, which the client treats as enabled.
    Unknown,
}

impl EnableState {
    pub fn short(&self) -> &'static str {
        match self {
            EnableState::Enabled => "on",
            EnableState::Disabled => "off",
            EnableState::Mixed => "mixed",
            EnableState::Unknown => "-",
        }
    }
}

/// A logical addon: one primary folder plus any sibling folders that belong
/// to it (DBM-Core + DBM-GUI + DBM-StatusBarTimers...).
#[derive(Debug, Clone)]
pub struct InstalledAddon {
    pub primary: InstalledFolder,
    pub members: Vec<InstalledFolder>,
}

impl InstalledAddon {
    pub fn title(&self) -> String {
        self.primary
            .toc
            .as_ref()
            .and_then(|t| t.title.clone())
            .unwrap_or_else(|| self.primary.name.clone())
    }
    pub fn version(&self) -> String {
        self.primary
            .toc
            .as_ref()
            .and_then(|t| t.version.clone())
            .unwrap_or_default()
    }
    pub fn folder_names(&self) -> Vec<String> {
        std::iter::once(self.primary.name.clone())
            .chain(self.members.iter().map(|m| m.name.clone()))
            .collect()
    }
    pub fn folder_paths(&self) -> Vec<PathBuf> {
        std::iter::once(self.primary.path.clone())
            .chain(self.members.iter().map(|m| m.path.clone()))
            .collect()
    }
    /// `Some(false)` when every TOC Interface value targets another flavor.
    pub fn supports(&self, flavor: &Flavor) -> Option<bool> {
        self.primary.toc.as_ref()?.supports(flavor)
    }
    /// Dependencies not satisfied by any installed folder.
    pub fn missing_deps(&self, installed: &BTreeSet<String>) -> Vec<String> {
        let Some(toc) = &self.primary.toc else {
            return Vec::new();
        };
        toc.dependencies
            .iter()
            .filter(|d| !installed.iter().any(|i| i.eq_ignore_ascii_case(d)))
            .cloned()
            .collect()
    }
}

/// Scan an install and group its AddOns folders into logical addons.
pub fn scan(install: &Install) -> Vec<InstalledAddon> {
    let dir = install.addons_dir();
    let states = read_enable_states(&install.wtf_dir());
    let mut folders: Vec<InstalledFolder> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let toc = toc::read_dir_toc(&path, Some(&install.flavor));
            let enabled = states
                .get(&name.to_lowercase())
                .copied()
                .unwrap_or(EnableState::Unknown);
            folders.push(InstalledFolder {
                name,
                path,
                toc,
                enabled,
            });
        }
    }
    group(folders)
}

/// Group folders into addons.
///
/// Folders sharing an `X-Curse-Project-ID` / `X-Wago-ID` / `X-WoWI-ID` belong
/// together. Otherwise a folder that depends on a sibling *and* shares that
/// sibling's name prefix (`DBM-GUI` -> `DBM-Core`) is attached to it. Pure
/// library folders that many addons depend on stay separate.
fn group(folders: Vec<InstalledFolder>) -> Vec<InstalledAddon> {
    let by_name: HashMap<String, usize> = folders
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.to_lowercase(), i))
        .collect();

    // Union-find over folder indices.
    let mut parent: Vec<usize> = (0..folders.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    fn union(p: &mut [usize], a: usize, b: usize) {
        let ra = find(p, a);
        let rb = find(p, b);
        if ra != rb {
            p[rb] = ra;
        }
    }

    // 1. Shared project ids.
    let mut by_id: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, f) in folders.iter().enumerate() {
        if let Some(t) = &f.toc {
            for key in ["X-Curse-Project-ID", "X-Wago-ID", "X-WoWI-ID"] {
                if let Some(v) = t.extra.get(key) {
                    by_id
                        .entry(format!("{key}={}", v.trim()))
                        .or_default()
                        .push(i);
                }
            }
        }
    }
    for idxs in by_id.values() {
        for w in idxs.windows(2) {
            union(&mut parent, w[0], w[1]);
        }
    }

    // 2. Dependency on a sibling with a shared name prefix.
    for (i, f) in folders.iter().enumerate() {
        let Some(t) = &f.toc else { continue };
        for dep in &t.dependencies {
            if let Some(&j) = by_name.get(&dep.to_lowercase())
                && shares_prefix(&f.name, &folders[j].name)
            {
                union(&mut parent, i, j);
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..folders.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }

    let mut addons: Vec<InstalledAddon> = Vec::new();
    let mut taken = vec![false; folders.len()];
    for idxs in groups.values() {
        // Primary: the folder nothing else in the group depends on... or
        // simply the one with the shortest name, which is almost always
        // the "Core"/main folder.
        let primary_idx = *idxs
            .iter()
            .min_by_key(|&&i| (is_depended_on(&folders, idxs, i), folders[i].name.len()))
            .unwrap();
        let mut members = Vec::new();
        for &i in idxs {
            if i != primary_idx {
                members.push(folders[i].clone());
            }
            taken[i] = true;
        }
        members.sort_by(|a, b| a.name.cmp(&b.name));
        addons.push(InstalledAddon {
            primary: folders[primary_idx].clone(),
            members,
        });
    }
    addons.sort_by_key(|a| a.title().to_lowercase());
    addons
}

/// `true` when some other group member depends on `i` (so `i` is a core).
/// Returned inverted (as 0/1) so `min_by_key` prefers cores.
fn is_depended_on(folders: &[InstalledFolder], group: &[usize], i: usize) -> u8 {
    let name = folders[i].name.to_lowercase();
    let depended = group.iter().any(|&j| {
        j != i
            && folders[j]
                .toc
                .as_ref()
                .is_some_and(|t| t.dependencies.iter().any(|d| d.to_lowercase() == name))
    });
    if depended { 0 } else { 1 }
}

fn shares_prefix(a: &str, b: &str) -> bool {
    let pa = a.split(['-', '_']).next().unwrap_or(a);
    let pb = b.split(['-', '_']).next().unwrap_or(b);
    pa.eq_ignore_ascii_case(pb)
}

/// Every character's `WTF\Account\<account>\<realm>\<character>\AddOns.txt`.
fn addons_files(wtf: &Path) -> Vec<PathBuf> {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    };
    dirs(&wtf.join("Account"))
        .iter()
        .flat_map(|account| dirs(account))
        .flat_map(|realm| dirs(&realm))
        .map(|character| character.join("AddOns.txt"))
        .filter(|f| f.is_file())
        .collect()
}

/// Merge every character's AddOns.txt into one state per folder.
fn read_enable_states(wtf: &Path) -> HashMap<String, EnableState> {
    let mut seen: HashMap<String, (bool, bool)> = HashMap::new(); // (any_enabled, any_disabled)
    for f in addons_files(wtf) {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for line in text.lines() {
            let Some((name, state)) = line.split_once(':') else {
                continue;
            };
            let e = seen
                .entry(name.trim().to_lowercase())
                .or_insert((false, false));
            if state.trim().eq_ignore_ascii_case("enabled") {
                e.0 = true;
            } else {
                e.1 = true;
            }
        }
    }
    seen.into_iter()
        .map(|(k, (en, dis))| {
            let s = match (en, dis) {
                (true, false) => EnableState::Enabled,
                (false, true) => EnableState::Disabled,
                (true, true) => EnableState::Mixed,
                (false, false) => EnableState::Unknown,
            };
            (k, s)
        })
        .collect()
}

/// Turn folders on or off for every character, adding them to any
/// AddOns.txt that does not list them yet. Returns the number of characters.
pub fn set_enabled(wtf: &Path, folders: &[String], on: bool) -> Result<usize> {
    let state = if on { "enabled" } else { "disabled" };
    let files = addons_files(wtf);
    for file in &files {
        let text = fs::read_to_string(file)?;
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut todo: Vec<&String> = folders.iter().collect();
        let mut lines: Vec<String> = Vec::new();
        for line in text.lines() {
            let name = line.split_once(':').map(|(n, _)| n.trim()).unwrap_or("");
            match todo.iter().position(|f| f.eq_ignore_ascii_case(name)) {
                Some(i) => {
                    todo.remove(i);
                    lines.push(format!("{name}: {state}"));
                }
                None => lines.push(line.to_string()),
            }
        }
        lines.extend(todo.iter().map(|f| format!("{f}: {state}")));
        let new = lines.join(eol) + eol;
        if new != text {
            fs::write(file, new).with_context(|| format!("writing {}", file.display()))?;
        }
    }
    Ok(files.len())
}

/// Lower-cased set of installed folder names, for dependency checks.
pub fn folder_name_set(addons: &[InstalledAddon]) -> BTreeSet<String> {
    addons.iter().flat_map(|a| a.folder_names()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_enabled_rewrites_every_character() {
        let temp = crate::package::TempDir::new().unwrap();
        let wtf = temp.path.join("WTF");
        let realm = wtf.join("Account/ACC/Realm");
        let (a, b) = (realm.join("Alice"), realm.join("Bob"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(
            a.join("AddOns.txt"),
            "DBM-Core: enabled\r\nOther: enabled\r\n",
        )
        .unwrap();
        fs::write(b.join("AddOns.txt"), "Other: disabled\n").unwrap();

        let folders = vec!["DBM-Core".to_string(), "DBM-GUI".to_string()];
        assert_eq!(set_enabled(&wtf, &folders, false).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(a.join("AddOns.txt")).unwrap(),
            "DBM-Core: disabled\r\nOther: enabled\r\nDBM-GUI: disabled\r\n"
        );
        assert_eq!(
            fs::read_to_string(b.join("AddOns.txt")).unwrap(),
            "Other: disabled\nDBM-Core: disabled\nDBM-GUI: disabled\n"
        );
        let states = read_enable_states(&wtf);
        assert_eq!(states["dbm-core"], EnableState::Disabled);
        assert_eq!(states["other"], EnableState::Mixed);

        set_enabled(&wtf, &folders, true).unwrap();
        assert_eq!(read_enable_states(&wtf)["dbm-gui"], EnableState::Enabled);
    }
}
