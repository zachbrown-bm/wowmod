//! Left panel: a directory browser filtered to folders and zip files.

use std::fs;
use std::path::{Path, PathBuf};

use ratatui::widgets::ListState;

use crate::package;
use crate::toc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Parent,
    Dir,
    /// A folder that directly contains `<Name>.toc`.
    AddonDir,
    Zip,
    File,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
}

impl Entry {
    /// Can this entry be analysed as an addon package?
    pub fn is_installable(&self) -> bool {
        matches!(self.kind, Kind::Dir | Kind::AddonDir | Kind::Zip)
    }
}

pub struct Browser {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub state: ListState,
    pub filter: bool,
    pub error: Option<String>,
}

impl Browser {
    pub fn new(start: PathBuf, filter: bool) -> Browser {
        let mut b = Browser {
            cwd: start,
            entries: Vec::new(),
            state: ListState::default(),
            filter,
            error: None,
        };
        b.reload();
        b
    }

    pub fn reload(&mut self) {
        let keep = self.selected().map(|e| e.path.clone());
        self.entries.clear();
        self.error = None;
        if self.cwd.parent().is_some() {
            self.entries.push(Entry {
                name: "..".into(),
                path: self.cwd.clone(),
                kind: Kind::Parent,
            });
        }
        match fs::read_dir(&self.cwd) {
            Ok(rd) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();
                for e in rd.flatten() {
                    let path = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') || name.starts_with('$') {
                        continue;
                    }
                    if path.is_dir() {
                        let kind = if toc::toc_files_for_dir(&path).is_empty() {
                            Kind::Dir
                        } else {
                            Kind::AddonDir
                        };
                        dirs.push(Entry { name, path, kind });
                    } else if package::is_zip(&path) {
                        files.push(Entry {
                            name,
                            path,
                            kind: Kind::Zip,
                        });
                    } else if !self.filter {
                        files.push(Entry {
                            name,
                            path,
                            kind: Kind::File,
                        });
                    }
                }
                dirs.sort_by_key(|e| e.name.to_lowercase());
                files.sort_by_key(|e| e.name.to_lowercase());
                self.entries.extend(dirs);
                self.entries.extend(files);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        let idx = keep
            .and_then(|p| self.entries.iter().position(|e| e.path == p))
            .unwrap_or(0);
        self.state.select(if self.entries.is_empty() {
            None
        } else {
            Some(idx.min(self.entries.len() - 1))
        });
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.entries.len() as isize - 1);
        self.state.select(Some(next as usize));
    }

    pub fn first(&mut self) {
        if !self.entries.is_empty() {
            self.state.select(Some(0));
        }
    }
    pub fn last(&mut self) {
        if !self.entries.is_empty() {
            self.state.select(Some(self.entries.len() - 1));
        }
    }

    /// Enter the selected directory (or go up for `..`). Returns true if moved.
    pub fn enter(&mut self) -> bool {
        let Some(sel) = self.selected().cloned() else {
            return false;
        };
        match sel.kind {
            Kind::Parent => self.up(),
            Kind::Dir | Kind::AddonDir => {
                self.cwd = sel.path;
                self.state.select(Some(0));
                self.reload();
                true
            }
            _ => false,
        }
    }

    pub fn up(&mut self) -> bool {
        let from = self.cwd.clone();
        let Some(parent) = self.cwd.parent().map(Path::to_path_buf) else {
            return false;
        };
        self.cwd = parent;
        self.reload();
        // Highlight the folder we came from.
        if let Some(i) = self.entries.iter().position(|e| e.path == from) {
            self.state.select(Some(i));
        }
        true
    }

    pub fn go_to(&mut self, path: PathBuf) -> bool {
        if path.is_dir() {
            self.cwd = path;
            self.state.select(Some(0));
            self.reload();
            true
        } else {
            false
        }
    }
}
