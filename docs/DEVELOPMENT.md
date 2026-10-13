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
│           ├── job_center.rs   # F08 operation registry: job ids, states, progress, cancel
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

### Operation jobs (F08)

`operation_queue.rs` stays the single mutation serializer; `job_center.rs` adds
a registry wrapped around it. `Ctx::submit_job` (in `ops.rs`) registers a job
with a stable id, captured sources/destination, then runs the mutation worker
with the job's GIO `Cancellable` propagated to the cancellable variants of
`kito-core` (copy, move, trash, delete, restore, empty). The worker reports
progress via atomics only: really attempted items against a known batch total,
plus measured current-file bytes from the GIO copy callback. Unknown totals
stay indeterminate; no recursive pre-scan invents percentages.

Every submitted FIFO task holds the shared `GApplication` from enqueue through
its main-context completion and cleanup. Closing the last window therefore
does not terminate already-submitted jobs; the application remains alive until
their completions have run. A closed tab or window is not updated by a later UI
completion, and this is not a persistence guarantee: quitting or crashing the
process can still interrupt work.

The internal clipboard tracker is also application-scoped: `main()` passes the
same tracker to every window `Ctx`. URI lists, cut generations, provider
identity and MOVE retry-identity guards therefore survive switching windows; a
cut in one Kito window pasted into another remains a MOVE. The GDK clipboard's
current content-provider identity, not matching text, identifies Kito's own
publication. A foreign provider change invalidates the shared internal state,
and a new publication in any Kito window replaces the prior generation.

The application likewise owns one `JobCenter` and one `OperationQueue`.
Operations dialogs are per window, but they present the same process-wide job
registry and work queue.

Job states are `queued`, `running`, `cancelling`, `completed`, `partial`,
`failed`, `cancelled`. Queued cancellation stays visible and must bypass the
worker when the FIFO reaches it; its completion still runs to release clipboard
and drag/drop state. Running
jobs observe the cancellation at the next cancellable GIO call. Completed
elements complete non-cancellable undo bookkeeping after a successful MOVE or
Trash, then cancellation applies at the next operation boundary. Cancellation
is cooperative at GIO operation boundaries, not a rollback of an item already
mutated. Completed elements remain registered; a job with failed or cancelled elements is never
presented as plain success (`partial`, `failed` or `cancelled`). Retrying
failed items starts a new job with the same captured destination; the old job
keeps its records. The Operations dialog (per window, shared registry) lists
every job with status, sources/destination, progress, a cancel command and
per-item details. At most 100 terminal jobs are retained; active jobs are never
evicted. Interrupted GIO copies or moves may leave partial output, but remain
errors rather than successes; no cleanup is attempted. After a failed copy or
move, the worker checks the selected target without following symlinks. A
verified-present target is reported by URI and blind retry is withheld. For a
copy, target `NotFound` preserves the original error and retry remains
available because the source is not mutated. A move can have side effects on
both endpoints: GIO may implement it as copy+delete across filesystems. If its
target lookup returns `NotFound`, the worker also queries the source with
`NOFOLLOW_SYMLINKS` and `Cancellable::NONE`; retry preserves the original error
only when this identity and the stable identity captured before `move_` are both
available and exactly equal. Identity capture uses `NOFOLLOW_SYMLINKS` and the
job cancellable. `OperationItemResult` retains that identity only for a failed
MOVE and never displays it. If a backend does not provide a stable identity, the
initial move is still attempted, but a later source lookup cannot authorize
retry. A changed, missing or unavailable identity marks the move outcome/source
status uncertain, reports that the target was not verified present, includes the
target URI, preserves the original GIO error domain/code, and withholds retry.
Other target lookup failures likewise report the candidate URI as status unknown
and withhold retry without claiming it exists. `Exists` is retryable only when
the target is verified absent and, for a move, the source identity is unchanged.
A MOVE failure without a pre-operation stable identity is never automatically
retryable; an initially missing source has no identity to authorize a retry.
Before a MOVE retry, the worker carries the original URI-to-identity mapping
and the core compares that identity immediately before `move_`; a missing,
unavailable or changed identity returns
`retry withheld: MOVE source identity changed` without calling `move_`. COPY
retries are unchanged. The internal cut clipboard retains per-source MOVE
guards after a failure. A stable identity is retained only for an outcome
classified as retryable; partial, uncertain and identity-changed results become
`Withheld` and manual Ctrl+V records a failed job without calling `move_`. A
worker panic or closed result channel makes every URI in that dispatch
`Unavailable`, since its filesystem outcome is unknown; queue rejection, spawn
failure and cancellation before start preserve existing guards. Unresolved cut
entries remain in the clipboard, and retrying a safe subset leaves entries
outside that subset untouched. This reduces but cannot eliminate the
path-based TOCTOU interval between the identity query and GIO's `move_`; no
portable atomic compare-and-move API is available. Do not delete uncertain
output without verified ownership.
A fully consumed cut also leaves the system clipboard untouched. `finish_cut`
keeps an internal tombstone (`uris` empty) instead of clearing the provider,
because GDK has no atomic compare-and-clear: clearing after the asynchronous
move could overwrite content another app published meanwhile. The system
provider can therefore still expose the previously published plain-text URI
list. That is harmless: the tombstone keeps the same published text, so a later
paste resolves it as an internal, consumed clipboard and reports an empty
clipboard (`clip-empty`) without dispatching a MOVE job and without pasting the
stale URIs as foreign copy data. The decision is taken by `paste_action`, which
is free of GDK access and covered by unit tests. Kito never empties the system
clipboard on its own; it only publishes (or replaces) its own provider, and a
failed publish is reported instead of being declared a successful copy or cut.
Terminal history is capped at 100. The FIFO accepts at most 128 pending tasks,
excluding its single running worker. Further submissions are rejected before
worker start and without acquiring an application hold; their completion still
runs on the GTK main thread exactly once. Registered jobs finish as `failed`
without item outcomes, users see a localized queue-full notification, and no
filesystem mutation or undo invalidation occurs. Clipboard and drop completions
still run their cleanup on rejection. Automatic cleanup of partial output is
deliberately not attempted: GIO does not provide a portable ownership guarantee
that makes deleting an uncertain path safe across backends. Preserve and report
uncertain output instead. Rename and
short creation actions are outside the F08 job center. Worker panics and spawn failures mark the job failed without
implying per-element outcomes, and undo history invalidation rules stay
exactly the ones in `operation_queue.rs`.

A completion guard also finalizes an active job as `failed` if its completion
callback returns or unwinds before recording a terminal outcome. It preserves
any terminal status already recorded; worker failures finalized before the
callback do not trigger a second notification. The queue's application hold
continues to cover this cleanup.

`job_center.rs` regression tests run workers against temporary directories
(cancel-in-flight, full success) and classified cancellations without touching
the session Trash; the unit tests cover ids, states, progress determinacy and
queued-vs-running cancellation.
