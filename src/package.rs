//! Turning "a thing the user pointed at" (a zip or a folder) into a list of
//! addon root folders ready to copy into `Interface\AddOns`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::flavor::Flavor;
use crate::toc::{self, Toc};

/// One folder that will become `Interface\AddOns\<name>`.
#[derive(Debug, Clone)]
pub struct AddonRoot {
    /// Folder name as it will appear in AddOns.
    pub name: String,
    /// Where the folder currently lives (inside a temp dir for zips).
    pub path: PathBuf,
    pub toc: Toc,
}

/// Result of analysing a source. Holds the temp dir alive while the roots
/// inside it are needed.
#[derive(Debug)]
pub struct Package {
    pub source: PathBuf,
    pub roots: Vec<AddonRoot>,
    _temp: Option<TempDir>,
}

impl Package {
    /// Analyse a zip file or directory.
    pub fn analyze(source: &Path, flavor: Option<&Flavor>) -> Result<Package> {
        if source.is_dir() {
            let roots = find_roots(source, flavor)?;
            return Ok(Package {
                source: source.to_path_buf(),
                roots,
                _temp: None,
            });
        }
        if is_zip(source) {
            let temp = TempDir::new()?;
            extract_zip(source, &temp.path)
                .with_context(|| format!("extracting {}", source.display()))?;
            let roots = find_roots(&temp.path, flavor)?;
            return Ok(Package {
                source: source.to_path_buf(),
                roots,
                _temp: Some(temp),
            });
        }
        bail!("{} is neither a folder nor a .zip file", source.display())
    }

    #[allow(dead_code)]
    pub fn names(&self) -> Vec<&str> {
        self.roots.iter().map(|r| r.name.as_str()).collect()
    }
}

pub fn is_zip(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// Walk down from `start` looking for addon roots.
///
/// Rules, applied per directory level:
/// 1. If this directory itself has a TOC named after it, it is the sole root.
/// 2. Otherwise, every child directory with a matching TOC is a root
///    (multi-folder packages like DBM-Core + DBM-GUI).
/// 3. If none were found and this directory has loose `.toc` files whose name
///    does not match the folder (a zip of the addon's *contents*), treat the
///    directory as a root renamed to the TOC's base name.
/// 4. If none were found and there is exactly one child directory (ignoring
///    junk like `__MACOSX`), descend into it (GitHub's `Repo-main/` wrapper).
pub fn find_roots(start: &Path, flavor: Option<&Flavor>) -> Result<Vec<AddonRoot>> {
    let mut dir = start.to_path_buf();
    for _depth in 0..8 {
        if let Some(root) = root_from_dir(&dir, flavor) {
            return Ok(vec![root]);
        }
        let children = child_dirs(&dir)?;
        let roots: Vec<AddonRoot> = children
            .iter()
            .filter_map(|c| root_from_dir(c, flavor))
            .collect();
        if !roots.is_empty() {
            return Ok(roots);
        }
        if let Some(root) = root_from_loose_toc(&dir, flavor) {
            return Ok(vec![root]);
        }
        match children.len() {
            1 => dir = children.into_iter().next().unwrap(),
            0 => bail!(
                "no addon found: no folder containing a matching .toc file under {}",
                start.display()
            ),
            n => bail!(
                "no addon found: {} contains {n} folders but none has a .toc file named after it",
                dir.display()
            ),
        }
    }
    bail!(
        "no addon found within 8 nested folders of {}",
        start.display()
    )
}

fn root_from_dir(dir: &Path, flavor: Option<&Flavor>) -> Option<AddonRoot> {
    let toc = toc::read_dir_toc(dir, flavor)?;
    let name = dir.file_name()?.to_str()?.to_string();
    Some(AddonRoot {
        name,
        path: dir.to_path_buf(),
        toc,
    })
}

fn root_from_loose_toc(dir: &Path, flavor: Option<&Flavor>) -> Option<AddonRoot> {
    let mut tocs: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toc")))
        .collect();
    if tocs.is_empty() {
        return None;
    }
    tocs.sort();
    // Base name = shortest stem (strips _Mainline etc.).
    let stems: Vec<String> = tocs
        .iter()
        .filter_map(|p| p.file_stem()?.to_str().map(str::to_string))
        .collect();
    let base = stems.iter().min_by_key(|s| s.len())?.clone();
    // Choose a TOC for this flavor among those sharing the base.
    let files: Vec<toc::TocFile> = tocs
        .iter()
        .zip(&stems)
        .filter(|(_, s)| s.len() >= base.len() && s[..base.len()].eq_ignore_ascii_case(&base))
        .map(|(p, s)| toc::TocFile {
            path: p.clone(),
            suffix: s[base.len()..].strip_prefix(['_', '-']).map(str::to_string),
        })
        .collect();
    let selected = toc::select_toc(&files, flavor)?;
    let toc = Toc::parse_file(&selected.path).ok()?;
    Some(AddonRoot {
        name: base,
        path: dir.to_path_buf(),
        toc,
    })
}

fn child_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "__MACOSX" || name.starts_with('.') {
            continue;
        }
        out.push(path);
    }
    out.sort();
    Ok(out)
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // enclosed_name() rejects `..` and absolute paths (zip-slip).
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(&out)?;
        io::copy(&mut entry, &mut f)?;
    }
    Ok(())
}

