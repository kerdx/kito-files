# Developing Kito Files

[Project overview](../README.md) · [User guide](USAGE.md) · [Verification guide](VERIFICATION.md)

## Build and checks

```bash
cargo build
cargo build --release
```

```bash
./scripts/check.sh            # formatting, compilation, workspace tests, Clippy
cargo run                     # manual testing in the current graphical session
```

The check script works from any directory, stops on the first failure and uses
the lockfile for compilation, tests and Clippy. It excludes the integration test
that uses the session Trash; `--with-trash-test` includes it in a disposable test
session. GUI tests may skip without a display. Follow the
[verification guide](VERIFICATION.md) for fixtures, manual cases and coverage reporting.

Manual testing is done on Wayland (GNOME, sway, Hyprland). GTK selects the available
Wayland or X11 backend automatically; X11 has not been part of development testing.

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
| GLib / GIO | >= 2.88 | |
| gdk-pixbuf | compatible system version | Pulled in by GTK4 |
| Graphical session | Wayland or X11 | GTK chooses automatically; see the verification matrix for current coverage |
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
| `async-channel` | 2 | Worker results delivered without polling |
| `tempfile` | 3 *(dev)* | isolated configuration and operation tests |

**`kito-i18n`**

| Crate | Version | Purpose |
|---|---|---|
| `fluent-bundle` | 0.16 | Embedded messages, parameters and plurals |
| `sys-locale` | 0.3 | System locale selection using Linux locale conventions |
| `unic-langid` | 0.9 | Locale identifiers for Fluent |

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
│           ├── dnd.rs          # GDK file-list drag sources, targets and action negotiation
│           ├── context_menu.rs # file / background / trash popovers
│           ├── path_completion.rs # asynchronous local path suggestions
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
  Copy/move during paste, Trash, permanent deletion, restore, emptying the Trash,
  rename, creation and Properties reads run in background threads, with results
  delivered to the main loop through a bounded async channel (no polling).
  Directory listing also runs in a worker thread with generation-guarded,
  chunked staging and atomic view updates; only the newest navigation touches
  history and widgets. Selections are stored as URI identities and restored
  after sorting, view changes and monitor updates. Sort-order changes sort the
  cached entries on a worker and apply them in chunks. Small bookmark/configuration
  updates remain synchronous. The sidebar checks Trash contents asynchronously.
- **Live directory changes** — a confirmed directory gets a per-tab GIO monitor.
  Events are coalesced into a bounded batch and queried off the UI thread; simple
  changes update the entry cache, while ambiguous or overflowing batches trigger
  a background reconciliation. Folder URI, navigation generation and monitor
  revision guard results. Monitor installation happens only after successful
  navigation; failed navigation keeps the old monitor, and tab disposal releases
  the monitor and its pending source.
- **Drag-and-drop** — `dnd.rs` uses GDK `FileList` content and accepts file lists
  or `text/uri-list` at asynchronous targets. It captures source URIs at drag
  preparation and the destination at drop, negotiates only Copy/Move actions
  actually offered by GDK, and rejects Link. Drops reuse the operation-result
  pipeline in `ops.rs`; incoming external Move copies first and reports Move to
  the source only after complete success. Partial external transfers report no
  completed Move, so the source retains its originals. Outgoing external Move
  trashes originals only after the destination signals success. Internal moves
  track the originating drag to avoid deleting the source twice. The tab-strip
  extra-drop callback is synchronous: external Move is rejected there because
  it cannot defer acknowledgment until the async transfer finishes. Drop on the
  folder view to move external items into that directory.
- **No GNOME desktop coupling** — libadwaita is used as a widget library only: no
  GSettings/dconf, no libpanel, no Tracker, no desktop portals required.
- **Localization** — the `kito-i18n` crate resolves the system locale through
  `sys-locale`, supports an optional saved language override, and formats embedded
  Fluent catalogs with English fallback. No runtime catalog path or `msgfmt` is needed.
- **Freedesktop, not GNOME** — GIO/GVfs for files, freedesktop bookmarks, icon themes
  and `gio::AppInfo` for launching default applications.
- **GTK4 menu icons** — `GtkPopoverMenu` does not show an icon attribute beside a
  normal text menu item. Icons are shown for icon-only items or sections using an
  icon-button display hint ([GTK docs](https://docs.gtk.org/gtk4/class.PopoverMenu.html),
  [GTK explanation](https://discourse.gnome.org/t/g-menu-attribute-icon-does-not-work-in-gtk4-as-expected/7377)).
  When a row needs both an icon and a label, use a custom `gtk::Popover` row with
  `GtkImage` and `GtkLabel`, an accessible label, and the existing window action;
  see the application menu in `main.rs` and context menus in `context_menu.rs`.
  `GMenuItem::set_icon` alone is not enough for a regular text row.

## Adding translations

To add a language, add a catalog under `crates/kito-i18n/locales/`, register it in
`crates/kito-i18n/src/lib.rs`, extend locale resolution and the preferences language
selector, and add detection, fallback, plural and parameterized-message tests.

## Performance

Folder enumeration and sorting run in a worker thread. GIO enumeration uses a
cancellable; sorting is synchronous and can only be stopped at its boundary,
so cancellation during sorting discards the result after the sort finishes.
After 200 ms, an active request may show a small, cancellable overlay; a fast
request shows no indicator. The current path and listing remain paired until
the complete next model is built in 500-row chunks on the GTK main thread and
swapped in one `ListStore::splice`. History, path and content commit together
only after success. Generation checks protect each delayed indicator, worker
result and chunk from superseded navigation; closing a tab cancels its worker.
The empty page appears only after a successful listing with no visible items.

Sort order is per tab; zoom and visible Details columns are global persistent
preferences in the existing `settings.conf` model. Size, content type and modified
time are collected through GIO during enumeration; the comparator performs no
file I/O and folder sizes are not recursively calculated. Re-sorting a loaded
folder runs on a worker, then stages rows in bounded chunks while preserving URI
selection and the current scroll anchor.

Live monitor updates debounce nearby changes but have both a maximum batch size
and a periodic flush, so continuous events do not postpone updates indefinitely.
An event batch is queried once per URI and sorted/applied together; incomplete or
ambiguous batches reconcile the folder in the background. Explicit reloads and
monitor updates share generation guards to prevent duplicate or stale commits.

The app logs worker enumeration and sorting durations separately, followed by
model build/application wall time. The last figure includes chunk scheduling
and main-loop delays; it is not a pure CPU measurement. These figures separate
phases but do not establish a shorter total load or a user-perceived speedup.
The Phase 0 cached lowercase-key benchmark (5–8× faster on shuffled names)
predates natural numeric sorting and is historical; it does not describe the
current comparator. Phase 1 precomputes compact natural-name keys and keeps URI
tie-breaks, without parsing digit runs into machine integers. The new release
comparison and its variability are recorded in the [verification matrix](VERIFICATION.md);
the GTK-facing sort remains on a worker. The model still updates in bulk
`splice` calls (1001 → 1 notifications per 1000 rows). Worker results wake the
main loop once through a channel instead of polling it. The release profile stays
at `opt-level = "z"` (historical measurement: `3` adds 31% size with no speedup).

File operations return per-item operation/source/destination/status/error
records from workers. The main thread presents one localized summary and
expandable details; retries receive only failed or cancelled items. Cut
retries remain moves and are guarded by the original clipboard generation, so
a newer clipboard is not overwritten. Completion refreshes tabs showing the
captured destination rather than acting on whichever tab happens to be active.
Details and reproduction commands: [performance report](perf-report.md).
