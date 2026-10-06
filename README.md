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
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Architecture](#architecture)
- [Development](#development)
- [License](#license)

---

## Features

**Navigation**

- Classic view: exactly one folder on screen, with breadcrumbs in the header bar
- Three view modes: **Icons**, **Compact** and **Details**, switchable from the menu
- Tabs (`AdwTabView`), each tab keeps its own folder, view mode and back/forward history
- Back / forward / up navigation, plus mouse side buttons (back = button 8, forward = button 9)
- Editable path bar: `Ctrl+L` (or click the current breadcrumb) to type any path or URI
- Show / hide hidden files (dotfiles)
- Status bar with item and selection counters

**Sidebar**

- **Places** — XDG user directories (Home, Documents, Downloads, Pictures, Music, Videos, Desktop)
- **Devices** — mounted volumes via `GVolumeMonitor`, with mount/unmount handling
- **Network** — network browsing entry point
- **Trash** — with dedicated context menu (empty / background actions)
- Bookmarks / pins written to `~/.config/gtk-3.0/bookmarks`, the standard freedesktop
  bookmarks file. Changes made by other apps are picked up live through a directory monitor.

**File operations**

- Copy and cut prepare the clipboard; paste performs copying or moving in a
  background thread, with the result reported through a toast
- Name collisions get a safe suffix: `report.txt` → `report (copy).txt`, `report (copy 2).txt`
- Move to trash by default; permanent delete requires an explicit confirmation dialog
- Restore items from the Trash to their original location
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
| `gtk4` | 0.11 (feature `gnome_50`) | UI toolkit bindings |
| `libadwaita` | 0.9 (feature `v1_9`) | adaptive widgets, dialogs, toasts, tabs |
| `gio` | 0.22 | actions, application, monitors |
| `glib` | 0.22 | main loop, spawning, error handling |

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

Everything else is done from the UI: the header bar (navigation + path), the menu at
the top right (view mode, hidden files, about), the left sidebar (places, devices,
network, trash) and the context menus.

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
| `Enter` | Open the selected item |
| `Esc` | Leave the path editor |
| Mouse button 8 / 9 | Back / Forward |

---

## Architecture

A Cargo workspace with two crates:

```
kito-files/
├── Cargo.toml            # workspace + release profile
├── crates/
│   ├── kito-core/        # file operations on GIO, zero GTK dependency
│   │   └── src/
│   │       ├── lib.rs          # list, copy, move, trash, delete, rename, mkdir, restore
│   │       └── bookmarks.rs    # freedesktop bookmarks (~/.config/gtk-3.0/bookmarks)
│   └── kito-gtk/         # UI only; calls into kito-core
│       └── src/
│           ├── main.rs         # window, header bar, breadcrumbs, actions, shortcuts
│           ├── file_list.rs    # Icons / Compact / Details views
│           ├── tabs.rs         # AdwTabView, per-tab state and history
│           ├── sidebar.rs      # Places, Devices, Network, Trash
│           ├── ops.rs          # clipboard, file operations, properties, dialogs
│           ├── context_menu.rs # file / background / trash popovers
│           └── terminal.rs     # terminal detection and launch, optional root shell
└── data/                 # .desktop entry and application icon
```

Design rules:

- **`kito-core` has no GTK dependency** — file operations use `gio::File`, while
  freedesktop bookmarks use `std::fs`. Unit tests run on temporary directories
  without a display server via `cargo test -p kito-core`.
- **`kito-gtk` owns the UI and integration** — it calls `kito-core` for file operations
  and also handles configuration-directory setup and terminal executable detection.
  Copy/move during paste, trash, permanent deletion, restore and emptying the Trash
  run in background threads, with results delivered to the main loop. Directory
  listing and some smaller operations still run synchronously on the UI thread.
- **No GNOME desktop coupling** — libadwaita is used as a widget library only: no
  GSettings/dconf, no libpanel, no Tracker, no desktop portals required.
- **Freedesktop, not GNOME** — GIO/GVfs for files, freedesktop bookmarks, icon themes
  and `gio::AppInfo` for launching default applications.

---

## Development

```bash
cargo test -p kito-core        # unit tests (no display needed)
cargo clippy -- -D warnings    # lints
cargo fmt --check              # formatting
cargo run                     # manual testing in the current graphical session
```

Manual testing is done on Wayland (GNOME, sway, Hyprland). GTK selects the available
Wayland or X11 backend automatically; X11 has not been part of development testing.

---

## License

Kito Files is released under the **MIT License** (see [LICENSE](LICENSE)).
