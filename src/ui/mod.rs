//! The ratatui application: state, event loop and key handling.

mod browser;
mod render;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::style::Color;
use ratatui::widgets::ListState;

use crate::addon::{self, InstalledAddon};
use crate::config::{Config, DEFAULT_KEY_COLOR};
use crate::flavor::Flavor;
use crate::installer;
use crate::keymap::{Action, Keymap};
use crate::package::Package;
use crate::wow::{self, Install};
use browser::Browser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Browser,
    Installs,
    Addons,
}

/// Something that needs a yes/no, or text, before continuing.
pub enum Modal {
    Help {
        scroll: u16,
    },
    Confirm {
        title: String,
        lines: Vec<String>,
        action: ConfirmAction,
    },
    Input {
        title: String,
        value: String,
        action: InputAction,
    },
    Message {
        title: String,
        lines: Vec<String>,
    },
    /// Uninstall confirmation with a "back up first" checkbox.
    Uninstall {
        lines: Vec<String>,
        paths: Vec<PathBuf>,
        backup: bool,
    },
}

pub enum ConfirmAction {
    Install,
    Rollback { folder: String, backup: PathBuf },
}

pub enum InputAction {
    GoTo,
    AddRoot,
}

/// Result of analysing the browser selection in a background thread.
pub struct Preview {
    pub generation: u64,
    pub result: std::result::Result<Package, String>,
}

