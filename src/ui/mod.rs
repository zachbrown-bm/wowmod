//! The ratatui application: state, event loop and key handling.

mod browser;
mod render;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;
use ratatui::widgets::ListState;

use crate::addon::{self, InstalledAddon};
use crate::config::{Config, DEFAULT_KEY_COLOR};
use crate::curseforge::{self, Plan};
use crate::flavor::Flavor;
use crate::installer;
use crate::keymap::{Action, KeyReview, Keymap};
use crate::manifest::Manifest;
use crate::package::Package;
use crate::wow::{self, Install};
use browser::Browser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Browser,
    Installs,
    Addons,
}

/// Screen rectangles of the three panels from the last draw. Zero-sized
/// until the first frame, so clicks before then match nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PanelAreas {
    pub browser: Rect,
    pub installs: Rect,
    pub addons: Rect,
}

impl PanelAreas {
    /// The panel drawn at this screen position, if any.
    pub fn panel_at(&self, column: u16, row: u16) -> Option<Focus> {
        let p = Position::new(column, row);
        if self.browser.contains(p) {
            Some(Focus::Browser)
        } else if self.installs.contains(p) {
            Some(Focus::Installs)
        } else if self.addons.contains(p) {
            Some(Focus::Addons)
        } else {
            None
        }
    }
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
    /// Keys bound twice in keymaps.toml: pick yours or the new default per
    /// action. Shown before anything else; saving is the only way in.
    Keys(KeyReview),
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
    CurseForge(Vec<(Install, Plan)>),
    Quit,
}

pub enum InputAction {
    GoTo,
    AddRoot,
    CurseForge,
}

