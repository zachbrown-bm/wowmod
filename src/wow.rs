//! Discovering World of Warcraft installations on this machine.
//!
//! A "root" is e.g. `D:\World of Warcraft`. Inside it each flavor lives in an
//! underscore folder (`_retail_`, `_forever_`, ...) that contains
//! `.flavor.info` (product code on line 2) and `Interface\AddOns`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::flavor::Flavor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub flavor: Flavor,
    /// e.g. `D:\World of Warcraft\_retail_`
    pub path: PathBuf,
}

impl Install {
    pub fn addons_dir(&self) -> PathBuf {
        self.path.join("Interface").join("AddOns")
    }
    pub fn wtf_dir(&self) -> PathBuf {
        self.path.join("WTF")
    }
    pub fn folder_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    pub fn label(&self) -> String {
        let name = self.flavor.display_name();
        let folder = self.folder_name();
        if name == folder || name == self.flavor.code {
            folder
        } else {
            format!("{name} ({folder})")
        }
    }
}

/// Find every flavor install under the given extra roots plus whatever
/// Battle.net and the registry know about.
pub fn discover(extra_roots: &[PathBuf]) -> Vec<Install> {
    let mut roots: BTreeSet<PathBuf> = BTreeSet::new();
    roots.extend(extra_roots.iter().cloned());
    roots.extend(battle_net_roots());
    roots.extend(registry_roots());
    roots.extend(common_roots());

    let mut installs = Vec::new();
    for root in roots {
        installs.extend(scan_root(&root));
    }
    installs.sort_by(|a, b| a.path.cmp(&b.path));
    installs.dedup();
    installs
}

/// If `path` is itself a flavor folder, or a WoW root, or an AddOns folder,
/// return the installs it describes.
pub fn scan_root(root: &Path) -> Vec<Install> {
    let mut out = Vec::new();
    if let Some(i) = install_from_flavor_dir(root) {
        out.push(i);
        return out;
    }
    // Walk up a couple of levels in case the user gave Interface\AddOns.
    for anc in root.ancestors().skip(1).take(3) {
        if let Some(i) = install_from_flavor_dir(anc) {
            out.push(i);
            return out;
        }
    }
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir()
            && name.starts_with('_')
            && name.ends_with('_')
            && let Some(i) = install_from_flavor_dir(&p)
        {
            out.push(i);
        }
    }
    out
}

fn install_from_flavor_dir(dir: &Path) -> Option<Install> {
    if !dir.join("Interface").is_dir() && !dir.join(".flavor.info").is_file() {
        return None;
    }
    let folder = dir.file_name()?.to_str()?;
    if !(folder.starts_with('_') && folder.ends_with('_')) {
        return None;
    }
    let flavor = read_flavor_info(dir)
        .or_else(|| Flavor::from_folder_name(folder))
        .unwrap_or_else(|| Flavor::new(folder.trim_matches('_')));
    Some(Install {
        flavor,
        path: dir.to_path_buf(),
    })
}

fn read_flavor_info(dir: &Path) -> Option<Flavor> {
    let text = fs::read_to_string(dir.join(".flavor.info")).ok()?;
    // Line 1: "Product Flavor!STRING:0", line 2: "wowf"
    let code = text.lines().nth(1)?.trim();
    if code.is_empty() {
        None
    } else {
        Some(Flavor::new(code))
    }
}

/// Battle.net Agent's product.db is a protobuf blob; the install paths are
/// plain strings inside it, so we just scan for `X:/...World of Warcraft`.
fn battle_net_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(program_data) = std::env::var_os("ProgramData") else {
        return out;
    };
    let db = PathBuf::from(program_data)
        .join("Battle.net")
        .join("Agent")
        .join("product.db");
    let Ok(bytes) = fs::read(&db) else {
        return out;
    };
    for s in printable_runs(&bytes) {
        // Strings look like "D:/World of Warcraft" or "C:/Program Files (x86)/World of Warcraft".
        if s.len() >= 3 && s.as_bytes()[1] == b':' && s.contains("World of Warcraft") {
            let cleaned = s.trim_end_matches(['/', '\\']).replace('/', "\\");
            let p = PathBuf::from(cleaned);
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}

fn printable_runs(bytes: &[u8]) -> Vec<String> {
    let mut runs = Vec::new();
    let mut cur = String::new();
    for &b in bytes {
        if (0x20..0x7f).contains(&b) {
            cur.push(b as char);
        } else {
            if cur.len() >= 4 {
                runs.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.len() >= 4 {
        runs.push(cur);
    }
    runs
}

#[cfg(windows)]
fn registry_roots() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::*;
    let mut out = Vec::new();
    let keys = [
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
        ),
        (
            HKEY_CURRENT_USER,
            r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
        ),
    ];
    for (hive, sub) in keys {
        if let Ok(k) = RegKey::predef(hive).open_subkey(sub) {
            for v in ["InstallPath", "GamePath"] {
                if let Ok(s) = k.get_value::<String, _>(v) {
                    let p = PathBuf::from(s.trim_end_matches(['/', '\\']));
                    // InstallPath usually points at the flavor dir; take its parent too.
                    if p.is_dir() {
                        out.push(p.clone());
                        if let Some(parent) = p.parent() {
                            out.push(parent.to_path_buf());
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn registry_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn common_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let candidates = [
        r"C:\Program Files (x86)\World of Warcraft",
        r"C:\Program Files\World of Warcraft",
        r"C:\World of Warcraft",
        r"C:\Games\World of Warcraft",
        r"D:\World of Warcraft",
        r"D:\Games\World of Warcraft",
        r"E:\World of Warcraft",
        r"E:\Games\World of Warcraft",
        "/Applications/World of Warcraft",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_dir() {
            out.push(p);
        }
    }
    out
}

/// A WoW client is running. It rewrites AddOns.txt on logout, which would
/// undo any change made meanwhile. Assumes not when the process list
/// cannot be read.
pub fn game_running() -> bool {
    let out = if cfg!(windows) {
        std::process::Command::new("tasklist")
            .args(["/fo", "csv", "/nh"])
            .output()
    } else {
        std::process::Command::new("ps")
            .args(["-A", "-o", "comm="])
            .output()
    };
    let Ok(out) = out else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        // tasklist: "Wow.exe","1234",... / ps: a path
        let name = if cfg!(windows) {
            line.split(',').next().unwrap_or("").trim_matches('"')
        } else {
            line.trim()
        };
        is_wow_process(name)
    })
}

/// `Wow.exe`, `WowClassic.exe`, ... on Windows (and Wine), or the
/// `World of Warcraft` app binary on macOS.
fn is_wow_process(name: &str) -> bool {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    base.starts_with("world of warcraft")
        || (base.starts_with("wow") && base.ends_with(".exe") && base != "wowmod.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_wow_clients_but_not_wowmod() {
        assert!(is_wow_process("Wow.exe"));
        assert!(is_wow_process("WowClassic.exe"));
        assert!(is_wow_process(
            "/Applications/World of Warcraft/_retail_/World of Warcraft.app/Contents/MacOS/World of Warcraft"
        ));
        assert!(!is_wow_process("wowmod.exe"));
        assert!(!is_wow_process("/Users/me/.local/bin/wowmod"));
        assert!(!is_wow_process("Battle.net.exe"));
    }
}
