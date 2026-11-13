//! Installing and updating addons from CurseForge.
//!
//! Uses the JSON endpoints behind www.curseforge.com, which need no API key.
//! They are undocumented, so this stays small. "Install" or meta packages
//! (e.g. Spoken: English) ship an empty stub zip and list the real addons as
//! required dependencies, so every mod is installed along with its
//! dependencies, recursively.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::flavor::Flavor;
use crate::installer;
use crate::manifest::{Entry, Manifest};
use crate::package::{Package, TempDir};
use crate::wow::Install;

const MODS: &str = "https://www.curseforge.com/api/v1/mods";
const AGENT: &str = concat!("wowmod/", env!("CARGO_PKG_VERSION"));

#[derive(Deserialize)]
struct Page<T> {
    data: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: u64,
    pub display_name: String,
    /// Bytes.
    pub file_length: u64,
    /// 1 release, 2 beta, 3 alpha.
    release_type: u8,
    /// Client versions such as "1.60.1" or "12.0.5".
    game_versions: Vec<String>,
}

impl File {
    /// Does this file target the flavor? Files with no usable versions are
    /// let through rather than blocked.
    fn fits(&self, flavor: &Flavor) -> bool {
        let interfaces: Vec<u32> = self
            .game_versions
            .iter()
            .filter_map(|v| interface_of(v))
            .collect();
        interfaces.is_empty() || interfaces.iter().any(|&i| flavor.matches_interface(i))
    }
}

/// "1.60.1" -> 16001, the `## Interface:` number for that client version.
fn interface_of(version: &str) -> Option<u32> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    let (a, b, c) = (parts.next()??, parts.next()??, parts.next()??);
    Some(a * 10_000 + b * 100 + c)
}

#[derive(Deserialize)]
struct Dependency {
    id: u64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Widget {
    id: u64,
}

fn get_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    ureq::get(url)
        .header("User-Agent", AGENT)
        .call()
        .with_context(|| format!("fetching {url}"))?
        .body_mut()
        .read_json()
        .with_context(|| format!("reading {url}"))
}

/// Project id from an id, a `/projects/<id>` link or an addon page URL.
pub fn project_id(input: &str) -> Result<u64> {
    let s = input.trim().trim_end_matches('/');
    if let Ok(id) = s.parse() {
        return Ok(id);
    }
    let after = |marker: &str| {
        s.split(marker)
            .nth(1)
            .and_then(|rest| rest.split(['/', '?', '#']).next())
    };
    if let Some(id) = after("/projects/").and_then(|p| p.parse().ok()) {
        return Ok(id);
    }
    if let Some(slug) = after("/wow/addons/") {
        let w: Widget = get_json(&format!("https://api.cfwidget.com/wow/addons/{slug}"))
            .with_context(|| {
                format!("could not look up {slug}; pass the Project ID from the addon page instead")
            })?;
        return Ok(w.id);
    }
    bail!("expected a CurseForge addon URL or project id, got {input}")
}

/// Newest file for the flavor, preferring stable releases over betas.
fn latest_file(mod_id: u64, flavor: &Flavor) -> Result<Option<File>> {
    let page: Page<File> = get_json(&format!("{MODS}/{mod_id}/files?pageIndex=0&pageSize=50"))?;
    let fits: Vec<File> = page.data.into_iter().filter(|f| f.fits(flavor)).collect();
    let stable = fits
        .iter()
        .filter(|f| f.release_type == 1)
        .max_by_key(|f| f.id);
    Ok(stable.or(fits.iter().max_by_key(|f| f.id)).cloned())
}

fn required_deps(mod_id: u64) -> Result<Vec<u64>> {
    let page: Page<Dependency> = get_json(&format!("{MODS}/{mod_id}/dependencies?pageSize=50"))?;
    Ok(page
        .data
        .into_iter()
        .filter(|d| d.kind == "RequiredDependency")
        .map(|d| d.id)
        .collect())
}

fn download(mod_id: u64, file_id: u64, dest: &Path) -> Result<()> {
    let url = format!("{MODS}/{mod_id}/files/{file_id}/download");
    let mut resp = ureq::get(&url)
        .header("User-Agent", AGENT)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut out = fs::File::create(dest)?;
    io::copy(&mut resp.body_mut().as_reader(), &mut out)
        .with_context(|| format!("downloading {url}"))?;
    Ok(())
}

/// One mod that needs installing: missing, or a newer file is out.
#[derive(Debug, Clone)]
pub struct Planned {
    pub mod_id: u64,
    pub file: File,
}

/// What installing or updating some mods into one install would do.
#[derive(Debug, Default)]
pub struct Plan {
    pub todo: Vec<Planned>,
    pub up_to_date: usize,
    /// Mods with no file for the flavor, or that could not be looked up.
    pub problems: Vec<String>,
}

/// Work out what `ids` and everything they require (recursively) need in
/// `target`. Mods whose recorded file is already the newest are skipped.
pub fn plan(ids: &[u64], target: &Install, manifest: &Manifest) -> Plan {
    let mut plan = Plan::default();
    let mut queue = ids.to_vec();
    let mut seen = HashSet::new();
    let addons = target.addons_dir();
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        let file = required_deps(id).and_then(|deps| {
            queue.extend(deps);
            latest_file(id, &target.flavor)
        });
        match file {
            Ok(Some(file)) => {
                let current = manifest.get(&target.path, id).is_some_and(|e| {
                    e.file_id == file.id && e.folders.iter().all(|f| addons.join(f).is_dir())
                });
                if current {
                    plan.up_to_date += 1;
                } else {
                    plan.todo.push(Planned { mod_id: id, file });
                }
            }
            Ok(None) => plan.problems.push(format!(
                "mod {id} has no file for {}",
                target.flavor.display_name()
            )),
            Err(e) => plan.problems.push(format!("mod {id}: {e:#}")),
        }
    }
    plan
}

/// Download one planned mod, install it and record it in the manifest.
pub fn install(
    p: &Planned,
    target: &Install,
    opts: &installer::Options,
    manifest: &mut Manifest,
) -> Result<()> {
    let temp = TempDir::new()?;
    let zip = temp.path.join(format!("{}-{}.zip", p.mod_id, p.file.id));
    download(p.mod_id, p.file.id, &zip)?;
    let pkg = Package::analyze(&zip, Some(&target.flavor))?;
    let report = installer::install(&pkg, target, opts)?;
    manifest.set(Entry {
        install: target.path.clone(),
        mod_id: p.mod_id,
        file_id: p.file.id,
        name: p.file.display_name.clone(),
        folders: report.installed.iter().map(|r| r.folder.clone()).collect(),
        installed_at: chrono::Local::now().to_rfc3339(),
    });
    manifest.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_from_input() {
        assert_eq!(project_id("1721569").unwrap(), 1721569);
        assert_eq!(
            project_id("https://www.curseforge.com/projects/1721569/").unwrap(),
            1721569
        );
        assert!(project_id("spoken").is_err());
    }

    #[test]
    fn files_match_flavor_by_game_version() {
        let file = |versions: &[&str]| File {
            id: 1,
            display_name: String::new(),
            file_length: 0,
            release_type: 1,
            game_versions: versions.iter().map(|v| v.to_string()).collect(),
        };
        assert_eq!(interface_of("1.60.1"), Some(16001));
        assert!(file(&["1.15.9", "1.60.1"]).fits(&Flavor::new("wowf")));
        assert!(!file(&["1.15.9"]).fits(&Flavor::new("wowf")));
        assert!(file(&["Retail"]).fits(&Flavor::new("wowf")));
    }
}
