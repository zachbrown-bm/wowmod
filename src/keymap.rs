//! User-configurable key bindings.
//!
//! Bindings live in `%APPDATA%\wowmod\config\keymaps.toml` (TOML). The file is
//! written with the defaults the first time the app runs and is only ever
//! read after that, so comments and edits survive. Every action maps to a
//! list of keys; an action missing from the file keeps its default and an
//! empty list unbinds it.
//!
//! When a new version adds or changes a default, the file can end up with a
//! key bound to two actions. The app then opens a [`KeyReview`] before
//! anything else so the user can pick, per action, their keys or the new
//! default. Saving edits only those lines in place, keeping comments.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

/// One key press, as written in the config file (`u`, `R`, `F5`, `Ctrl-c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        ev.code == self.code
            && ev.modifiers.contains(KeyModifiers::CONTROL) == self.ctrl
            && ev.modifiers.contains(KeyModifiers::ALT) == self.alt
    }
}

impl FromStr for Key {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Key> {
        let s = s.trim();
        if s.is_empty() {
            bail!("empty key name");
        }
        // A lone "-" or "+" is the key itself, not a separator.
        let parts: Vec<&str> = if s.chars().count() == 1 {
            vec![s]
        } else {
            s.split(['-', '+']).collect()
        };
        let (name, mods) = parts.split_last().unwrap();
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => ctrl = true,
                "alt" | "meta" | "a" => alt = true,
                "shift" | "s" => shift = true,
                other => bail!("unknown modifier {other:?} in {s:?}"),
            }
        }
        let lower = name.to_ascii_lowercase();
        let code = if name.chars().count() == 1 {
            let c = name.chars().next().unwrap();
            KeyCode::Char(if shift { c.to_ascii_uppercase() } else { c })
        } else if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            KeyCode::F(n)
        } else {
            match lower.as_str() {
                "enter" | "return" => KeyCode::Enter,
                "esc" | "escape" => KeyCode::Esc,
                "tab" if shift => KeyCode::BackTab,
                "tab" => KeyCode::Tab,
                "backtab" => KeyCode::BackTab,
                "space" => KeyCode::Char(' '),
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "pgup" | "pageup" => KeyCode::PageUp,
                "pgdn" | "pagedown" => KeyCode::PageDown,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                "backspace" | "bksp" => KeyCode::Backspace,
                "del" | "delete" => KeyCode::Delete,
                "ins" | "insert" => KeyCode::Insert,
                _ => bail!("unknown key {name:?}"),
            }
        };
        Ok(Key { code, ctrl, alt })
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl-")?;
        }
        if self.alt {
            f.write_str("Alt-")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::Enter => f.write_str("Enter"),
            KeyCode::Esc => f.write_str("Esc"),
            KeyCode::Tab => f.write_str("Tab"),
            KeyCode::BackTab => f.write_str("Shift-Tab"),
            KeyCode::Up => f.write_str("Up"),
            KeyCode::Down => f.write_str("Down"),
            KeyCode::Left => f.write_str("Left"),
            KeyCode::Right => f.write_str("Right"),
            KeyCode::PageUp => f.write_str("PgUp"),
            KeyCode::PageDown => f.write_str("PgDn"),
            KeyCode::Home => f.write_str("Home"),
            KeyCode::End => f.write_str("End"),
            KeyCode::Backspace => f.write_str("Backspace"),
            KeyCode::Delete => f.write_str("Del"),
            KeyCode::Insert => f.write_str("Ins"),
            other => write!(f, "{other:?}"),
        }
    }
}

impl TryFrom<String> for Key {
    type Error = anyhow::Error;
    fn try_from(s: String) -> Result<Key> {
        s.parse()
    }
}

impl From<Key> for String {
    fn from(k: Key) -> String {
        k.to_string()
    }
}

