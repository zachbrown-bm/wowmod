//! Parsing of World of Warcraft `.toc` files.
//!
//! A TOC file is a list of `## Key: Value` directive lines followed by the
//! Lua/XML files the addon loads. Only the directives matter to us.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::flavor::Flavor;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toc {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    /// `## Interface:` values; may be comma-delimited since 10.2.7.
    pub interface: Vec<u32>,
    pub dependencies: Vec<String>,
    pub optional_deps: Vec<String>,
    pub load_on_demand: bool,
    pub default_disabled: bool,
    /// Every `X-*` directive, with the `X-` prefix kept.
    pub extra: BTreeMap<String, String>,
}

impl Toc {
    pub fn parse_file(path: &Path) -> std::io::Result<Toc> {
        let bytes = fs::read(path)?;
        // TOC files are usually UTF-8 (often with a BOM) but can be latin-1.
        let text = String::from_utf8_lossy(&bytes);
        Ok(Toc::parse(&text))
    }

    pub fn parse(text: &str) -> Toc {
        let mut toc = Toc::default();
        let text = text.trim_start_matches('\u{feff}');
        for raw in text.lines() {
            // The client only reads the first 1024 chars of a line.
            let line: String = raw.chars().take(1024).collect();
            let line = line.trim_end();
            let Some(rest) = line.strip_prefix("##") else {
                continue;
            };
            let Some((key, value)) = rest.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "Title" => toc.title = Some(strip_color_codes(value)),
                "Notes" => toc.notes = Some(strip_color_codes(value)),
                "Author" => toc.author = Some(value.to_string()),
                "Version" => toc.version = Some(value.to_string()),
                "Interface" => {
                    toc.interface = value
                        .split(',')
                        .filter_map(|v| v.trim().parse::<u32>().ok())
                        .collect();
                }
                "OptionalDeps" | "OptionalDependencies" => {
                    toc.optional_deps.extend(split_list(value));
                }
                "LoadOnDemand" => toc.load_on_demand = value == "1",
                "DefaultState" => toc.default_disabled = value.eq_ignore_ascii_case("disabled"),
                k if k.starts_with("Dep") || k == "RequiredDeps" => {
                    toc.dependencies.extend(split_list(value));
                }
                k if k.starts_with("X-") => {
                    toc.extra.insert(k.to_string(), value.to_string());
                }
                _ => {}
            }
        }
        toc
    }

    #[allow(dead_code)]
    pub fn curse_id(&self) -> Option<u64> {
        self.extra.get("X-Curse-Project-ID")?.trim().parse().ok()
    }
    #[allow(dead_code)]
    pub fn wago_id(&self) -> Option<&str> {
        self.extra.get("X-Wago-ID").map(String::as_str)
    }
    #[allow(dead_code)]
    pub fn wowi_id(&self) -> Option<&str> {
        self.extra.get("X-WoWI-ID").map(String::as_str)
    }

    /// Does this TOC declare an Interface number for the given flavor?
    /// `None` when the TOC declares nothing (older addons often omit it).
    pub fn supports(&self, flavor: &Flavor) -> Option<bool> {
        if self.interface.is_empty() {
            return None;
        }
        Some(self.interface.iter().any(|i| flavor.matches_interface(*i)))
    }
}

fn split_list(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Remove `|cAARRGGBB` ... `|r` color escapes and `|T...|t` textures.
pub fn strip_color_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '|' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => {
                for _ in 0..8 {
                    chars.next();
                }
            }
            Some('r') => {}
            Some('T') => {
                // Skip until the closing `|t`.
                let mut prev_pipe = false;
                for n in chars.by_ref() {
                    if prev_pipe && n == 't' {
                        break;
                    }
                    prev_pipe = n == '|';
                }
            }
            Some('|') => out.push('|'),
            Some(other) => {
                out.push('|');
                out.push(other);
            }
            None => out.push('|'),
        }
    }
    out.trim().to_string()
}

/// Given a directory, list the TOC files that belong to it: `<Name>.toc` and
/// `<Name>_<Suffix>.toc` / `<Name>-<Suffix>.toc` where `Name` is the folder
/// name (case-insensitive, as on Windows).
pub fn toc_files_for_dir(dir: &Path) -> Vec<TocFile> {
    let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("toc") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.len() < dir_name.len() || !stem[..dir_name.len()].eq_ignore_ascii_case(dir_name) {
            continue;
        }
        let tail = &stem[dir_name.len()..];
        let suffix = match tail.chars().next() {
            None => None,
            Some('_') | Some('-') => Some(tail[1..].to_string()),
            Some(_) => continue, // e.g. folder "DBM" vs "DBM-Core.toc" in same dir
        };
        out.push(TocFile { path, suffix });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocFile {
    pub path: PathBuf,
    /// `None` for the plain `Name.toc`, otherwise e.g. `Mainline`, `Vanilla`.
    pub suffix: Option<String>,
}

/// Pick the TOC the client would load for `flavor`: a matching game suffix,
/// then a matching family suffix, then the plain file, then anything.
pub fn select_toc<'a>(files: &'a [TocFile], flavor: Option<&Flavor>) -> Option<&'a TocFile> {
    if let Some(flavor) = flavor {
        for wanted in flavor.toc_suffixes() {
            if let Some(f) = files.iter().find(|f| {
                f.suffix
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(wanted))
            }) {
                return Some(f);
            }
        }
    }
    files
        .iter()
        .find(|f| f.suffix.is_none())
        .or_else(|| files.first())
}

/// Read the whole directory's TOC set and merge into one view: the selected
/// TOC's fields, plus the union of every Interface number across all TOCs so
/// that flavor-support checks see every flavor the addon ships for.
pub fn read_dir_toc(dir: &Path, flavor: Option<&Flavor>) -> Option<Toc> {
    let files = toc_files_for_dir(dir);
    let selected = select_toc(&files, flavor)?;
    let mut toc = Toc::parse_file(&selected.path).ok()?;
    for f in &files {
        if f.path == selected.path {
            continue;
        }
        if let Ok(other) = Toc::parse_file(&f.path) {
            for i in other.interface {
                if !toc.interface.contains(&i) {
                    toc.interface.push(i);
                }
            }
            if toc.version.is_none() {
                toc.version = other.version;
            }
            if toc.title.is_none() {
                toc.title = other.title;
            }
        }
    }
    Some(toc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_directives() {
        let t = Toc::parse(
            "\u{feff}## Interface: 120105, 16001\n## Title: |cff00ff00Cool|r Addon\n## Version: 1.2.3\n## Dependencies: Ace3, LibStub\n## RequiredDeps: Foo\n## X-Curse-Project-ID: 3358\n## SavedVariables: X\n\nCore.lua\n",
        );
        assert_eq!(t.interface, vec![120105, 16001]);
        assert_eq!(t.title.as_deref(), Some("Cool Addon"));
        assert_eq!(t.version.as_deref(), Some("1.2.3"));
        assert_eq!(t.dependencies, vec!["Ace3", "LibStub", "Foo"]);
        assert_eq!(t.curse_id(), Some(3358));
    }

    #[test]
    fn strips_textures_and_escaped_pipes() {
        assert_eq!(
            strip_color_codes("|TInterface\\Icon:0|t Name ||x"),
            "Name |x"
        );
    }
}