/// A temp directory removed on drop.
#[derive(Debug)]
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new() -> Result<TempDir> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("wowmod-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::File::create(p)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }

    fn make_zip(dest: &Path, files: &[(&str, &str)]) {
        let f = fs::File::create(dest).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn folder_that_is_itself_the_root() {
        let t = TempDir::new().unwrap();
        let d = t.path.join("MyAddon");
        write(
            &d.join("MyAddon.toc"),
            "## Interface: 120105\n## Title: My\n",
        );
        write(&d.join("MyAddon_Vanilla.toc"), "## Interface: 11509\n");
        let roots = find_roots(&d, Some(&Flavor::new("wow"))).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "MyAddon");
        assert_eq!(roots[0].toc.interface, vec![120105, 11509]);
    }

    #[test]
    fn multi_root_zip_with_junk() {
        let t = TempDir::new().unwrap();
        let z = t.path.join("dbm.zip");
        make_zip(
            &z,
            &[
                (
                    "DBM-Core/DBM-Core.toc",
                    "## Interface: 120105\n## Title: DBM\n",
                ),
                ("DBM-Core/core.lua", ""),
                (
                    "DBM-GUI/DBM-GUI_Mainline.toc",
                    "## Interface: 120105\n## Dependencies: DBM-Core\n",
                ),
                ("__MACOSX/._x", ""),
                ("README.md", "hi"),
            ],
        );
        let p = Package::analyze(&z, None).unwrap();
        assert_eq!(p.names(), vec!["DBM-Core", "DBM-GUI"]);
    }

    #[test]
    fn nested_github_zip() {
        let t = TempDir::new().unwrap();
        let z = t.path.join("gh.zip");
        make_zip(
            &z,
            &[
                ("Repo-main/README.md", ""),
                ("Repo-main/MyAddon/MyAddon.toc", "## Interface: 16001\n"),
            ],
        );
        let p = Package::analyze(&z, None).unwrap();
        assert_eq!(p.names(), vec!["MyAddon"]);
        assert!(p.roots[0].path.ends_with("MyAddon"));
    }

    #[test]
    fn loose_contents_zip_takes_toc_name() {
        let t = TempDir::new().unwrap();
        let z = t.path.join("loose.zip");
        make_zip(
            &z,
            &[
                ("Thing.toc", "## Interface: 120105\n"),
                ("Thing_Mists.toc", ""),
                ("core.lua", ""),
            ],
        );
        let p = Package::analyze(&z, None).unwrap();
        assert_eq!(p.names(), vec!["Thing"]);
    }

    #[test]
    fn no_toc_is_an_error() {
        let t = TempDir::new().unwrap();
        let z = t.path.join("bad.zip");
        make_zip(&z, &[("a/x.lua", ""), ("b/y.lua", "")]);
        assert!(Package::analyze(&z, None).is_err());
    }
}