/// Messages from the CurseForge worker thread.
pub enum Job {
    /// What installing or updating would do, per target install.
    Planned(Vec<(Install, Plan)>),
    Progress(String),
    Done(String),
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
    /// Where each panel was drawn last frame, so mouse clicks can focus it.
    pub panel_areas: PanelAreas,
    pub modal: Option<Modal>,
    pub status: String,
    pub preview: Option<Preview>,
    pub preview_pending: bool,
    pub preview_gen: u64,
    pub preview_rx: mpsc::Receiver<Preview>,
    pub preview_tx: mpsc::Sender<Preview>,
    /// Install was pressed while analysing; the prompt opens once it's done.
    pub install_queued: bool,
    /// A CurseForge lookup or install is running.
    pub job_busy: bool,
    pub job_rx: mpsc::Receiver<Job>,
    pub job_tx: mpsc::Sender<Job>,
    /// A finished lookup waiting for any open dialog to close.
    pub planned: Option<Vec<(Install, Plan)>>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config, keys: Keymap) -> App {
        let installs = wow::discover(&config.wow_roots);
        let browser = Browser::new(config.start_dir(), config.filter_browser);
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel();
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
            panel_areas: PanelAreas::default(),
            modal: None,
            status: String::new(),
            preview: None,
            preview_pending: false,
            preview_gen: 0,
            preview_rx: rx,
            preview_tx: tx,
            install_queued: false,
            job_busy: false,
            job_rx,
            job_tx,
            planned: None,
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
        // The selection or flavor changed, so a queued install no longer applies.
        if self.install_queued {
            self.install_queued = false;
            self.status = "Install cancelled.".into();
        }
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
        // Wait for any open dialog to close before showing the prompt.
        if self.install_queued && !self.preview_pending && self.modal.is_none() {
            self.install_queued = false;
            self.status.clear();
            self.begin_install();
        }
    }

    /// Look up on a worker thread what installing `project` needs in each
    /// target, or with `None`, what updating the mods in the manifest needs.
    fn start_plan(&mut self, project: Option<String>) {
        if self.job_busy {
            self.status = "CurseForge is still busy, try again when it finishes.".into();
            return;
        }
        let targets = self.targets();
        if targets.is_empty() {
            self.status = "No WoW install selected.".into();
            return;
        }
        self.job_busy = true;
        self.status = "Checking CurseForge ...".into();
        let tx = self.job_tx.clone();
        std::thread::spawn(move || {
            let id = match project.as_deref().map(curseforge::project_id).transpose() {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx.send(Job::Done(format!("{e:#}")));
                    return;
                }
            };
            let mut manifest = Manifest::load();
            let plans = targets
                .into_iter()
                .map(|t| {
                    let ids = match id {
                        Some(id) => vec![id],
                        None => manifest.ids_for(&t),
                    };
                    let plan = curseforge::plan(&ids, &t, &manifest);
                    (t, plan)
                })
                .collect();
            if id.is_none() {
                // Saves the entries `ids_for` dropped for uninstalled mods.
                let _ = manifest.save();
            }
            let _ = tx.send(Job::Planned(plans));
        });
    }

    fn poll_jobs(&mut self) {
        while let Ok(msg) = self.job_rx.try_recv() {
            match msg {
                Job::Planned(plans) => {
                    self.job_busy = false;
                    self.status.clear();
                    self.planned = Some(plans);
                }
                Job::Progress(s) => self.status = s,
                Job::Done(s) => {
                    self.job_busy = false;
                    self.refresh_addons();
                    self.request_preview();
                    self.status = s;
                }
            }
        }
        if self.modal.is_none()
            && let Some(plans) = self.planned.take()
        {
            self.confirm_plan(plans);
        }
    }

    fn confirm_plan(&mut self, plans: Vec<(Install, Plan)>) {
        let todo: Vec<_> = plans.iter().flat_map(|(_, p)| &p.todo).collect();
        let problems = plans.iter().any(|(_, p)| !p.problems.is_empty());
        if todo.is_empty() && !problems {
            let checked: usize = plans.iter().map(|(_, p)| p.up_to_date).sum();
            self.status = if checked == 0 {
                format!(
                    "Nothing installed from CurseForge yet. Press {} to add something.",
                    self.keys.short(Action::CurseForge)
                )
            } else {
                format!("All {checked} CurseForge mod(s) are up to date.")
            };
            return;
        }
        let mut lines = Vec::new();
        for (t, p) in &plans {
            lines.push(format!("Into {}:", t.label()));
            for m in &p.todo {
                lines.push(format!(
                    "  {}  ({})",
                    m.file.display_name,
                    size(m.file.file_length)
                ));
            }
            if p.up_to_date > 0 {
                lines.push(format!("  {} already up to date", p.up_to_date));
            }
            for e in &p.problems {
                lines.push(format!("  ! {e}"));
            }
        }
        if todo.is_empty() {
            self.modal = Some(Modal::Message {
                title: "CurseForge".into(),
                lines,
            });
            return;
        }
        let bytes = todo.iter().map(|m| m.file.file_length).sum();
        lines.push(String::new());
        lines.push(format!(
            "{} download(s), {} in total.",
            todo.len(),
            size(bytes)
        ));
        lines.push(if self.config.backups {
            "Replaced folders are moved to AddOns.wowmod-backup.".into()
        } else {
            "Backups are OFF: replaced folders are deleted.".into()
        });
        self.modal = Some(Modal::Confirm {
            title: "Install from CurseForge?".into(),
            lines,
            action: ConfirmAction::CurseForge(plans),
        });
    }

    /// Download and install the planned mods on a worker thread, reporting
    /// progress in the status bar.
    fn do_curseforge(&mut self, plans: Vec<(Install, Plan)>) {
        self.job_busy = true;
        let opts = installer::Options {
            backups: self.config.backups,
        };
        let tx = self.job_tx.clone();
        std::thread::spawn(move || {
            let mut manifest = Manifest::load();
            let total: usize = plans.iter().map(|(_, p)| p.todo.len()).sum();
            let mut n = 0;
            let mut failed = Vec::new();
            for (t, plan) in &plans {
                for m in &plan.todo {
                    n += 1;
                    let _ = tx.send(Job::Progress(format!(
                        "Installing {} ({n}/{total}, {}) ...",
                        m.file.display_name,
                        size(m.file.file_length)
                    )));
                    if let Err(e) = curseforge::install(m, t, &opts, &mut manifest) {
                        failed.push(format!("{}: {e:#}", m.file.display_name));
                    }
                }
            }
            let _ = tx.send(Job::Done(if failed.is_empty() {
                format!("Installed {total} mod(s) from CurseForge.")
            } else {
                format!("{} of {total} failed: {}", failed.len(), failed.join(" | "))
            }));
        });
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
        if self.preview_pending {
            self.install_queued = true;
            self.status = "Analysing, install will start when done.".into();
            return;
        }
        let Some(preview) = &self.preview else {
            self.status = "Select a zip file or addon folder in the browser first.".into();
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
            lines.extend(toc_detail_lines(t));
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

    /// Show problems found at startup, if any, in one dialog.
    fn show_startup_warnings(&mut self) {
        if !self.startup_warnings.is_empty() {
            self.modal = Some(Modal::Message {
                title: "Configuration".into(),
                lines: std::mem::take(&mut self.startup_warnings),
            });
        }
    }

    /// Key review keys are fixed (the user's keymap is what is being fixed).
    fn handle_key_review(&mut self, key: event::KeyEvent, mut review: KeyReview) {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            _ if ctrl_c => self.should_quit = true,
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => review.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => review.move_by(1),
            KeyCode::PageUp => review.move_by(-10),
            KeyCode::PageDown => review.move_by(10),
            KeyCode::Left | KeyCode::Char('h') => review.choose(false),
            KeyCode::Right | KeyCode::Char('l') => review.choose(true),
            KeyCode::Enter if review.keymap().conflicts().is_empty() => {
                if let Err(e) = review.save() {
                    self.startup_warnings
                        .insert(0, format!("Could not save key bindings: {e:#}"));
                }
                self.keys = review.keymap();
                self.show_startup_warnings();
                return;
            }
            _ => {}
        }
        if !self.should_quit {
            self.modal = Some(Modal::Keys(review));
        }
    }

    /// Ask before leaving. Settings are saved on the way out either way.
    fn begin_quit(&mut self) {
        self.modal = Some(Modal::Confirm {
            title: "Quit?".into(),
            lines: if self.job_busy {
                vec![
                    "Exit wowmod?".into(),
                    "A CurseForge download or install is still running and will be cut off.".into(),
                ]
            } else {
                vec!["Exit wowmod?".into()]
            },
            action: ConfirmAction::Quit,
        });
    }

    /// Details of the zip or addon folder selected in the browser, laid out
    /// like `show_addon_details` so both panels read the same way. Uses the
    /// background analysis that the preview pane already requested.
    fn show_package_details(&mut self) {
        let Some(entry) = self.browser.selected() else {
            return;
        };
        if self.preview_pending {
            self.status = format!("Still analysing {} ...", entry.name);
            return;
        }
        let Some(preview) = &self.preview else {
            return;
        };
        let pkg = match &preview.result {
            Ok(p) => p,
            Err(e) => {
                self.modal = Some(Modal::Message {
                    title: entry.name.clone(),
                    lines: vec![e.clone()],
                });
                return;
            }
        };
        let Some(primary) = pkg.roots.first() else {
            return;
        };
        let title = primary
            .toc
            .title
            .clone()
            .unwrap_or_else(|| primary.name.clone());
        let mut lines = vec![
            format!("Source:  {}", pkg.source.display()),
            format!("Folder:  {}", primary.name),
        ];
        lines.extend(toc_detail_lines(&primary.toc));
        if pkg.roots.len() > 1 {
            lines.push(String::new());
            lines.push("Member folders:".into());
            for r in &pkg.roots[1..] {
                lines.push(format!(
                    "  {}  {}",
                    r.name,
                    r.toc.version.clone().unwrap_or_default()
                ));
            }
        }
        // Dependencies the package does not bring and the target install lacks.
        let mut have = addon::folder_name_set(&self.addons);
        have.extend(pkg.roots.iter().map(|r| r.name.clone()));
        let missing: Vec<&str> = pkg
            .roots
            .iter()
            .flat_map(|r| r.toc.dependencies.iter())
            .filter(|d| !have.iter().any(|h| h.eq_ignore_ascii_case(d)))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            lines.push(String::new());
            lines.push(format!("Missing dependencies: {}", missing.join(", ")));
        }
        self.modal = Some(Modal::Message { title, lines });
    }

    // ----- mouse handling ------------------------------------------------

    /// A left click focuses the panel under the pointer. Dialogs keep the
    /// mouse out, like they do the keys, so a stray click cannot change
    /// what a confirmation applies to.
    pub fn handle_click(&mut self, column: u16, row: u16) {
        if self.modal.is_some() {
            return;
        }
        if let Some(panel) = self.panel_areas.panel_at(column, row) {
            self.focus = panel;
        }
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
            Action::Quit => self.begin_quit(),
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
            Action::CurseForge => {
                self.modal = Some(Modal::Input {
                    title: "Install from CurseForge".into(),
                    value: String::new(),
                    action: InputAction::CurseForge,
                })
            }
            Action::Update => self.start_plan(None),
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
                    self.show_package_details();
                } else if self.browser.selected().is_some() {
                    // Folders are entered with Forward only, so Enter means
                    // "details" everywhere.
                    self.status = format!(
                        "{} opens a folder; {} shows details of a zip or addon folder.",
                        self.keys.short(Action::Forward),
                        self.keys.short(Action::Select)
                    );
                }
                false
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
            Modal::Keys(review) => self.handle_key_review(key, review),
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
                    ConfirmAction::CurseForge(plans) => self.do_curseforge(plans),
                    ConfirmAction::Quit => self.should_quit = true,
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
                        InputAction::CurseForge => self.start_plan(Some(trimmed)),
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