/// Everything a key can do. The name in the config file is the snake_case
/// form of the variant (`next_panel`, `toggle_backups`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // Global: checked first, whatever panel has focus.
    Quit,
    Help,
    NextPanel,
    PrevPanel,
    PanelBrowser,
    PanelInstalls,
    PanelAddons,
    Refresh,
    ToggleBackups,
    ToggleFilter,
    GoTo,
    AddRoot,
    Install,
    Uninstall,
    Rollback,
    ToggleEnabled,
    #[serde(rename = "curseforge")]
    CurseForge,
    Update,
    // Navigation inside the focused panel.
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
    Back,
    Forward,
    Select,
    Mark,
    // Dialogs.
    Confirm,
    Cancel,
    Toggle,
}

impl Action {
    pub const GLOBAL: &[Action] = &[
        Action::Quit,
        Action::Help,
        Action::NextPanel,
        Action::PrevPanel,
        Action::PanelBrowser,
        Action::PanelInstalls,
        Action::PanelAddons,
        Action::Refresh,
        Action::ToggleBackups,
        Action::ToggleFilter,
        Action::GoTo,
        Action::AddRoot,
        Action::Install,
        Action::Uninstall,
        Action::Rollback,
        Action::ToggleEnabled,
        Action::CurseForge,
        Action::Update,
    ];
    pub const NAV: &[Action] = &[
        Action::Up,
        Action::Down,
        Action::PageUp,
        Action::PageDown,
        Action::First,
        Action::Last,
        Action::Back,
        Action::Forward,
        Action::Select,
        Action::Mark,
    ];
    pub const MODAL: &[Action] = &[Action::Confirm, Action::Cancel, Action::Toggle];

