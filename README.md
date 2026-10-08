# Kito Files

> 🤖 **ALERT — This app was written in the company of AI.**
> It has been pair-programmed with a machine that never sleeps, never asks for a
> coffee break and occasionally writes a comment explaining something nobody
> asked about. Proceed with a sense of humour.

**A lightweight, native Linux file manager built with Rust, GTK4 and libadwaita.**

One folder at a time, with tabs and no GNOME desktop dependency. GTK selects the
Wayland or X11 backend automatically; development testing is done on Wayland.

**Status:** 0.1.0 — early development / MVP · **License:** MIT

## Features

- Tabs, back/forward navigation and Icons, Compact or Details views.
- Editable breadcrumbs and asynchronous local path autocompletion.
- Places, devices, network locations and shared freedesktop bookmarks.
- Copy, cut, paste, rename, file/folder creation and properties.
- Trash, restore and confirmed permanent deletion, with collision handling.
- Asynchronous folder loading and bulk view updates.
- Terminal integration and preferences for view, click behavior and window controls.
- English and Italian, detected from the system or selected in Preferences.

## Build and run

Requires Linux, a stable Rust toolchain, a C compiler, `pkg-config` and development
packages for **GTK4 ≥ 4.22**, **libadwaita ≥ 1.9**, **GLib/GIO ≥ 2.88** and gdk-pixbuf.
Distribution package commands are in the [development guide](docs/DEVELOPMENT.md#dependencies).
Your distribution must provide libraries meeting these minimum versions.

```bash
git clone https://github.com/kerdx/kito-files.git
cd kito-files
cargo build --release
./target/release/kito-files
# Open specific folders
./target/release/kito-files ~/Documents ~/Downloads
```

Install **GVfs** for Trash and network backends. Terminal actions require a supported
terminal emulator; root shells also require `sudo`.

## Install

After building, install for the current user:

```bash
install -Dm755 target/release/kito-files ~/.local/bin/kito-files
install -Dm644 data/it.kito.KitoFiles.desktop ~/.local/share/applications/it.kito.KitoFiles.desktop
install -Dm644 data/it.kito.KitoFiles.svg ~/.local/share/icons/hicolor/scalable/apps/it.kito.KitoFiles.svg
update-desktop-database ~/.local/share/applications
```

Make sure `~/.local/bin` is in your session's `PATH`.
[Full installation and uninstall instructions](docs/USAGE.md#installation).

## Preferences and documentation

Open **Preferences** from the app menu or with `Ctrl+,`. Settings are saved in
`~/.config/kito-files/settings.conf`, respecting `XDG_CONFIG_HOME`. The interface
follows the system language, with English fallback, or your saved English/Italian choice.

- [User guide](docs/USAGE.md): navigation, shortcuts, preferences and installation.
- [Development guide](docs/DEVELOPMENT.md): dependencies, architecture, tests and translations.
- [Performance report](docs/perf-report.md): measurements and optimization details.

File selection currently supports one item at a time. See the user guide for
file-creation limitations and other usage details.

Released under the [MIT License](LICENSE).