/// "476 MB" or "9 KB".
fn size(bytes: u64) -> String {
    if bytes < 1_000_000 {
        format!("{} KB", bytes.div_ceil(1000))
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

/// The TOC fields shown in a details dialog, shared by the installed-addon
/// and the browser (zip / folder) dialogs.
fn toc_detail_lines(t: &crate::toc::Toc) -> Vec<String> {
    let mut lines = Vec::new();
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
    lines
}

pub fn run(config: Config) -> Result<()> {
    let (keys, mut warnings) = Keymap::load();
    let mut terminal = ratatui::init();
    // ratatui::init() sets up raw mode and the alternate screen but not the
    // mouse; clicks are used to focus panels. Failing to enable it is not
    // fatal, the keyboard still works.
    let mouse = execute!(std::io::stdout(), EnableMouseCapture).is_ok();
    let mut app = App::new(config, keys);
    warnings.append(&mut app.startup_warnings);
    app.startup_warnings = warnings;
    match Keymap::path() {
        Some(path) if !app.keys.conflicts().is_empty() => {
            app.modal = Some(Modal::Keys(KeyReview::new(&app.keys, path)))
        }
        _ => app.show_startup_warnings(),
    }
    let result = event_loop(&mut terminal, &mut app);
    if mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
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
        app.poll_jobs();
        terminal.draw(|f| render::draw(f, app))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) => app.handle_key(k),
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    app.handle_click(m.column, m.row)
                }
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

        // Install pressed while analysing is queued. It waits for an open
        // dialog to close, then opens the install prompt.
        assert!(app.preview_pending);
        app.begin_install();
        assert!(app.install_queued && app.modal.is_none());
        app.modal = Some(Modal::Help { scroll: 0 });
        let start = std::time::Instant::now();
        while app.preview_pending && start.elapsed() < Duration::from_secs(5) {
            app.poll_preview();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(app.modal, Some(Modal::Help { .. })));
        app.modal = None;
        app.poll_preview();
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
        assert!(!app.install_queued);
        app.modal = None;

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

        // Enter on the addon folder shows its details, not an install prompt.
        app.focus = Focus::Browser;
        app.handle_key(event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match &app.modal {
            Some(Modal::Message { title, lines }) => {
                assert_eq!(title, "Some Addon");
                let joined = lines.join("\n");
                assert!(joined.contains("Folder:  SomeAddon"), "{joined}");
                assert!(joined.contains("Version: 9"), "{joined}");
                assert!(joined.contains("Interface: 16001"), "{joined}");
            }
            _ => panic!("expected details dialog"),
        }
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        assert!(buffer_text(terminal.backend()).contains("Version: 9"));
        app.modal = None;

        // Open the install confirmation and render it too.
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

        // 'd' opens the uninstall dialog with the checkbox preset from config.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
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
        // covered by 'd' not having opened a rollback dialog above.
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

        // The default key does nothing; the configured one opens the dialog.
        app.handle_key(event::KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
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
        let x = row.find("d uninstall").unwrap() as u16;
        assert_eq!(buf[(x, y)].fg, Color::Rgb(0x8a, 0x8a, 0x8a));
        assert_eq!(buf[(x + 2, y)].fg, Color::DarkGray);

        // Panel titles start with their hot key in the key color too. Index by
        // cell, not byte: the border glyphs are multi-byte.
        let cells: Vec<String> = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        for key in ["1 Browse", "2 WoW installs"] {
            let x = (0..cells.len())
                .find(|&i| cells[i..].join("").starts_with(key))
                .unwrap() as u16;
            assert_eq!(buf[(x, 0)].fg, Color::Rgb(0x8a, 0x8a, 0x8a), "{key}");
            assert_eq!(buf[(x + 2, 0)].fg, Color::DarkGray, "{key}");
        }
    }

    #[test]
    fn quitting_asks_first() {
        let mut app = App::new(
            Config {
                wow_roots: Vec::new(),
                browse_dir: Some(std::env::temp_dir()),
                ..Config::default()
            },
            Keymap::default(),
        );
        let press = |app: &mut App, c: char| {
            app.handle_key(event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        };
        press(&mut app, 'q');
        assert!(matches!(
            app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::Quit,
                ..
            })
        ));
        assert!(!app.should_quit);
        // Saying no keeps the app open.
        press(&mut app, 'n');
        assert!(app.modal.is_none());
        assert!(!app.should_quit);
        // Esc is bound to both Quit and Cancel: it opens the prompt, then closes it.
        app.handle_key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
        app.handle_key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.modal.is_none() && !app.should_quit);
        // Saying yes quits.
        press(&mut app, 'q');
        press(&mut app, 'y');
        assert!(app.should_quit);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        app.should_quit = false;
        press(&mut app, 'q');
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        assert!(text.contains("Quit?") && text.contains("Exit wowmod?"), "{text}");
    }

    #[test]
    fn clicking_a_panel_focuses_it() {
        let mut app = App::new(
            Config {
                wow_roots: Vec::new(),
                browse_dir: Some(std::env::temp_dir()),
                ..Config::default()
            },
            Keymap::default(),
        );
        // Nothing has been drawn yet, so a click lands on no panel.
        app.handle_click(10, 10);
        assert_eq!(app.focus, Focus::Browser);

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let areas = app.panel_areas;
        assert!(areas.browser.width > 0 && areas.installs.width > 0 && areas.addons.width > 0);
        // Left column is Browser, right column splits into Installs over Addons.
        assert_eq!(areas.browser.x, 0);
        assert_eq!(areas.installs.x, areas.addons.x);
        assert_eq!(areas.installs.bottom(), areas.addons.y);

        let mid = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
        let (c, r) = mid(areas.addons);
        app.handle_click(c, r);
        assert_eq!(app.focus, Focus::Addons);
        let (c, r) = mid(areas.installs);
        app.handle_click(c, r);
        assert_eq!(app.focus, Focus::Installs);
        // Clicking the preview pane (bottom of the left column) still means Browser.
        app.handle_click(areas.browser.x + 1, areas.browser.bottom() - 2);
        assert_eq!(app.focus, Focus::Browser);
        // The status bar row below the panels belongs to none of them.
        app.focus = Focus::Addons;
        app.handle_click(5, 31);
        assert_eq!(app.focus, Focus::Addons);

        // A dialog swallows clicks.
        app.modal = Some(Modal::Message {
            title: "x".into(),
            lines: vec![],
        });
        let (c, r) = mid(areas.installs);
        app.handle_click(c, r);
        assert_eq!(app.focus, Focus::Addons);
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

    #[test]
    fn key_review_blocks_saving_until_no_key_is_bound_twice() {
        let temp = crate::package::TempDir::new().unwrap();
        let path = temp.path.join("keymaps.toml");
        // A file from before `update` existed: uninstall on u, update's default.
        fs::write(&path, "uninstall = [\"u\"]\n").unwrap();
        let keys = Keymap::parse("uninstall = [\"u\"]\n").unwrap();
        let config = Config {
            browse_dir: Some(temp.path.clone()),
            ..Config::default()
        };
        let mut app = App::new(config, keys);
        app.modal = Some(Modal::Keys(KeyReview::new(&app.keys, path.clone())));
        let press = |app: &mut App, code: KeyCode| {
            app.handle_key(event::KeyEvent::new(code, KeyModifiers::NONE))
        };

        let backend = TestBackend::new(100, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        let text = buffer_text(terminal.backend());
        assert!(
            text.contains("u is bound to both uninstall and update"),
            "{text}"
        );

        // Enter does nothing while the clash is there.
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.modal, Some(Modal::Keys(_))));

        // The cursor starts on the first clash (uninstall): take the new
        // default, then save.
        let Some(Modal::Keys(review)) = &app.modal else {
            unreachable!()
        };
        assert_eq!(review.rows[review.selected].action, Action::Uninstall);
        press(&mut app, KeyCode::Right);
        terminal.draw(|f| render::draw(f, &mut app)).unwrap();
        assert!(buffer_text(terminal.backend()).contains("No key is bound twice"));
        press(&mut app, KeyCode::Enter);
        assert!(app.modal.is_none());
        assert_eq!(app.keys.label(Action::Uninstall), "d");
        assert_eq!(app.keys.label(Action::Update), "u");
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "uninstall = [\"d\"]\n");
    }
}