pub struct App {
    pub config: Config,
    pub keys: Keymap,
    /// Color for key names in hints, parsed from `config.key_color`.
    pub key_color: Color,
    /// Problems found while starting up, shown once in a dialog.
    pub startup_warnings: Vec<String>,
    pub browser: Browser,
    pub installs: Vec<Install>,
    pub install_state: ListState,
    /// Extra install targets toggled with Space (indices into `installs`).
    pub marked: BTreeSet<usize>,
    pub addons: Vec<InstalledAddon>,
    pub addon_state: ListState,
    pub focus: Focus,
    pub modal: Option<Modal>,
    pub status: String,
    pub preview: Option<Preview>,
    pub preview_pending: bool,
    pub preview_gen: u64,
    pub preview_rx: mpsc::Receiver<Preview>,
    pub preview_tx: mpsc::Sender<Preview>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config, keys: Keymap) -> App {
        let installs = wow::discover(&config.wow_roots);
        let browser = Browser::new(config.start_dir(), config.filter_browser);
        let (tx, rx) = mpsc::channel();
        let mut startup_warnings = Vec::new();
        let key_color = match config.key_color.trim().parse::<Color>() {
            Ok(c) => c,
            Err(_) => {
                startup_warnings.push(format!(
                    "key_color {:?} in config.toml is not a color; using {DEFAULT_KEY_COLOR}.",
                    config.key_color
                ));
                startup_warnings.push(
                    "Use a name (gray, light blue, yellow), #rrggbb, or a 0-255 index.".into(),
                );
                DEFAULT_KEY_COLOR.parse().expect("default color")
            }
        };
        let mut app = App {
            browser,
            installs,
            install_state: ListState::default(),
            marked: BTreeSet::new(),
            addons: Vec::new(),
            addon_state: ListState::default(),
            focus: Focus::Browser,
            modal: None,
            status: String::new(),
            preview: None,
            preview_pending: false,
            preview_gen: 0,
            preview_rx: rx,
            preview_tx: tx,
            should_quit: false,
            config,
            keys,
            key_color,
            startup_warnings,
        };
        if !app.installs.is_empty() {
            let idx = app
                .config
                .last_flavor
                .as_ref()
                .and_then(|f| app.installs.iter().position(|i| &i.flavor.code == f))
                .unwrap_or(0);
            app.install_state.select(Some(idx));
        } else {
            app.status = "No WoW installs found. Press a to add a World of Warcraft folder.".into();
        }
        app.refresh_addons();
        app.request_preview();
        app
    }

    pub fn current_install(&self) -> Option<&Install> {
        self.install_state
            .selected()
            .and_then(|i| self.installs.get(i))
    }

    pub fn current_flavor(&self) -> Option<&Flavor> {
        self.current_install().map(|i| &i.flavor)
    }

    /// The installs an install action targets: marked ones, else the current.
    pub fn targets(&self) -> Vec<Install> {
        if self.marked.is_empty() {
            self.current_install().cloned().into_iter().collect()
        } else {
            self.marked
                .iter()
                .filter_map(|&i| self.installs.get(i).cloned())
                .collect()
        }
    }

    pub fn current_addon(&self) -> Option<&InstalledAddon> {
        self.addon_state.selected().and_then(|i| self.addons.get(i))
    }

    pub fn refresh_installs(&mut self) {
        let keep = self.current_install().map(|i| i.path.clone());
        self.installs = wow::discover(&self.config.wow_roots);
        self.marked.clear();
        let idx = keep
            .and_then(|p| self.installs.iter().position(|i| i.path == p))
            .unwrap_or(0);
        self.install_state.select(if self.installs.is_empty() {
            None
        } else {
            Some(idx)
        });
        self.refresh_addons();
    }

    pub fn refresh_addons(&mut self) {
        let keep = self.current_addon().map(|a| a.primary.name.clone());
        self.addons = match self.current_install() {
            Some(i) => addon::scan(i),
            None => Vec::new(),
        };
        let idx = keep
            .and_then(|n| self.addons.iter().position(|a| a.primary.name == n))
            .unwrap_or(0);
        self.addon_state.select(if self.addons.is_empty() {
            None
        } else {
            Some(idx.min(self.addons.len() - 1))
        });
    }

    /// Kick off analysis of the browser selection on a worker thread.
    pub fn request_preview(&mut self) {
        self.preview_gen += 1;
        let generation = self.preview_gen;
        self.preview = None;
        let Some(entry) = self.browser.selected().cloned() else {
            self.preview_pending = false;
            return;
        };
        if !entry.is_installable() {
            self.preview_pending = false;
            return;
        }
        self.preview_pending = true;
        let flavor = self.current_flavor().cloned();
        let tx = self.preview_tx.clone();
        std::thread::spawn(move || {
            let result =
                Package::analyze(&entry.path, flavor.as_ref()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Preview { generation, result });
        });
    }

    fn poll_preview(&mut self) {
        while let Ok(p) = self.preview_rx.try_recv() {
            if p.generation == self.preview_gen {
                self.preview_pending = false;
                self.preview = Some(p);
            }
        }
    }

    fn select_install_by(&mut self, delta: isize) {
        if self.installs.is_empty() {
            return;
        }
        let cur = self.install_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.installs.len() as isize - 1) as usize;
        if Some(next) != self.install_state.selected() {
            self.install_state.select(Some(next));
            self.config.last_flavor = Some(self.installs[next].flavor.code.clone());
            self.refresh_addons();
            self.request_preview();
        }
    }

    fn select_addon_by(&mut self, delta: isize) {
        if self.addons.is_empty() {
            return;
        }
        let cur = self.addon_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.addons.len() as isize - 1) as usize;
        self.addon_state.select(Some(next));
    }

    // ----- actions -----------------------------------------------------

    fn begin_install(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            self.status = "No WoW install selected.".into();
            return;
        }
        let Some(preview) = &self.preview else {
            self.status = if self.preview_pending {
                "Still analysing, try again in a moment.".into()
            } else {
                "Select a zip file or addon folder in the browser first.".into()
            };
            return;
        };
        let pkg = match &preview.result {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Cannot install: {e}");
                return;
            }
        };
        let mut lines = Vec::new();
        for t in &targets {
            lines.push(format!("Into {}:", t.label()));
            for l in installer::preview_lines(&pkg.roots, t) {
                lines.push(format!("  {l}"));
            }
            for r in &pkg.roots {
                if r.toc.supports(&t.flavor) == Some(false) {
                    lines.push(format!(
                        "  ! {} does not list an Interface for {}",
                        r.name,
                        t.flavor.display_name()
                    ));
                }
            }
        }
        lines.push(String::new());
        lines.push(if self.config.backups {
            "Replaced folders are moved to AddOns.wowmod-backup.".into()
        } else {
            "Backups are OFF: replaced folders are deleted.".into()
        });
        self.modal = Some(Modal::Confirm {
            title: "Install?".into(),
            lines,
            action: ConfirmAction::Install,
        });
    }

    fn do_install(&mut self) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        let Ok(pkg) = preview.result else { return };
        let opts = installer::Options {
            backups: self.config.backups,
        };
        let mut msgs = Vec::new();
        for t in self.targets() {
            match installer::install(&pkg, &t, &opts) {
                Ok(rep) => msgs.push(format!(
                    "{}: {} ({} replaced)",
                    t.label(),
                    rep.installed
                        .iter()
                        .map(|r| r.folder.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    rep.replaced.len()
                )),
                Err(e) => msgs.push(format!("{}: FAILED {e:#}", t.label())),
            }
        }
        self.status = format!("Installed {}", msgs.join(" | "));
        self.refresh_addons();
        self.request_preview();
    }

    fn begin_uninstall(&mut self) {
        let Some(a) = self.current_addon() else {
            self.status = "No addon selected.".into();
            return;
        };
        let paths = a.folder_paths();
        let mut lines = vec![
            format!(
                "Remove {} from {}?",
                a.title(),
                self.current_install()
                    .map(|i| i.label())
                    .unwrap_or_default()
            ),
            String::new(),
        ];
        for n in a.folder_names() {
            lines.push(format!("  {n}"));
        }
        lines.push(String::new());
        lines.push("SavedVariables in WTF are left untouched.".into());
        self.modal = Some(Modal::Uninstall {
            lines,
            paths,
            backup: self.config.backups,
        });
    }

    fn do_uninstall(&mut self, paths: Vec<PathBuf>, backup: bool) {
        let Some(install) = self.current_install().cloned() else {
            return;
        };
        let opts = installer::Options { backups: backup };
        match installer::uninstall(&install, &paths, &opts) {
            Ok(_) => {
                self.status = format!(
                    "Removed {} folder(s) from {}{}",
                    paths.len(),
                    install.label(),
                    if backup { " (backed up)" } else { "" }
                )
            }
            Err(e) => self.status = format!("Uninstall failed: {e:#}"),
        }
        self.refresh_addons();
        self.request_preview();
    }

    fn begin_rollback(&mut self) {
        let Some(install) = self.current_install().cloned() else {
            return;
        };
        let Some(a) = self.current_addon() else {
            self.status = "No addon selected.".into();
            return;
        };
        let folder = a.primary.name.clone();
        let backups = installer::backups_for(&install, &folder);
        let Some(latest) = backups.first().cloned() else {
            self.status = format!(
                "No backups of {folder} in {}",
                installer::backup_dir(&install).display()
            );
            return;
        };
        let lines = vec![
            format!("Replace the installed {folder} with the newest backup?"),
            String::new(),
            format!("  {}", latest.display()),
            String::new(),
            "The current folder is deleted (not backed up again).".into(),
        ];
        self.modal = Some(Modal::Confirm {
            title: "Roll back?".into(),
            lines,
            action: ConfirmAction::Rollback {
                folder,
                backup: latest,
            },
        });
    }

    fn do_rollback(&mut self, folder: String, backup: PathBuf) {
        let Some(install) = self.current_install().cloned() else {
            return;
        };
        match installer::rollback(&install, &folder, &backup) {
            Ok(()) => self.status = format!("Rolled back {folder}"),
            Err(e) => self.status = format!("Rollback failed: {e:#}"),
        }
        self.refresh_addons();
    }

    fn show_addon_details(&mut self) {
        let Some(a) = self.current_addon() else {
            return;
        };
        let mut lines = vec![format!("Folder:  {}", a.primary.path.display())];
        if let Some(t) = &a.primary.toc {
            if let Some(v) = &t.author {
                lines.push(format!("Author:  {v}"));
            }
            if let Some(v) = &t.version {
                lines.push(format!("Version: {v}"));
            }
            if !t.interface.is_empty() {
                lines.push(format!(
                    "Interface: {}",
                    t.interface
                        .iter()
                        .map(|i| format!("{i} ({})", crate::flavor::interface_to_version(*i)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(v) = &t.notes {
                lines.push(format!("Notes:   {v}"));
            }
            if !t.dependencies.is_empty() {
                lines.push(format!("Requires: {}", t.dependencies.join(", ")));
            }
            if !t.optional_deps.is_empty() {
                lines.push(format!("Optional: {}", t.optional_deps.join(", ")));
            }
            for (k, v) in &t.extra {
                lines.push(format!("{k}: {v}"));
            }
        }
        if !a.members.is_empty() {
            lines.push(String::new());
            lines.push("Member folders:".into());
            for m in &a.members {
                lines.push(format!(
                    "  {}  {}",
                    m.name,
                    m.toc
                        .as_ref()
                        .and_then(|t| t.version.clone())
                        .unwrap_or_default()
                ));
            }
        }
        let installed = addon::folder_name_set(&self.addons);
        let missing = a.missing_deps(&installed);
        if !missing.is_empty() {
            lines.push(String::new());
            lines.push(format!("Missing dependencies: {}", missing.join(", ")));
        }
        self.modal = Some(Modal::Message {
            title: a.title(),
            lines,
        });
    }

    // ----- key handling --------------------------------------------------

    fn handle_key(&mut self, key: event::KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if self.modal.is_some() {
            self.handle_modal_key(key);
            return;
        }
        if let Some(action) = self.keys.global_action(&key) {
            self.handle_global(action);
            return;
        }
        let nav = self.keys.nav_action(&key);
        match self.focus {
            Focus::Browser => self.handle_browser_key(nav),
            Focus::Installs => self.handle_installs_key(nav),
            Focus::Addons => self.handle_addons_key(nav),
        }
    }

    fn handle_global(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::Help => self.modal = Some(Modal::Help { scroll: 0 }),
            Action::NextPanel => {
                self.focus = match self.focus {
                    Focus::Browser => Focus::Installs,
                    Focus::Installs => Focus::Addons,
                    Focus::Addons => Focus::Browser,
                }
            }
            Action::PrevPanel => {
                self.focus = match self.focus {
                    Focus::Browser => Focus::Addons,
                    Focus::Installs => Focus::Browser,
                    Focus::Addons => Focus::Installs,
                }
            }
            Action::PanelBrowser => self.focus = Focus::Browser,
            Action::PanelInstalls => self.focus = Focus::Installs,
            Action::PanelAddons => self.focus = Focus::Addons,
            Action::Refresh => {
                self.browser.reload();
                self.refresh_installs();
                self.request_preview();
                self.status = "Refreshed.".into();
            }
            Action::ToggleBackups => {
                self.config.backups = !self.config.backups;
                self.status = format!("Backups {}", if self.config.backups { "ON" } else { "OFF" });
            }
            Action::ToggleFilter => {
                self.config.filter_browser = !self.config.filter_browser;
                self.browser.filter = self.config.filter_browser;
                self.browser.reload();
            }
            Action::GoTo => {
                self.modal = Some(Modal::Input {
                    title: "Go to folder".into(),
                    value: self.browser.cwd.display().to_string(),
                    action: InputAction::GoTo,
                })
            }
            Action::AddRoot => {
                self.modal = Some(Modal::Input {
                    title: "Add World of Warcraft folder".into(),
                    value: String::new(),
                    action: InputAction::AddRoot,
                })
            }
            Action::Install => self.begin_install(),
            Action::Uninstall => {
                if self.focus != Focus::Browser {
                    self.focus = Focus::Addons;
                    self.begin_uninstall();
                }
            }
            Action::Rollback if self.focus != Focus::Browser => self.begin_rollback(),
            _ => {}
        }
    }

    fn handle_browser_key(&mut self, nav: Option<Action>) {
        let moved = match nav {
            Some(Action::Up) => {
                self.browser.move_by(-1);
                true
            }
            Some(Action::Down) => {
                self.browser.move_by(1);
                true
            }
            Some(Action::PageUp) => {
                self.browser.move_by(-15);
                true
            }
            Some(Action::PageDown) => {
                self.browser.move_by(15);
                true
            }
            Some(Action::First) => {
                self.browser.first();
                true
            }
            Some(Action::Last) => {
                self.browser.last();
                true
            }
            Some(Action::Back) => self.browser.up(),
            Some(Action::Forward) => self.browser.enter(),
            Some(Action::Select) => {
                let is_addon_like = self.browser.selected().is_some_and(|e| {
                    matches!(e.kind, browser::Kind::Zip | browser::Kind::AddonDir)
                });
                if is_addon_like {
                    self.begin_install();
                    false
                } else {
                    self.browser.enter()
                }
            }
            _ => false,
        };
        if moved {
            self.request_preview();
        }
    }

    fn handle_installs_key(&mut self, nav: Option<Action>) {
        match nav {
            Some(Action::Up) => self.select_install_by(-1),
            Some(Action::Down) => self.select_install_by(1),
            Some(Action::First) => self.select_install_by(-(self.installs.len() as isize)),
            Some(Action::Last) => self.select_install_by(self.installs.len() as isize),
            Some(Action::Mark) => {
                if let Some(i) = self.install_state.selected()
                    && !self.marked.remove(&i)
                {
                    self.marked.insert(i);
                }
            }
            Some(Action::Select) | Some(Action::Forward) => self.focus = Focus::Addons,
            _ => {}
        }
    }

    fn handle_addons_key(&mut self, nav: Option<Action>) {
        match nav {
            Some(Action::Up) => self.select_addon_by(-1),
            Some(Action::Down) => self.select_addon_by(1),
            Some(Action::PageUp) => self.select_addon_by(-15),
            Some(Action::PageDown) => self.select_addon_by(15),
            Some(Action::First) => self.addon_state.select(if self.addons.is_empty() {
                None
            } else {
                Some(0)
            }),
            Some(Action::Last) => self.addon_state.select(self.addons.len().checked_sub(1)),
            Some(Action::Select) => self.show_addon_details(),
            Some(Action::Back) => self.focus = Focus::Installs,
            _ => {}
        }
    }

    fn handle_modal_key(&mut self, key: event::KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        match modal {
            Modal::Help { scroll } => match self.keys.nav_action(&key) {
                Some(Action::Up) => {
                    self.modal = Some(Modal::Help {
                        scroll: scroll.saturating_sub(1),
                    })
                }
                Some(Action::Down) => {
                    self.modal = Some(Modal::Help {
                        scroll: scroll.saturating_add(1),
                    })
                }
                Some(Action::PageUp) => {
                    self.modal = Some(Modal::Help {
                        scroll: scroll.saturating_sub(10),
                    })
                }
                Some(Action::PageDown) => {
                    self.modal = Some(Modal::Help {
                        scroll: scroll.saturating_add(10),
                    })
                }
                _ => {
                    // any other key closes
                }
            },
            Modal::Message { .. } => {
                // any key closes
            }
            Modal::Confirm {
                title,
                lines,
                action,
            } => match self.keys.modal_action(&key) {
                Some(Action::Confirm) => match action {
                    ConfirmAction::Install => self.do_install(),
                    ConfirmAction::Rollback { folder, backup } => self.do_rollback(folder, backup),
                },
                Some(Action::Cancel) => self.status = "Cancelled.".into(),
                _ => {
                    self.modal = Some(Modal::Confirm {
                        title,
                        lines,
                        action,
                    })
                }
            },
            Modal::Uninstall {
                lines,
                paths,
                backup,
            } => match self.keys.modal_action(&key) {
                Some(Action::Confirm) => self.do_uninstall(paths, backup),
                Some(Action::Cancel) => self.status = "Cancelled.".into(),
                Some(Action::Toggle) => {
                    self.modal = Some(Modal::Uninstall {
                        lines,
                        paths,
                        backup: !backup,
                    })
                }
                _ => {
                    self.modal = Some(Modal::Uninstall {
                        lines,
                        paths,
                        backup,
                    })
                }
            },
            Modal::Input {
                title,
                mut value,
                action,
            } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let trimmed = value.trim().trim_matches('"').to_string();
                    match action {
                        InputAction::GoTo => {
                            if self.browser.go_to(PathBuf::from(&trimmed)) {
                                self.request_preview();
                            } else {
                                self.status = format!("Not a folder: {trimmed}");
                            }
                        }
                        InputAction::AddRoot => {
                            let p = PathBuf::from(&trimmed);
                            let found = wow::scan_root(&p);
                            if found.is_empty() {
                                self.status = format!(
                                    "No WoW flavor folders (with Interface\\AddOns) found under {trimmed}"
                                );
                            } else {
                                if !self.config.wow_roots.contains(&p) {
                                    self.config.wow_roots.push(p);
                                }
                                self.refresh_installs();
                                self.status = format!("Added {} install(s)", found.len());
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::Input {
                        title,
                        value,
                        action,
                    });
                }
                KeyCode::Char(c) => {
                    value.push(c);
                    self.modal = Some(Modal::Input {
                        title,
                        value,
                        action,
                    });
                }
                _ => {
                    self.modal = Some(Modal::Input {
                        title,
                        value,
                        action,
                    })
                }
            },
        }
    }
}

pub fn run(config: Config) -> Result<()> {
    let (keys, mut warnings) = Keymap::load();
    let mut terminal = ratatui::init();
    let mut app = App::new(config, keys);
    warnings.append(&mut app.startup_warnings);
    if !warnings.is_empty() {
        app.modal = Some(Modal::Message {
            title: "Configuration".into(),
            lines: warnings,
        });
    }
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    app.config.browse_dir = Some(app.browser.cwd.clone());
    if let Err(e) = app.config.save() {
        eprintln!("warning: could not save config: {e:#}");
    }
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        app.poll_preview();
        terminal.draw(|f| render::draw(f, app))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) => app.handle_key(k),
                Event::Paste(s) => {
                    if let Some(Modal::Input { value, .. }) = &mut app.modal {
                        value.push_str(&s);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;
    use std::fs;

    /// Build a fake WoW root + a zip and render the app headlessly.
    #[test]
    fn renders_without_panicking() {
        let temp = crate::package::TempDir::new().unwrap();
        let wow = temp.path.join("World of Warcraft").join("_retail_");
        fs::create_dir_all(wow.join("Interface").join("AddOns").join("Existing")).unwrap();
        fs::write(
            wow.join(".flavor.info"),
            "Product Flavor!STRING:0
wow
",
        )
        .unwrap();
        fs::write(
            wow.join("Interface/AddOns/Existing/Existing.toc"),
            "## Interface: 120105
## Title: Existing Addon
## Version: 3.1
## Dependencies: MissingLib
",
        )
        .unwrap();
        let dl = temp.path.join("Downloads");
        fs::create_dir_all(dl.join("SomeAddon")).unwrap();
        fs::write(
            dl.join("SomeAddon/SomeAddon.toc"),
            "## Interface: 16001
## Title: Some Addon
## Version: 9
",
        )
        .unwrap();

        let config = Config {
            wow_roots: vec![temp.path.join("World of Warcraft")],
            browse_dir: Some(dl.clone()),
            last_flavor: None,
            backups: true,
            filter_browser: true,
            ..Config::default()
        };
        let mut app = App::new(config, Keymap::default());
        // Point at our fake install explicitly (the machine may have real ones).
        let idx = app
            .installs
            .iter()
            .position(|i| i.path == wow)
            .expect("fake install detected");
        app.install_state.select(Some(idx));
        app.refresh_addons();
        assert_eq!(app.addons.len(), 1);
        assert_eq!(app.addons[0].title(), "Existing Addon");

        // Select the addon folder in the browser and wait for the preview.
        let pos = app
            .browser
            .entries
            .iter()
            .position(|e| e.name == "SomeAddon")
            .unwrap();
        app.browser.state.select(Some(pos));
        app.request_preview();
        let p = app.preview_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        app.preview = Some(p);
        app.preview_pending = false;

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        println!("{text}");
        assert!(text.contains("Browse"));
        assert!(text.contains("Existing Addon"));
        assert!(text.contains("missing MissingLib"));
        assert!(text.contains("SomeAddon"));
        assert!(text.contains("no Interface version for Retail"));

        // Open the install confirmation and render it too.
        app.focus = Focus::Browser;
        app.begin_install();
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        println!("{text}");
        assert!(text.contains("Install?"));

        // Confirm and check the folder landed.
        app.handle_modal_key(event::KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(
            wow.join("Interface/AddOns/SomeAddon/SomeAddon.toc")
                .is_file()
        );
        assert_eq!(app.addons.len(), 2);
    }

    #[test]
    fn uninstall_dialog_toggles_backup_and_removes_folder() {
        let temp = crate::package::TempDir::new().unwrap();
        let wow = temp.path.join("World of Warcraft").join("_retail_");
        let addon_dir = wow.join("Interface").join("AddOns").join("Gone");
        fs::create_dir_all(&addon_dir).unwrap();
        fs::write(
            wow.join(".flavor.info"),
            "Product Flavor!STRING:0
wow
",
        )
        .unwrap();
        fs::write(
            addon_dir.join("Gone.toc"),
            "## Title: Gone
",
        )
        .unwrap();

        let config = Config {
            wow_roots: vec![temp.path.join("World of Warcraft")],
            browse_dir: Some(temp.path.clone()),
            last_flavor: None,
            backups: true,
            filter_browser: true,
            ..Config::default()
        };
        let mut app = App::new(config, Keymap::default());
        let idx = app
            .installs
            .iter()
            .position(|i| i.path == wow)
            .expect("fake install detected");
        app.install_state.select(Some(idx));
        app.refresh_addons();
        assert_eq!(app.addons.len(), 1);
        app.focus = Focus::Addons;

        // 'u' opens the uninstall dialog with the checkbox preset from config.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert!(matches!(
            app.modal,
            Some(Modal::Uninstall { backup: true, .. })
        ));

        // Space toggles it off, and the dialog shows the unchecked box.
        app.handle_key(event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(matches!(
            app.modal,
            Some(Modal::Uninstall { backup: false, .. })
        ));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        assert!(text.contains("[ ] Back up folders"));
        assert!(text.contains("deleted permanently"));

        // Toggle back on, confirm, and the folder should be in the backup dir.
        app.handle_key(event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        assert!(buffer_text(terminal.backend()).contains("[x] Back up folders"));
        app.handle_key(event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.modal.is_none());
        assert!(!addon_dir.exists());
        assert_eq!(app.addons.len(), 0);
        let backups: Vec<_> = fs::read_dir(installer::backup_dir(&app.installs[idx]))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].path().join("Gone.toc").is_file());
        assert!(app.status.contains("backed up"));

        // 'r' now means rollback: it restores the backup.
        app.refresh_addons();
        assert!(app.addons.is_empty());
        // Rollback needs a selected addon, so no-op here; the rebinding itself is
        // covered by 'u' not having opened a rollback dialog above.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(app.modal.is_none());
    }

    #[test]
    fn custom_keymap_drives_actions_and_hints() {
        let temp = crate::package::TempDir::new().unwrap();
        let wow = temp.path.join("World of Warcraft").join("_retail_");
        let addon_dir = wow.join("Interface").join("AddOns").join("Thing");
        fs::create_dir_all(&addon_dir).unwrap();
        fs::write(wow.join(".flavor.info"), "Product Flavor!STRING:0\nwow\n").unwrap();
        fs::write(addon_dir.join("Thing.toc"), "## Title: Thing\n").unwrap();

        let config = Config {
            wow_roots: vec![temp.path.join("World of Warcraft")],
            browse_dir: Some(temp.path.clone()),
            last_flavor: None,
            backups: true,
            filter_browser: true,
            ..Config::default()
        };
        let keys = Keymap::parse("uninstall = [\"x\"]\nrollback = [\"Ctrl-z\"]\nhelp = [\"F1\"]\n")
            .unwrap();
        let mut app = App::new(config, keys);
        let idx = app
            .installs
            .iter()
            .position(|i| i.path == wow)
            .expect("fake install detected");
        app.install_state.select(Some(idx));
        app.refresh_addons();
        app.focus = Focus::Addons;

        // Status bar and help show the configured keys, not the defaults.
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        assert!(text.contains("x uninstall"), "{text}");
        assert!(text.contains("Ctrl-z rollback"), "{text}");
        assert!(text.contains("F1 help"), "{text}");

        // The old key does nothing; the new one opens the dialog.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert!(app.modal.is_none());
        app.handle_key(event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(matches!(app.modal, Some(Modal::Uninstall { .. })));
        app.handle_key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.modal.is_none());

        // '?' no longer opens help, F1 does, and the help lists the bindings.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.modal.is_none());
        app.handle_key(event::KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert!(matches!(app.modal, Some(Modal::Help { .. })));
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        assert!(text.contains("uninstall the selected addon"), "{text}");
        let line = text
            .lines()
            .find(|l| l.contains("uninstall the selected addon"))
            .unwrap();
        assert!(line.contains(" x "), "{line}");
    }

    #[test]
    fn key_color_is_parsed_and_bad_values_warn() {
        let base = Config {
            wow_roots: Vec::new(),
            browse_dir: Some(std::env::temp_dir()),
            ..Config::default()
        };
        let app = App::new(base.clone(), Keymap::default());
        assert_eq!(app.key_color, Color::Rgb(0x8a, 0x8a, 0x8a));
        assert!(app.startup_warnings.is_empty());

        let app = App::new(
            Config {
                key_color: "light blue".into(),
                ..base.clone()
            },
            Keymap::default(),
        );
        assert_eq!(app.key_color, Color::LightBlue);

        let mut app = App::new(
            Config {
                key_color: "not a color".into(),
                ..base
            },
            Keymap::default(),
        );
        assert_eq!(app.key_color, Color::Rgb(0x8a, 0x8a, 0x8a));
        assert_eq!(app.startup_warnings.len(), 2);
        assert!(app.startup_warnings[0].contains("not a color"));

        // Key names in the status bar carry the key color; descriptions do not.
        app.focus = Focus::Addons;
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let y = buf.area.height - 1;
        let row: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .join("");
        let x = row.find("u uninstall").unwrap() as u16;
        assert_eq!(buf[(x, y)].fg, Color::Rgb(0x8a, 0x8a, 0x8a));
        assert_eq!(buf[(x + 2, y)].fg, Color::DarkGray);

        // Panel titles start with their hot key in the key color too.
        let top: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol())
            .collect::<Vec<_>>()
            .join("");
        for key in ["1 Browse", "2 WoW installs"] {
            let x = top.find(key).unwrap() as u16;
            assert_eq!(buf[(x, 0)].fg, Color::Rgb(0x8a, 0x8a, 0x8a), "{key}");
        }
    }

    fn buffer_text(backend: &TestBackend) -> String {
        let buf = backend.buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }
}
