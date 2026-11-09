# wowmod

A terminal UI (ratatui) for installing and managing World of Warcraft addons
from zips or folders you have already downloaded.

```
cargo run --release            # open the UI
wowmod list                    # print detected installs and their addons
wowmod install DBM.zip --flavor wowf
```

## Installing

You need Rust ([rustup.rs](https://rustup.rs)). Then, from a clone of this
repo:

```
.\install.ps1        # Windows (PowerShell)
./install.sh         # macOS / Linux
```

Both run `cargo install --path . --locked`, which builds a release binary and
copies it to Cargo's bin directory (`%USERPROFILE%\.cargo\bin`, or
`$CARGO_HOME\bin`). rustup puts that directory on your `PATH`, so `wowmod`
then works from any terminal. The script warns if it is not on `PATH`.
Re-run the script after pulling to upgrade; `cargo uninstall wowmod` removes it.

## What it does

- Detects every WoW flavor on the machine (`_retail_`, `_forever_`, `_ptr_`,
  `_classic_era_`, ...) by scanning Battle.net's `product.db`, the registry,
  and common paths, then reading each flavor folder's `.flavor.info`.
  Extra folders can be added with `a` in the UI or `--root` on the CLI.
- Left panel browses folders and `.zip` files. Highlighting one analyses it in
  the background and shows what would be installed.
- Zips are extracted to a temp folder. The addon root is whichever folder
  holds `<Name>.toc` (or `<Name>_Mainline.toc` etc.), even when the zip wraps
  it in another folder (GitHub `Repo-main/`) or contains several addon folders
  (DBM-Core, DBM-GUI, ...). A zip of loose addon contents is also handled.
- Warns when the TOC's `## Interface:` numbers don't include the selected
  flavor, and flags installed addons with missing dependencies.
- Right panel lists installs and the addons in the selected one, grouped into
  logical addons (shared `X-Curse-Project-ID` or dependency + name prefix),
  with version and enabled state read from `WTF\...\AddOns.txt`.
- Install (`Enter`/`i`), uninstall (`u`), roll back to the newest backup (`r`).
  Replaced or removed folders are moved to
  `<flavor>\Interface\AddOns.wowmod-backup\<Folder>-<timestamp>` unless
  backups are turned off with `b`. The uninstall dialog has a "back up first"
  checkbox (toggle with `Space`) that starts from the `b` setting.
- `Space` marks extra installs so one zip goes to several flavors at once.

Settings persist in `%APPDATA%\wowmod\config\config.toml`. Any setting
missing from the file is added back with its default the next time the app
starts, so the file always lists everything you can change.

Key names in the status bar, dialog footers and help are drawn in
`key_color`, default `#8a8a8a`: a little lighter than the terminal's dark
gray, and a fixed RGB value so themes such as Solarized Dark (which maps
dark gray to its background) cannot hide it. Set it to any ratatui color
string: a name such as `gray`, `light blue` or `yellow`, a hex value such as
`#b0b0b0`, or a 0-255 palette index. An unrecognised value is reported at
startup and the default is used.

```toml
key_color = "light yellow"
```

## Keys

Press `?` in the app for the full list. The defaults are below.

| Key | Action |
|---|---|
| Tab / 1 2 3 | switch panel |
| Enter / i | install selected zip or addon folder |
| ← Backspace / → | parent folder / open folder |
| g | type a folder path to browse |
| f | show all files, not just zips and folders |
| Space | mark extra install targets |
| u | uninstall selected addon (confirmation has a backup checkbox) |
| r | roll back selected addon to its newest backup |
| a | add a World of Warcraft folder that was not detected |
| b | toggle backups |
| R / F5 | refresh |
| q | quit |

### Changing keys

The first run writes `%APPDATA%\wowmod\config\keymaps.toml` with every action and
its default keys. Edit the lists there and restart; the status bar, dialog
footers and help screen all show whatever you configured. Remove an action
to get its default back, or set it to `[]` to unbind it. The app never
rewrites the file, so comments survive.

```toml
# uninstall with Delete, roll back with Ctrl-z
uninstall = ["Del"]
rollback = ["Ctrl-z"]
```

Single characters are case-sensitive (`r` and `R` differ). Named keys:
`Enter Esc Tab Shift-Tab Space Up Down Left Right PgUp PgDn Home End
Backspace Del Ins F1..F12`; modifiers `Ctrl-x` and `Alt-x`. Global actions
(quit, install, uninstall, ...) are checked before panel navigation, and a
key bound to two actions is reported in a dialog at startup. A file that
cannot be parsed is reported the same way and the defaults are used.

## Layout

```
src/main.rs        CLI entry (ui, list, install)
src/toc.rs         .toc parsing, multi-TOC suffix selection
src/flavor.rs      flavor codes, Interface ranges, TOC suffixes
src/wow.rs         install discovery
src/addon.rs       scanning + grouping installed addons, enable state
src/package.rs     zip extraction + addon root detection
src/installer.rs   copy/backup/uninstall/rollback
src/config.rs      persisted settings
src/keymap.rs      configurable key bindings (keymaps.toml)
src/ui/            ratatui app, browser, rendering
```

## Roadmap: outstanding features and ideas

Nothing below is implemented yet. Items are roughly in priority order.

### Updating installed addons (the big one)

Today wowmod only installs what you have already downloaded. The goal is a
`U` key (and `wowmod update`) that checks every installed addon for a newer
release, downloads it, and reinstalls it with the usual backup.

**Identifying where an addon came from.** Three signals, in order of trust:

1. The install manifest. When wowmod installs something it should record
   `{folders, source path/URL, version, date}` in `%APPDATA%\wowmod\
   manifest.toml`. If the source was a URL we know exactly where to look.
2. TOC metadata. Most packaged addons carry provider ids in their TOC:
   `X-Curse-Project-ID`, `X-Wago-ID`, `X-WoWI-ID`, `X-Tukui-ProjectID`,
   `X-GitHub` / `X-Website` pointing at a repo. `toc.rs` already keeps every
   `X-*` key, so this is a lookup table away.
3. Fingerprinting, for addons installed by hand with no ids. Ajour and
   WowUp hash each folder (whitespace-stripped file contents, murmur2,
   sorted and hashed again) and ask CurseForge's fingerprint endpoint. This
   is the most work and depends on CurseForge API access (see below).

**Sources, and what each needs:**

| Source | API | Notes |
|---|---|---|
| GitHub Releases | public REST, unauthenticated 60 req/h, token raises it | Simplest. Pick the release asset zip; many addons ship a `release.json` (BigWigs packager) listing per-flavor zips. |
| Wago Addons | `https://addons.wago.io/api/external/...`, needs a free API token | Flavor-aware (`game_version` = retail/classic/...). Used by WowUp. |
| WoWInterface | `https://api.mmoui.com/v3/game/WOW/filedetails/<id>.json` | Public, no key. Not flavor-aware. |
| Tukui / ElvUI | `https://api.tukui.org/v1/addon/<name>` | Only ElvUI and Tukui themselves. |
| CurseForge | `https://api.curseforge.com/v1/...`, requires an approved API key | Covers the most addons but third-party managers are gated; WowUp works around it with an ad-supported build. Treat as optional, user-supplied key. |

**Update flow:** `manifest + TOC ids -> resolve provider -> fetch latest
version for the install's flavor -> compare to installed version -> download
zip to a cache dir -> run the existing `Package::analyze` + `installer::install`
path`. Version comparison should prefer the provider's release id or file date
over parsing `## Version:` strings, which are free-form.

**UI:** an "update available" column in the installed list, `U` to update
one or all, and a progress line in the status bar. Network work goes on a
worker thread like the preview analysis already does. Dependencies: `ureq` or
`reqwest` (blocking) plus `serde_json`.

**Also needed:** a per-addon "pinned" flag to skip updates, and a way to pick
the release channel (stable / beta / alpha) for providers that expose it.

### Installing by URL

Paste a GitHub, Wago, CurseForge or WoWInterface URL (`g` could accept URLs
as well as paths) and install straight from it. Same provider layer as
updating, so do these together.

### Uninstall extras

- Optionally delete `WTF\Account\*\SavedVariables\<Addon>.lua` and the
  per-character copies when uninstalling.
- Find orphaned SavedVariables for addons that are no longer installed.

### Enable / disable without launching the game

`AddOns.txt` is plain text per character, so toggling an addon on/off for one
or all characters is just a file edit. Needs care: WoW rewrites the file on
logout, so do this only while the game is not running.

### Profiles and sync

- Export the installed addon list (with provider ids) to a TOML file and
  import it on another machine or flavor: "install everything Retail has into
  Forever".
- Copy an addon from one flavor to another with one key, with the Interface
  warning shown first.

### Better matching and grouping

- Use Ajour-style fingerprints to recognise addons installed by other tools.
- Treat shared libraries (`Ace3`, `LibStub`, `LibDataBroker-1.1`) specially:
  hide them behind a toggle and warn before removing one something depends on.

### Quality of life

- Search / filter in both lists (`/`).
- Sort installed addons by name, version, enabled state, or last updated.
- Drag-and-drop a zip onto the window (terminals pass it as pasted text, so
  the paste handler can treat a path ending in `.zip` as "install this").
- Watch the Downloads folder and offer to install a newly finished zip.
- Open the addon's folder or project page from the details dialog.
- Theme configuration.
- Package for winget and a portable zip (`cargo install` works today via `install.ps1` / `install.sh`).
