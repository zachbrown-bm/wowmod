//! Copying addon roots into `Interface\AddOns`, with backup of anything we
//! replace, and removal.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::package::{AddonRoot, Package};
use crate::wow::Install;

/// What happened for one installed folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub folder: String,
    pub version: Option<String>,
    pub source: PathBuf,
    pub installed_at: String,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct InstallReport {
    pub installed: Vec<InstalledRecord>,
    pub replaced: Vec<String>,
}

pub struct Options {
    pub backups: bool,
}

/// Where backups go: `<flavor>\Interface\AddOns.wowmod-backup\<Folder>-<stamp>`.
pub fn backup_dir(install: &Install) -> PathBuf {
    install.path.join("Interface").join("AddOns.wowmod-backup")
}

pub fn install(pkg: &Package, target: &Install, opts: &Options) -> Result<InstallReport> {
    let addons = target.addons_dir();
    fs::create_dir_all(&addons).with_context(|| format!("creating {}", addons.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut report = InstallReport::default();

    for root in &pkg.roots {
        let dest = addons.join(&root.name);
        let mut backup = None;
        if dest.exists() {
            report.replaced.push(root.name.clone());
            if opts.backups {
                let bdir = backup_dir(target);
                fs::create_dir_all(&bdir)?;
                let b = bdir.join(format!("{}-{stamp}", root.name));
                move_dir(&dest, &b).with_context(|| format!("backing up {}", dest.display()))?;
                backup = Some(b);
            } else {
                fs::remove_dir_all(&dest)
                    .with_context(|| format!("removing {}", dest.display()))?;
            }
        }
        copy_dir(&root.path, &dest)
            .with_context(|| format!("copying {} to {}", root.path.display(), dest.display()))?;
        report.installed.push(InstalledRecord {
            folder: root.name.clone(),
            version: root.toc.version.clone(),
            source: pkg.source.clone(),
            installed_at: stamp.clone(),
            backup,
        });
    }
    Ok(report)
}

/// Remove the given folders from AddOns (optionally backing them up).
pub fn uninstall(target: &Install, folders: &[PathBuf], opts: &Options) -> Result<Vec<PathBuf>> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut backups = Vec::new();
    for f in folders {
        if !f.exists() {
            continue;
        }
        if opts.backups {
            let bdir = backup_dir(target);
            fs::create_dir_all(&bdir)?;
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let b = bdir.join(format!("{name}-{stamp}"));
            move_dir(f, &b)?;
            backups.push(b);
        } else {
            fs::remove_dir_all(f).with_context(|| format!("removing {}", f.display()))?;
        }
    }
    Ok(backups)
}

/// List backups available for a folder name, newest first.
pub fn backups_for(install: &Install, folder: &str) -> Vec<PathBuf> {
    let prefix = format!("{folder}-");
    let mut out: Vec<PathBuf> = fs::read_dir(backup_dir(install))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with(&prefix) && n.len() == prefix.len() + 15)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.reverse();
    out
}

/// Restore a backup over the live folder (the live folder is discarded).
pub fn rollback(install: &Install, folder: &str, backup: &Path) -> Result<()> {
    let dest = install.addons_dir().join(folder);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    move_dir(backup, &dest)
}

fn move_dir(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Cross-volume or locked: fall back to copy + delete.
    copy_dir(from, to)?;
    fs::remove_dir_all(from)?;
    Ok(())
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in walkdir::WalkDir::new(from).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(from)?;
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// A quick description of what an install would do, for the preview pane.
pub fn preview_lines(roots: &[AddonRoot], target: &Install) -> Vec<String> {
    let addons = target.addons_dir();
    roots
        .iter()
        .map(|r| {
            let exists = addons.join(&r.name).exists();
            let verb = if exists { "replace" } else { "add" };
            format!("{verb} {}", r.name)
        })
        .collect()
}
