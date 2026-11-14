# wowmod

A terminal UI (ratatui) for installing and managing World of Warcraft addons
from zips or folders you have already downloaded.

```
cargo run --release            # open the UI
wowmod list                    # print detected installs and their addons
wowmod install DBM.zip --flavor wowf
wowmod update                  # update wowmod to the newest release
```

## Installing

### Users (no Rust needed)

Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/zachbrown-bm/wowmod/main/install.ps1 | iex
```

macOS / Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/zachbrown-bm/wowmod/main/install.sh | sh
```

This downloads the latest [GitHub release](https://github.com/zachbrown-bm/wowmod/releases),
puts the binary in `%LOCALAPPDATA%\Programs\wowmod` (Windows) or `~/.local/bin`
(macOS / Linux), and adds that folder to your `PATH` on Windows (open a new
terminal afterwards). On macOS / Linux the script warns if `~/.local/bin` is
not on your `PATH`; add `export PATH="$HOME/.local/bin:$PATH"` to your shell
profile. `wowmod --version` shows what is installed.

wowmod updates itself: each time it runs (the UI, `list` or `install`) it
checks the latest GitHub release in the background, and if it is newer,
swaps the binary in place and says so. The new version runs from the next
start. `wowmod update` does the same on demand. Set `auto_update = false`
in `config.toml` to turn off the automatic check. Builds run from a cargo
`target` folder never update themselves. From a clone you can also run `.\install.ps1 -Version v0.1.0` to
pin a version. You can always grab the zip from the releases page by hand
instead.

To uninstall, delete the install folder and (on Windows) remove it from your
user `PATH` under System Properties > Environment Variables. wowmod's own
settings live in `%APPDATA%\wowmod` and are left alone.

### Developers

Build, then install the binary you just built over whatever is installed:

```
cargo build --release
.\install.ps1 -Local        # Windows
./install.sh --local        # macOS / Linux
```

Same install folder as the release build, so the dev build replaces it.

### Cutting a release

Push a version tag and the `Release` workflow builds Windows, macOS (Apple Silicon) and Linux
binaries and attaches them to a GitHub Release:

```
git tag v0.1.0
git push origin v0.1.0
```

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
- Install (`i`), uninstall (`d`), roll back to the newest backup (`r`). `Enter`
  on a zip or addon folder shows the same details dialog as an installed addon.
  Replaced or removed folders are moved to
  `<flavor>\Interface\AddOns.wowmod-backup\<Folder>-<timestamp>` unless
  backups are turned off with `b`. The uninstall dialog has a "back up first"
  checkbox (toggle with `Space`) that starts from the `b` setting.
- `Space` marks extra installs so one zip goes to several flavors at once.
- `e` turns the selected addon (and its sibling folders) on or off for every
  character, by editing each `AddOns.txt`. Mixed or off turns it on; on turns
  it off. Refused while World of Warcraft is running, because the game
  rewrites those files on logout and would undo the change.
- `c` installs from CurseForge: paste a `curseforge.com/projects/<id>` link
  or the Project ID (an addon page link is looked up through cfwidget, which
  does not know every addon). The addon and everything it requires is
  installed, recursively. This is what makes "install" / meta packages such
  as Spoken: English work: their zip is an empty stub and the real addons are
  listed as required dependencies on CurseForge. A dialog lists every mod and
  its download size before anything is downloaded.
- `u` updates what was installed with `c` in the selected (or marked)
  installs: newer files and any newly required addons, skipping addons you
  have since uninstalled or replaced with `i` (the install dialog warns when
  a zip replaces a CurseForge addon). Addons `u` keeps updated show an
  orange `cf` tag in the installed list. The newest stable file for the flavor wins, falling
  back to a beta when there is no stable one. What was installed is recorded
  in `manifest.toml` next to `config.toml`. Downloads run in the background
  with progress in the status bar. Both use the JSON endpoints behind
  www.curseforge.com, which need no API key but are undocumented and may
  change.

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

Press `?` in the app for the full list. The defaults are below. A left click
on a panel focuses it as well. Because the app captures the mouse, hold
`Shift` while dragging to select terminal text.

| Key | Action |
|---|---|
| Tab / 1 2 3 | switch panel |
| i | install selected zip or addon folder |
| Enter | show details of the selected zip / addon folder |
| ← Backspace / → | parent folder / open folder |
| g | type a folder path to browse |
| f | show all files, not just zips and folders |
| Space | mark extra install targets |
| d | uninstall selected addon (confirmation has a backup checkbox) |
| r | roll back selected addon to its newest backup |
| e | turn selected addon on / off for every character |
| c | install from a CurseForge link or project id, with everything it requires |
| u | update addons installed from CurseForge |
| a | add a World of Warcraft folder that was not detected |
| b | toggle backups |
| F5 | refresh |
| q | quit (asks for confirmation) |

### Changing keys

The first run writes `%APPDATA%\wowmod\config\keymaps.toml` with every action and
its default keys. Edit the lists there and restart; the status bar, dialog
footers and help screen all show whatever you configured. Remove an action
to get its default back, or set it to `[]` to unbind it. The app never
rewrites the file; the key review below only changes the lines you pick,
so comments survive.

```toml
# uninstall with Delete, roll back with Ctrl-z
uninstall = ["Del"]
rollback = ["Ctrl-z"]
```

Single characters are case-sensitive (`r` and `R` differ). Named keys:
`Enter Esc Tab Shift-Tab Space Up Down Left Right PgUp PgDn Home End
Backspace Del Ins F1..F12`; modifiers `Ctrl-x` and `Alt-x`. Global actions
(quit, install, uninstall, ...) are checked before panel navigation. A file
that cannot be parsed is reported in a dialog at startup and the defaults
are used.

If a key ends up bound to two actions (usually because a new version added
or changed a default that one of your keys already uses), the app opens a
key review before anything else. It lists every action with your keys and
the new default side by side: `Left` keeps yours, `Right` takes the new
default. `Enter` saves once no key is bound twice, changing only the lines
where you took the new default, and `Esc` quits without changing anything.

## Layout

```
src/main.rs        CLI entry (ui, list, install, update)
src/toc.rs         .toc parsing, multi-TOC suffix selection
src/flavor.rs      flavor codes, Interface ranges, TOC suffixes
src/wow.rs         install discovery
src/addon.rs       scanning + grouping installed addons, enable state
src/package.rs     zip extraction + addon root detection
src/installer.rs   copy/backup/uninstall/rollback
src/curseforge.rs  CurseForge add/update with dependency resolution
src/manifest.rs    what was installed from CurseForge (manifest.toml)
src/config.rs      persisted settings
src/selfupdate.rs  updating wowmod itself from GitHub releases
src/keymap.rs      configurable key bindings (keymaps.toml)
src/ui/            ratatui app, browser, rendering
```

## Roadmap: outstanding features and ideas

Nothing below is implemented yet. Items are roughly in priority order.

### Updating installed addons (the big one)

`c` / `u` cover CurseForge addons installed through wowmod (see above). The
goal is for `u` to check every installed addon for a newer release, including
ones from other providers and ones installed by other tools, download it, and
reinstall it with the usual backup.

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

### Per-character on / off

`e` switches an addon for every character at once. Picking single
characters would need a character list in the UI.

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
- Package for winget (GitHub releases and install scripts exist; see Installing).