    /// Config-file name (`toggle_backups`).
    pub fn name(self) -> String {
        match toml::Value::try_from(self) {
            Ok(toml::Value::String(s)) => s,
            _ => format!("{self:?}").to_lowercase(),
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::Help => "show help",
            Action::NextPanel => "focus next panel",
            Action::PrevPanel => "focus previous panel",
            Action::PanelBrowser => "focus the browser panel",
            Action::PanelInstalls => "focus the WoW installs panel",
            Action::PanelAddons => "focus the installed addons panel",
            Action::Refresh => "refresh everything",
            Action::ToggleBackups => "toggle backups on replace/uninstall",
            Action::ToggleFilter => "browser: show all files, not just zips and folders",
            Action::GoTo => "browser: type a folder path",
            Action::AddRoot => "add a World of Warcraft folder that was not detected",
            Action::Install => "install the selected zip or addon folder",
            Action::Uninstall => "uninstall the selected addon",
            Action::Rollback => "roll back the selected addon to its newest backup",
            Action::ToggleEnabled => "turn the selected addon on or off for every character",
            Action::CurseForge => {
                "install from a CurseForge link or project id, with everything it requires"
            }
            Action::Update => "update addons installed from CurseForge",
            Action::Up => "move up",
            Action::Down => "move down",
            Action::PageUp => "move up a page",
            Action::PageDown => "move down a page",
            Action::First => "jump to the first entry",
            Action::Last => "jump to the last entry",
            Action::Back => "browser: parent folder / addons: back to installs",
            Action::Forward => "browser: open folder / installs: go to addons",
            Action::Select => "browser: show details of a zip or addon folder / addons: show details",
            Action::Mark => "installs: mark extra install targets",
            Action::Confirm => "dialogs: confirm",
            Action::Cancel => "dialogs: cancel",
            Action::Toggle => "dialogs: toggle the checkbox",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keymap {
    map: BTreeMap<Action, Vec<Key>>,
    /// What the file sets, before defaults fill in the rest.
    overrides: BTreeMap<Action, Vec<Key>>,
}

fn keys(list: &[&str]) -> Vec<Key> {
    list.iter()
        .map(|s| s.parse().expect("default key"))
        .collect()
}

impl Default for Keymap {
    fn default() -> Keymap {
        let mut map = BTreeMap::new();
        let mut set = |a: Action, k: &[&str]| {
            map.insert(a, keys(k));
        };
        set(Action::Quit, &["q", "Esc", "Ctrl-c"]);
        set(Action::Help, &["?"]);
        set(Action::NextPanel, &["Tab"]);
        set(Action::PrevPanel, &["Shift-Tab"]);
        set(Action::PanelBrowser, &["1"]);
        set(Action::PanelInstalls, &["2"]);
        set(Action::PanelAddons, &["3"]);
        set(Action::Refresh, &["F5"]);
        set(Action::ToggleBackups, &["b"]);
        set(Action::ToggleFilter, &["f"]);
        set(Action::GoTo, &["g"]);
        set(Action::AddRoot, &["a"]);
        set(Action::Install, &["i"]);
        set(Action::Uninstall, &["d"]);
        set(Action::Rollback, &["r"]);
        set(Action::ToggleEnabled, &["e"]);
        set(Action::CurseForge, &["c"]);
        set(Action::Update, &["u"]);
        set(Action::Up, &["Up", "k"]);
        set(Action::Down, &["Down", "j"]);
        set(Action::PageUp, &["PgUp"]);
        set(Action::PageDown, &["PgDn"]);
        set(Action::First, &["Home"]);
        set(Action::Last, &["End"]);
        set(Action::Back, &["Left", "h", "Backspace"]);
        set(Action::Forward, &["Right", "l"]);
        set(Action::Select, &["Enter"]);
        set(Action::Mark, &["Space"]);
        set(Action::Confirm, &["y", "Enter"]);
        set(Action::Cancel, &["n", "Esc"]);
        set(Action::Toggle, &["Space", "b", "Tab"]);
        Keymap {
            map,
            overrides: BTreeMap::new(),
        }
    }
}

impl Keymap {
    pub fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "wowmod")?;
        Some(dirs.config_dir().join("keymaps.toml"))
    }

    /// Load the user's bindings, creating the file with defaults if it does
    /// not exist. Problems never stop the app: they come back as warnings and
    /// the defaults are used for whatever could not be read. Keys bound twice
    /// are not a warning; the app shows a [`KeyReview`] for those.
    pub fn load() -> (Keymap, Vec<String>) {
        let Some(path) = Keymap::path() else {
            return (
                Keymap::default(),
                vec!["No config directory; using default keys.".into()],
            );
        };
        Keymap::load_from(&path)
    }

    pub fn load_from(path: &Path) -> (Keymap, Vec<String>) {
        if !path.exists() {
            return match Keymap::default().write_template(path) {
                Ok(()) => (Keymap::default(), Vec::new()),
                Err(e) => (
                    Keymap::default(),
                    vec![format!("Could not write {}: {e:#}", path.display())],
                ),
            };
        }
        match fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))
            .and_then(|s| Keymap::parse(&s))
        {
            Ok(km) => (km, Vec::new()),
            Err(e) => (
                Keymap::default(),
                vec![
                    format!("Could not read key bindings from {}:", path.display()),
                    format!("{e:#}"),
                    "Using the default keys.".into(),
                ],
            ),
        }
    }

    /// Defaults overlaid with whatever the file sets.
    pub fn parse(text: &str) -> Result<Keymap> {
        let user: BTreeMap<Action, Vec<Key>> = toml::from_str(text)?;
        let mut km = Keymap::default();
        km.map.extend(user.clone());
        km.overrides = user;
        Ok(km)
    }

    pub fn keys(&self, a: Action) -> &[Key] {
        self.map.get(&a).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn is(&self, a: Action, ev: &KeyEvent) -> bool {
        self.keys(a).iter().any(|k| k.matches(ev))
    }

    fn first_of(&self, list: &[Action], ev: &KeyEvent) -> Option<Action> {
        list.iter().copied().find(|&a| self.is(a, ev))
    }

    pub fn global_action(&self, ev: &KeyEvent) -> Option<Action> {
        self.first_of(Action::GLOBAL, ev)
    }

    pub fn nav_action(&self, ev: &KeyEvent) -> Option<Action> {
        self.first_of(Action::NAV, ev)
    }

    pub fn modal_action(&self, ev: &KeyEvent) -> Option<Action> {
        self.first_of(Action::MODAL, ev)
    }

    /// All keys for an action, joined for display: `Left/h/Backspace`.
    pub fn label(&self, a: Action) -> String {
        let keys = self.keys(a);
        if keys.is_empty() {
            return "(unbound)".into();
        }
        keys.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Just the first key, for the status bar.
    pub fn short(&self, a: Action) -> String {
        self.keys(a)
            .first()
            .map(ToString::to_string)
            .unwrap_or_else(|| "(unbound)".into())
    }

    /// Keys bound to two actions that can fire at the same time: within the
    /// global + panel keys, or within the dialog keys.
    pub fn conflicts(&self) -> Vec<(Key, Action, Action)> {
        let mut out = Vec::new();
        let main: Vec<Action> = Action::GLOBAL.iter().chain(Action::NAV).copied().collect();
        for group in [main.as_slice(), Action::MODAL] {
            for (i, &a) in group.iter().enumerate() {
                for &b in &group[i + 1..] {
                    for k in self.keys(a) {
                        if self.keys(b).contains(k) {
                            out.push((*k, a, b));
                        }
                    }
                }
            }
        }
        out
    }

    fn write_template(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.template()).with_context(|| format!("writing {}", path.display()))
    }

    /// The config file text: a header explaining the syntax, then every
    /// action with its keys.
    pub fn template(&self) -> String {
        let mut s = String::from(
            "# wowmod key bindings.\n\
             #\n\
             # Each action lists the keys that trigger it. Edit the lists, save, and\n\
             # restart wowmod. Remove an action to get its default back; set it to []\n\
             # to unbind it.\n\
             #\n\
             # Single characters are case-sensitive (\"r\" and \"R\" differ). Named keys:\n\
             #   Enter Esc Tab Shift-Tab Space Up Down Left Right PgUp PgDn Home End\n\
             #   Backspace Del Ins F1..F12\n\
             # Modifiers: Ctrl-x, Alt-x, Shift-Tab.\n\
             #\n\
             # Global actions are checked before panel navigation, so a key bound to\n\
             # both always does the global thing.\n",
        );
        let sections: [(&str, &[Action]); 3] = [
            ("Global", Action::GLOBAL),
            ("Navigation inside the focused panel", Action::NAV),
            ("Dialogs", Action::MODAL),
        ];
        for (title, actions) in sections {
            s.push_str(&format!("\n# ---- {title} ----\n"));
            for &a in actions {
                let list = self
                    .keys(a)
                    .iter()
                    .map(|k| toml::Value::String(k.to_string()).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                s.push_str(&format!("# {}\n{} = [{list}]\n", a.describe(), a.name()));
            }
        }
        s
    }
}

/// One action in the key review: the user's keys from the file (if the file
/// sets it) and the current default.
#[derive(Debug, Clone)]
pub struct ReviewRow {
    pub action: Action,
    pub yours: Option<Vec<Key>>,
    pub default: Vec<Key>,
    pub use_default: bool,
}

impl ReviewRow {
    pub fn keys(&self) -> &[Key] {
        match &self.yours {
            Some(k) if !self.use_default => k,
            _ => &self.default,
        }
    }
}

/// Every action with the user's keys next to the defaults, shown when the
/// file binds a key twice. Saving is only allowed once no key is bound twice.
#[derive(Debug, Clone)]
pub struct KeyReview {
    pub rows: Vec<ReviewRow>,
    pub selected: usize,
    pub path: PathBuf,
}

impl KeyReview {
    /// Starts from the user's keys; actions the file does not set use the
    /// default, as they do today.
    pub fn new(km: &Keymap, path: PathBuf) -> KeyReview {
        let defaults = Keymap::default();
        let rows: Vec<ReviewRow> = Action::GLOBAL
            .iter()
            .chain(Action::NAV)
            .chain(Action::MODAL)
            .map(|&a| {
                let yours = km.overrides.get(&a).cloned();
                ReviewRow {
                    action: a,
                    use_default: yours.is_none(),
                    yours,
                    default: defaults.keys(a).to_vec(),
                }
            })
            .collect();
        // Start on the first action with a clash.
        let clashes = km.conflicts();
        let selected = rows
            .iter()
            .position(|r| clashes.iter().any(|c| c.1 == r.action || c.2 == r.action))
            .unwrap_or(0);
        KeyReview {
            rows,
            selected,
            path,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Pick the default (`true`) or the user's keys for the selected action.
    /// Actions the file does not set only have the default.
    pub fn choose(&mut self, use_default: bool) {
        if let Some(r) = self.rows.get_mut(self.selected)
            && r.yours.is_some()
        {
            r.use_default = use_default;
        }
    }

    /// Write the choices back to the file, changing only the actions where
    /// the user took the new default. Comments and everything else in the
    /// file stay as they are.
    pub fn save(&self) -> Result<()> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut doc: toml_edit::DocumentMut = text.parse()?;
        for r in self.rows.iter().filter(|r| r.yours.is_some() && r.use_default) {
            let keys: toml_edit::Array = r.default.iter().map(|k| k.to_string()).collect();
            match doc.get_mut(&r.action.name()).and_then(|i| i.as_value_mut()) {
                // Keep any trailing comment on the line.
                Some(v) => {
                    let decor = v.decor().clone();
                    *v = keys.into();
                    *v.decor_mut() = decor;
                }
                None => doc[&r.action.name()] = toml_edit::value(keys),
            }
        }
        fs::write(&self.path, doc.to_string())
            .with_context(|| format!("writing {}", self.path.display()))
    }

    /// The bindings as currently chosen.
    pub fn keymap(&self) -> Keymap {
        let map: BTreeMap<Action, Vec<Key>> = self
            .rows
            .iter()
            .map(|r| (r.action, r.keys().to_vec()))
            .collect();
        Keymap {
            overrides: map.clone(),
            map,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn parses_and_prints_keys() {
        for name in [
            "u",
            "R",
            "?",
            "F5",
            "Enter",
            "Esc",
            "Tab",
            "Shift-Tab",
            "Space",
            "Up",
            "PgDn",
            "Backspace",
            "Del",
            "Ctrl-c",
            "Alt-x",
        ] {
            let k: Key = name.parse().unwrap();
            assert_eq!(k.to_string(), name, "round trip of {name}");
        }
        assert_eq!("ctrl+C".parse::<Key>().unwrap().to_string(), "Ctrl-C");
        assert_eq!("shift-r".parse::<Key>().unwrap().to_string(), "R");
        assert_eq!("-".parse::<Key>().unwrap().code, KeyCode::Char('-'));
        assert!("Bogus".parse::<Key>().is_err());
        assert!("Hyper-x".parse::<Key>().is_err());
    }

    #[test]
    fn matching_ignores_shift_but_not_ctrl() {
        let r: Key = "R".parse().unwrap();
        assert!(r.matches(&ev(KeyCode::Char('R'), KeyModifiers::SHIFT)));
        assert!(!r.matches(&ev(KeyCode::Char('r'), KeyModifiers::NONE)));
        let c: Key = "Ctrl-c".parse().unwrap();
        assert!(c.matches(&ev(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!c.matches(&ev(KeyCode::Char('c'), KeyModifiers::NONE)));
    }

    #[test]
    fn template_round_trips_to_defaults() {
        let km = Keymap::parse(&Keymap::default().template()).unwrap();
        for &a in Action::GLOBAL
            .iter()
            .chain(Action::NAV)
            .chain(Action::MODAL)
        {
            assert_eq!(km.keys(a), Keymap::default().keys(a), "{}", a.name());
        }
        assert!(km.conflicts().is_empty());
    }

    #[test]
    fn user_overrides_apply_and_missing_keep_defaults() {
        let km = Keymap::parse("uninstall = [\"x\", \"Del\"]\nrefresh = []\n").unwrap();
        assert_eq!(km.label(Action::Uninstall), "x/Del");
        assert_eq!(km.label(Action::Refresh), "(unbound)");
        assert_eq!(km.label(Action::Rollback), "r");
        assert!(km.is(Action::Uninstall, &ev(KeyCode::Delete, KeyModifiers::NONE)));
        assert!(!km.is(
            Action::Uninstall,
            &ev(KeyCode::Char('u'), KeyModifiers::NONE)
        ));
        assert_eq!(
            km.global_action(&ev(KeyCode::F(5), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(Keymap::parse("uninstall = [\"NoSuchKey\"]").is_err());
        assert!(Keymap::parse("no_such_action = [\"x\"]").is_err());
        assert!(Keymap::parse("uninstall = \"u\"").is_err());
    }

    #[test]
    fn reports_conflicts() {
        let km = Keymap::parse("rollback = [\"d\"]\nmark = [\"i\"]\n").unwrap();
        let c = km.conflicts();
        let d: Key = "d".parse().unwrap();
        let i: Key = "i".parse().unwrap();
        assert!(
            c.contains(&(d, Action::Uninstall, Action::Rollback)),
            "{c:?}"
        );
        assert!(c.contains(&(i, Action::Install, Action::Mark)), "{c:?}");
    }

    #[test]
    fn review_resolves_a_new_default_clashing_with_an_old_file() {
        // A file from before `update` existed: uninstall on u, which is now
        // update's default.
        let km = Keymap::parse("uninstall = [\"u\"]\nquit = [\"q\"]\n").unwrap();
        assert_eq!(km.conflicts().len(), 1);
        let mut review = KeyReview::new(&km, PathBuf::from("keymaps.toml"));
        assert_eq!(review.keymap().conflicts().len(), 1);

        // update is not in the file, so only its default can be picked.
        let update = review
            .rows
            .iter()
            .position(|r| r.action == Action::Update)
            .unwrap();
        review.selected = update;
        review.choose(false);
        assert!(review.rows[update].use_default);

        // Taking the new default for uninstall clears the clash.
        let uninstall = review
            .rows
            .iter()
            .position(|r| r.action == Action::Uninstall)
            .unwrap();
        review.selected = uninstall;
        review.choose(true);
        let saved = review.keymap();
        assert!(saved.conflicts().is_empty());
        assert_eq!(saved.label(Action::Uninstall), "d");
        assert_eq!(saved.label(Action::Quit), "q");

    }

    #[test]
    fn review_save_only_changes_the_picked_lines() {
        let temp = crate::package::TempDir::new().unwrap();
        let path = temp.path.join("keymaps.toml");
        let text = "# my keys\nquit = [\"q\"] # just q\n\n# old uninstall\nuninstall = [\"u\"] # muscle memory\n";
        fs::write(&path, text).unwrap();
        let km = Keymap::parse(text).unwrap();
        let mut review = KeyReview::new(&km, path.clone());
        assert_eq!(review.rows[review.selected].action, Action::Uninstall);
        review.choose(true);
        review.save().unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# my keys\nquit = [\"q\"] # just q\n\n# old uninstall\nuninstall = [\"d\"] # muscle memory\n"
        );
    }

    #[test]
    fn load_creates_the_file_then_reads_it() {
        let temp = crate::package::TempDir::new().unwrap();
        let path = temp.path.join("sub").join("keymaps.toml");
        let (km, warnings) = Keymap::load_from(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(path.is_file());
        assert_eq!(km.label(Action::Uninstall), "d");

        fs::write(&path, "uninstall = [\"Del\"]\n").unwrap();
        let (km, warnings) = Keymap::load_from(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(km.label(Action::Uninstall), "Del");

        fs::write(&path, "this is not toml").unwrap();
        let (km, warnings) = Keymap::load_from(&path);
        assert!(!warnings.is_empty());
        assert_eq!(km.label(Action::Uninstall), "d");
    }
}
