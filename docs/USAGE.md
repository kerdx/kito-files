# Using Kito Files

[Project overview](../README.md) · [Development guide](DEVELOPMENT.md)

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
