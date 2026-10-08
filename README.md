# Kito Files

> 🤖 **ALERT — This app was written in the company of AI.**
> It has been pair-programmed with a machine that never sleeps, never asks for a
> coffee break and occasionally writes a comment explaining something nobody
> asked about. Proceed with a sense of humour.

**A lightweight, native Linux file manager, written in Rust with GTK4 and libadwaita.**

Kito Files follows the classic, one-folder-at-a-time layout. It is fast, has no desktop
environment hard dependencies, and uses GTK's automatic display backend selection
for Wayland or X11. Development testing is done on Wayland.

- **App ID:** `it.kito.KitoFiles`
- **Binary:** `kito-files`
- **Version:** 0.1.0 — *early development / MVP*
- **License:** MIT
- **Platform:** Linux; tested on Wayland

---

## Table of contents

- [Features](#features)
- [Dependencies](#dependencies)
- [Installation](#installation)
- [Usage](#usage)
- [Preferences](#preferences)
- [Languages](#languages)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Architecture](#architecture)
- [Development](#development)
- [License](#license)

---

## Features

**Navigation**

- Classic view: exactly one folder on screen, with breadcrumbs in the header bar
- Three view modes: **Icons**, **Compact** and **Details**, selected through three
  side-by-side buttons in the dedicated view popover at the top right. The toolbar
  button shows the active tab's view icon
- Tabs (`AdwTabView`), each tab keeps its own folder, view mode and back/forward history
- Back / forward / up navigation, plus mouse side buttons (back = button 8, forward = button 9)
- Failed navigation leaves the current folder and back/forward history unchanged
- The path bar fills the available header space; `Ctrl+L`, the current breadcrumb,
  or an empty area of the bar opens its editor
- Asynchronous local-directory suggestions support absolute paths, `~/` and paths
  relative to the active tab. Use Up/Down to choose, Tab to complete, and Enter to
  accept a suggestion or navigate; typed URIs remain available for remote locations
- Readable local paths and breadcrumb labels preserve spaces, Unicode and special
  characters without double-encoding; failed navigation leaves location and history unchanged
- Show / hide hidden files (dotfiles) through the view popover; the active tab
  updates immediately and other tabs apply the setting when selected
- Empty-folder page when there are no visible items
- Status bar with item and selection counters

**Sidebar**

- **Places** — XDG user directories (Home, Documents, Downloads, Pictures, Music, Videos, Desktop)
- **Devices** — volumes discovered through `GVolumeMonitor`, with mounting when needed
- **Network** — network browsing entry point
- **Trash** — with dedicated context menu (empty / background actions) and an icon
  that tracks whether the Trash is empty through asynchronous polling
- Bookmarks / pins written to `~/.config/gtk-3.0/bookmarks`, the standard freedesktop
  bookmarks file. Changes made by other apps are picked up live through a directory monitor.

**File operations**

- Copy and cut prepare the clipboard; paste performs copying or moving in a
  background thread, with the result reported through a toast
- Paste follows the current system clipboard rather than an outdated internal
  selection. Failed cut operations retain the failed items for another move attempt;
  repeated paste cannot dispatch the same cut concurrently
- Name collisions get a safe suffix: `report.txt` → `report (copy).txt`, `report (copy 2).txt`
- Copying a folder into itself or a subfolder is rejected before creating the
  destination; local destination checks also resolve symbolic links
- Move to trash by default; permanent delete requires an explicit confirmation dialog
- Permanent recursive deletion removes symbolic links without traversing their targets
- Restore items from the Trash to their original path and name, preserving existing
  files through collision suffixes
- Empty the Trash (with confirmation)
- Rename (`F2`) and new folder (`Ctrl+Shift+N`), both with input validation
- Create empty files with suggested names for text, Word, spreadsheet and HTML files.
  These are empty placeholders: `.docx` and `.xlsx` files are not valid Office
  documents until created or saved in an appropriate application
- Open folders in the current tab and files with the system default application (`gio::AppInfo`)
- Properties dialog with name, location, type, size and modification time
- Context menus for files, empty space, and the Trash

**Integration**

- `HANDLES_OPEN`: `kito-files ~/Scaricati` opens the requested folder
  (a file opens its parent folder)
- Uses the system Adwaita theme through `AdwStyleManager`; icons come from the
  freedesktop icon theme (Papirus, Breeze, Adwaita… all work)
- Minimal custom CSS on top of the stock theme — no GNOME desktop required, no dconf/GSettings
- Open a detected terminal emulator in the current local folder, including a root
  shell via `sudo -s` where supported

---

## Dependencies

### Build-time

| Component | Version required | Notes |
|---|---|---|
| Rust toolchain (`rustc` + `cargo`) | stable, edition 2021 | tested on rustc 1.98 |
| `pkg-config` | any recent | used by the `-sys` crates to locate libraries |
| C compiler (`gcc`/`clang`) | any recent | for the FFI bindings |
| GTK4 development headers | **>= 4.22** | crate feature `gnome_50` |
| libadwaita development headers | **>= 1.9** | crate feature `v1_9` |
| GLib / GIO development headers | **>= 2.88** | crate feature `v2_88` |
| gdk-pixbuf development headers | any recent | pulled in by GTK4 |

**Distribution packages:**

```bash
# Fedora / RHEL
sudo dnf install gtk4-devel libadwaita-devel glib2-devel gdk-pixbuf2.0-devel pkgconf gcc

# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev libgdk-pixbuf-2.0-dev pkg-config gcc

# Arch Linux
sudo pacman -S gtk4 libadwaita glib2 gdk-pixbuf2 pkgconf gcc
```

### Runtime

| Component | Version required | Notes |
|---|---|---|
| GTK4 | >= 4.22 | Uses the available display backend automatically |
| libadwaita | >= 1.9 | widget library only — no gnome-shell/mutter dragged in |
| GLib / GIO / gdk-pixbuf | >= 2.88 | |
| Graphical session | Wayland or X11 | Tested on Wayland |
| **gvfs** | recommended | required for Trash (`trash:///`) and network backends |
| Terminal emulator / `sudo` | optional | For terminal actions / root shells |

`gvfs` is an *optional runtime* dependency: without it, local file operations still work,
but trashing and network locations will fail.

### Rust crates (declared in `Cargo.toml`)

**`kito-core`**

| Crate | Version | Purpose |
|---|---|---|
| `gio` | 0.22 (feature `v2_88`) | all file operations, no GTK dependency |
| `glib` | 0.22 | error types, paths, main-loop helpers |
| `tempfile` | 3 *(dev)* | unit tests on temporary directories |

**`kito-gtk`**

| Crate | Version | Purpose |
|---|---|---|
| `kito-core` | path dependency | file operations backend |
| `kito-i18n` | path dependency | locale detection and translated messages |
| `gtk4` | 0.11 (feature `gnome_50`) | UI toolkit bindings |
| `libadwaita` | 0.9 (feature `v1_9`) | adaptive widgets, dialogs, toasts, tabs |
| `gio` | 0.22 | actions, application, monitors |
| `glib` | 0.22 | main loop, spawning, error handling |
| `tempfile` | 3 *(dev)* | isolated configuration and operation tests |

**`kito-i18n`**

| Crate | Version | Purpose |
|---|---|---|
| `fluent-bundle` | 0.16 | Embedded messages, parameters and plurals |
| `sys-locale` | 0.3 | System locale selection using Linux locale conventions |
| `unic-langid` | 0.9 | Locale identifiers for Fluent |

---

## Installation

### 1. Clone the repository

```bash
git clone https://github.com/kerdx/kito-files.git
cd kito-files
```

### 2. Build

Development build (fast, unoptimized):

```bash
cargo build
```

Release build (LTO, stripped, size-optimized — see `[profile.release]` in `Cargo.toml`):

```bash
cargo build --release
```

The binary is produced at `target/release/kito-files`
(`target/debug/kito-files` for the development build).

### 3. Run

```bash
cargo run --release
```

Or directly:

```bash
./target/release/kito-files
# open a specific folder, or several
./target/release/kito-files ~/Documents ~/Downloads
```

GTK automatically selects the display backend for the current session.

### 4. Install (optional, per-user)

```bash
# binary
install -Dm755 target/release/kito-files ~/.local/bin/kito-files

# desktop entry + icon
install -Dm644 data/it.kito.KitoFiles.desktop ~/.local/share/applications/it.kito.KitoFiles.desktop
install -Dm644 data/it.kito.KitoFiles.svg ~/.local/share/icons/hicolor/scalable/apps/it.kito.KitoFiles.svg
```

The checked-in `.desktop` file launches the binary through `PATH`:

```ini
Exec=kito-files %U
```

Make sure `~/.local/bin` is in your session's `PATH` before launching from the
desktop menu.

Refresh the desktop database so the launcher appears:

```bash
update-desktop-database ~/.local/share/applications
gtk-update-icon-cache ~/.local/share/icons/hicolor
```

### 5. Uninstall

```bash
rm -f ~/.local/bin/kito-files
rm -f ~/.local/share/applications/it.kito.KitoFiles.desktop
rm -f ~/.local/share/icons/hicolor/scalable/apps/it.kito.KitoFiles.svg
```

User data (bookmarks are **not** removed):

```bash
rm -rf ~/.config/kito-files
```

---

## Usage

```bash
kito-files                 # open the home directory
kito-files PATH [PATH...]  # open each path in its own tab
kito-files FILE            # open the folder containing FILE
```

The header bar separates navigation, view controls and app settings:

- **App menu**, next to the name at the top left: Preferences and About Kito Files.
- **Navigation and path**: back, forward, up, clickable breadcrumbs and a path editor.
  Click its empty area or press `Ctrl+L` to edit; local directory suggestions appear
  as you type, with relative paths based on the active tab. Tab completes the selected
  suggestion (or the common prefix), and Enter accepts a suggestion or opens the path.
- **View selector**, at the top right: Icons, Compact, Details and Show Hidden Files.
  The popover stays open while changing view options.
- **New tab** button: opens the current location in another tab.

The sidebar provides places, devices, network and Trash. Context menus provide file
operations, creation, terminal actions and properties. File selection currently
supports one item at a time.

---

## Preferences

Open **Preferences** from the app menu or press `Ctrl+,`. The libadwaita preferences
dialog contains **General** and **Integration** pages:

| Setting | Choices | Behavior |
|---|---|---|
| Default view | Icons, Compact, Details | Applies to new tabs; existing tabs keep their view |
| Open items | Double click, Single click | Applies immediately to existing and new tabs; keyboard activation is unchanged |
| Language | System language, English, Italiano | Updates app translations immediately in open windows |
| Window controls | Follow system settings, Show minimize / maximize / close | Follow uses the system layout live; turning it off shows only the selected buttons on the top right, immediately in open and new windows |
| Terminal | Automatic or an installed emulator | Uses the selected emulator for terminal actions |

Defaults are Icons, double-click opening, system language, following the system
window controls (custom mode starts with all three buttons visible) and automatic
terminal detection. In custom mode buttons appear only on the top right in
minimize, maximize, close order, and all three may be hidden. Returning to Follow
system settings removes the app override without saving a static copy of the
system layout; custom choices are kept while following the system, system changes
apply live, and global desktop settings are never modified. If a saved terminal
is no longer available, detection falls back to Automatic and the preferences
dialog reports the missing choice. Root terminal actions require an emulator
that supports launching a root shell and `sudo`.

Changes are saved automatically and shared by open windows. Preferences are stored
atomically in `~/.config/kito-files/settings.conf`, or
`$XDG_CONFIG_HOME/kito-files/settings.conf` when `XDG_CONFIG_HOME` is an absolute path.
Missing or invalid values fall back to defaults; dconf and GSettings are not required.

---

## Languages

Kito Files includes **English** and **Italian** translations. By default it follows
the system message locale, including regional variants such as `it_IT` and `en_US`.
Unsupported or unavailable locales fall back to English.

A manual language choice in Preferences overrides automatic detection and is saved
between launches. Returning to **System language** resumes automatic selection.
The app updates its interface immediately, including the open preferences dialog;
standard GTK/libadwaita strings continue to follow the system locale independently.

Translations use embedded Fluent catalogs with parameters, plural forms and English
fallback. No separate translation files need to be installed alongside the binary.

To add a language, add a catalog under `crates/kito-i18n/locales/`, register it in
`crates/kito-i18n/src/lib.rs`, extend locale resolution and the preferences language
selector, and add detection, fallback, plural and parameterized-message tests.

---

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+C` | Copy |
| `Ctrl+X` | Cut |
| `Ctrl+V` | Paste |
| `Delete` | Move to Trash |
| `Shift+Delete` | Delete permanently (confirmation required) |
| `F2` | Rename selection |
| `Ctrl+L` | Edit the current path |
| `F5` / `Ctrl+R` | Reload the folder |
| `Ctrl+Shift+N` | New folder |
| `Ctrl+,` | Open Preferences |
| `Enter` | Open the selected item |
| `Esc` | Leave the path editor |
| Mouse button 8 / 9 | Back / Forward |

---

## Architecture

A Cargo workspace with three crates:

```
kito-files/
├── Cargo.toml            # workspace + release profile
├── crates/
│   ├── kito-core/        # file operations on GIO, zero GTK dependency
│   │   └── src/
│   │       ├── lib.rs          # list, copy, move, trash, delete, rename, mkdir, restore
│   │       └── bookmarks.rs    # freedesktop bookmarks (~/.config/gtk-3.0/bookmarks)
│   ├── kito-i18n/        # locale detection, Fluent catalogs and config helpers
│   │   └── locales/      # embedded en.ftl and it.ftl catalogs
│   └── kito-gtk/         # UI and desktop integration
│       └── src/
│           ├── main.rs         # window, header bar, breadcrumbs, actions, shortcuts
│           ├── file_list.rs    # Icons / Compact / Details views
│           ├── tabs.rs         # AdwTabView, per-tab state and history
│           ├── sidebar.rs      # Places, Devices, Network, Trash
│           ├── ops.rs          # clipboard, file operations, properties, dialogs
│           ├── context_menu.rs # file / background / trash popovers
│           ├── l10n.rs         # shared app translations and live language updates
│           ├── preferences/    # preference model, shared state and persistence
│           ├── preferences_dialog.rs # General / Integration preferences UI
│           └── terminal.rs     # terminal detection and launch, optional root shell
└── data/                 # .desktop entry and application icon
```

Design rules:

- **`kito-core` has no GTK dependency** — file operations use `gio::File`, while
  freedesktop bookmarks use `std::fs`. Unit tests run on temporary directories
  without a display server via `cargo test -p kito-core`.
- **`kito-gtk` owns the UI and integration** — it calls `kito-core` for file operations
  and also handles configuration-directory setup and terminal executable detection.
  Copy/move during paste, permanent deletion, restore and emptying the Trash
  run in background threads, with results delivered to the main loop. Directory
  listing, moving items to the Trash and some smaller operations still run
  synchronously on the UI thread. The sidebar checks Trash contents asynchronously.
- **No GNOME desktop coupling** — libadwaita is used as a widget library only: no
  GSettings/dconf, no libpanel, no Tracker, no desktop portals required.
- **Localization** — the `kito-i18n` crate resolves the system locale through
  `sys-locale`, supports an optional saved language override, and formats embedded
  Fluent catalogs with English fallback. No runtime catalog path or `msgfmt` is needed.
- **Freedesktop, not GNOME** — GIO/GVfs for files, freedesktop bookmarks, icon themes
  and `gio::AppInfo` for launching default applications.

---

## Development

```bash
cargo test --workspace        # backend, localization and UI logic unit tests
cargo clippy -- -D warnings    # lints
cargo fmt --check              # formatting
cargo run                     # manual testing in the current graphical session
```

Manual testing is done on Wayland (GNOME, sway, Hyprland). GTK selects the available
Wayland or X11 backend automatically; X11 has not been part of development testing.

---

## License

Kito Files is released under the **MIT License** (see [LICENSE](LICENSE)).
