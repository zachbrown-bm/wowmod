//! Updating wowmod itself from the newest GitHub release.
//!
//! The new binary is copied next to the running one as `wowmod.new`, then the
//! running one is renamed to `wowmod.old` and the new one renamed into its
//! place. Renames are instant, so quitting mid-update never leaves wowmod
//! missing. Windows allows renaming a running exe but not overwriting it;
//! the leftover `.old` file is removed on the next check.

use std::fs;
use std::io;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::package::TempDir;

const REPO: &str = "zachbrown-bm/wowmod";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const AGENT: &str = concat!("wowmod/", env!("CARGO_PKG_VERSION"));

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// Release asset for this platform, as built by `.github/workflows/release.yml`.
fn asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("wowmod-windows-x86_64.zip"),
        ("linux", "x86_64") => Some("wowmod-linux-x86_64.tar.gz"),
        ("macos", "aarch64") => Some("wowmod-macos-aarch64.tar.gz"),
        _ => None,
    }
}

/// `1.2.3` style versions; anything unparsable counts as 0.
fn parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    parts(latest) > parts(current)
}

/// Running from a cargo `target` folder, which is never updated.
pub fn is_dev_build() -> bool {
    std::env::current_exe()
        .map(|exe| exe.components().any(|c| c.as_os_str() == "target"))
        .unwrap_or(true)
}

/// Installs the newest release over the running binary. Returns the new
/// version, or `None` when already up to date or a dev build.
pub fn update() -> Result<Option<String>> {
    let exe = std::env::current_exe()?;
    let old = exe.with_extension("old");
    let _ = fs::remove_file(&old);
    if is_dev_build() {
        return Ok(None);
    }

    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let release: Release = ureq::get(&url)
        .header("User-Agent", AGENT)
        .call()
        .with_context(|| format!("fetching {url}"))?
        .body_mut()
        .read_json()?;
    if !is_newer(&release.tag_name, CURRENT) {
        return Ok(None);
    }
    let Some(asset) = asset() else {
        bail!("no prebuilt wowmod for this platform");
    };

    let temp = TempDir::new()?;
    let archive = temp.path.join(asset);
    let url = format!(
        "https://github.com/{REPO}/releases/download/{}/{asset}",
        release.tag_name
    );
    let mut resp = ureq::get(&url)
        .header("User-Agent", AGENT)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    io::copy(
        &mut resp.body_mut().as_reader(),
        &mut fs::File::create(&archive)?,
    )?;

    // tar ships with macOS, Linux and Windows 10+, and reads zips on Windows.
    let ok = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&temp.path)
        .output()
        .context("running tar")?
        .status
        .success();
    if !ok {
        bail!("could not extract {asset}");
    }
    let extracted = temp.path.join(exe.file_name().context("no exe name")?);
    let new = exe.with_extension("new");
    fs::copy(&extracted, &new).context("copying the new wowmod next to the old one")?;
    fs::rename(&exe, &old).context("moving the running wowmod aside")?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(e).context("moving the new wowmod into place");
    }
    Ok(Some(release.tag_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("v0.2.1", "0.1.3"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.3", "0.1.3"));
        assert!(!is_newer("v0.1.2", "0.1.3"));
    }
}
