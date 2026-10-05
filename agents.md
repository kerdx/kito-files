# Kito Files — agents.md

> Nome progetto: **Kito Files** (visibile in UI, About, `.desktop`).
> Identificativi tecnici invariati: crate `kito-core`/`kito-gtk`,
> binario `kito-files`, app id `it.kito.KitoFiles`, config `~/.config/kito-files/`,
> textdomain `kito-files`, actions `~/.local/share/kito-files/actions/`.

## Indice
1. Obiettivo
2. Riferimenti
3. Stack (toolchain, UI, backend, preview, config/search/log/i18n)
4. Architettura
5. Scope MVP v1 / fuori MVP
6. Piano Fase 1 (scaffold + checkpoint)
7. Testing / qualità
8. Regole WM / portabilità
9. Cosa non fare
10. Packaging

## 1. Obiettivo
File manager in Rust + GTK4, nativo, leggero, indipendente da GNOME.
Vista classica (stile Thunar/Nautilus/Nemo, una cartella alla volta) —
niente Miller columns. Wayland-only. Deve girare su GNOME, KDE e window
manager Wayland puri (sway/Hyprland/river/labwc), senza dipendenze hard
GNOME e senza X11.

## 2. Riferimenti
| Progetto | Prendere | Evitare |
|---|---|---|
| `euclio/fm` | Solo pattern generici (preview async, progress op) | Vista Miller, setup con `libpanel-git`, toy con rischio data loss |
| `Relm4/Relm4` 0.10 | Niente (scartato: overhead vs gtk4-rs diretto) | — |
| `lxqt/pcmanfm-qt` | Core separato da UI, backend su GIO/GVFS, nessun hard-dep DE | Stack C++/Qt6, `libfm-qt`, licenza GPLv2 |
| `GNOME/nautilus` | Pathbar breadcrumb + menu, batch rename con tmp-file anti-collisioni, thumbnail async, sidebar luoghi/recent/bookmark | Dipendenze Tracker/portal GNOME |
| `linuxmint/nemo` | Type-ahead find, status bar selezione, toggle breadcrumb/entry, open-in-terminal configurabile, open-as-root via pkexec, progress visibile, formato custom actions TOML | Stack GTK3 + cinnamon/xapp, extension Python/C, `nemo-desktop` |
| `xfce/thunar` (primario per layout) | Side pane `Devices\|Places\|Network` + tree toggle, 3 viste (icone / dettagliata / compatta), split panes (v1.x), status/toolbar custom (v1.x), dialog proprietà completo, remember-view per cartella (v1.x), custom actions a sottomenu (v1.x) | Stack GTK3 + libexo/libthunarx/xfconf, plugin C |

## 3. Stack

### Toolchain
- Rust stable, edition 2021, solo `cargo`.
- Release: `lto = true`, `strip = true`, `opt-level = "z"` o `"s"`,
  `codegen-units = 1`, `panic = "abort"`.

### UI — GTK4 + libadwaita, niente GNOME desktop
- Target GNOME 50: `gtk4 0.11` (feature `gnome_50` = GTK 4.22 + gio v2_88)
  + `libadwaita 0.9` (feature `v1_10`) via gtk4-rs / libadwaita-rs:
  `gtk = { package = "gtk4", version = "0.11", features = ["gnome_50"] }`,
  `adw = { package = "libadwaita", version = "0.9", features = ["v1_10"] }`.
  libadwaita = ~5MB, solo `gtk4/glib2/pango/graphene/fribidi/appstream`,
  nessun trascinamento di gnome-shell/mutter.
- Fallback distro vecchie: tenere il codice compatibile con
  `gnome_49` + libadwaita `v1_8` (GTK 4.18+).
- Vietato: `libpanel`, `GSettings/dconf` per la config.
- Widget: `AdwApplication + AdwApplicationWindow + AdwNavigationSplitView
  (sidebar + contenuto) + AdwTabView + AdwToastOverlay +
  GtkColumnView/GtkGridView + SingleSelection + Factory + GtkPopover +
  GtkSearchEntry`.
- Layout: sidebar a sinistra, header con breadcrumb, corpo con UNA vista
  alla volta, status bar in basso.
- Sidebar: 3 sezioni (`Devices | Places | Network`) + tree toggle,
  `GVolumeMonitor` + `~/.config/gtk-3.0/bookmarks` + XDG user dirs.
- DnD/clipboard: solo API GTK (`GtkDragSource/DROP`, `gdk::Clipboard`).
- Finestre su `xdg-shell`; `layer-shell` solo per eventuale `--desktop`.
  Test solo Wayland (`GDK_BACKEND=wayland`).
- Tema Adwaita di sistema via `AdwStyleManager`; CSS custom solo ritocchi;
  icone da `IconTheme` freedesktop.

### Backend file — freedesktop, non GNOME
- Tutto via `gio::File` (copy/move/trash/mount/monitor) + `GFileMonitor`
  + `GVolumeMonitor`. Mai `std::fs` nel thread UI (`glib::spawn_future_local`).
- Scan: `jwalk + rayon`. Mime: `gio content-type` + icon theme.
- Open: `gio AppInfo` + fallback `xdg-open`; errori permessi/polkit in
  dialog, mai panic. `gvfs` solo runtime opzionale.
- Safety: nessuna distruttiva senza conferma; trash di default, delete
  permanente solo con conferma esplicita.

### Preview — minimale di default
- `GtkTextView` plain + `gdk-pixbuf`, resto via `xdg-open`.
  Troncare i file grandi (prime/decine di KB + indicatore), mai file
  interi in memoria.
- Feature `preview-full` (`gtksourceview + poppler-glib`), default OFF.

### Config / search / log / i18n
- Config TOML (`serde + toml + directories`, XDG `~/.config/kito-files/`),
  chiave `lang` opzionale come override lingua.
- Search `nucleo` + fallback substring. Log `log + env_logger`.
  Thumbnails `~/.cache/thumbnails` fatte in casa.
- i18n `gettext-rs` dal MVP: sorgenti sempre in inglese, `gettext::_()`
  solo in UI; lingua dal sistema (`setlocale(LC_MESSAGES,"")` →
  `bindtextdomain` + `textdomain`); fallback inglese automatico;
  `kito-core` resta in inglese; `lang` in config ha precedenza.

## 4. Architettura
- `kito-core`: ops file pure su gio, testabile senza GTK.
- `kito-gtk`: solo UI che chiama `kito-core`.

## 5. Scope MVP v1 / fuori MVP
- Dentro: navigazione classica 3 viste, tab, sidebar 3 sezioni + tree
  toggle, copy/move/rename/trash con progress + undo dove possibile,
  search fuzzy, preview troncata, apertura con app default, bookmark,
  pathbar breadcrumb toggleabile, status bar, type-ahead find,
  open-in-terminal configurabile, open-as-root via pkexec con warning,
  progress in title + toast, dialog proprietà base.
- Fuori (v1.x): `--desktop` (processo separato), rete avanzata, batch
  rename pattern/regex, custom actions TOML a sottomenu, split panes,
  toolbar/statusbar custom, remember-view per cartella, plugin,
  full-text index, Miller columns (scartate, non reintrodurre).

## 6. Piano Fase 1 (scaffold + checkpoint)
1. Workspace `kito-core` + `kito-gtk`, dipendenze §3, `main.rs` con
   `AdwApplicationWindow` + HeaderBar minima.
2. Verifica `GDK_BACKEND=wayland cargo run` su GNOME 50.
3. `kito-core`: `ls(dir)` + copy/move/trash con progress (test temp dir).
4. UI: UNA `GtkColumnView` navigabile (click/Enter/Backspace) + refresh
   `GFileMonitor` (o F5 manuale).
5. Checkpoint: se la navigazione classica è fluida → Fase 2
   (sidebar, tab, clipboard, status bar). Altrimenti si rivede il
   widget lista prima di investire altro.

## 7. Testing / qualità
- `cargo test -p kito-core` su temp dir per ogni op file.
- `cargo clippy -- -D warnings`, `cargo fmt --check` in CI.
- Test manuale Wayland su GNOME + sway/Hyprland.

## 8. Regole WM / portabilità
1. Senza GNOME installato, senza dconf, senza portali hard
   (libadwaita ok: è widget lib, non desktop).
2. Wayland-only, niente API X11, niente test Xorg.
3. Solo `xdg-shell` per le finestre (`wlr-layer-shell` solo `--desktop`).
4. Config TOML, mai GSettings.
5. Runtime: `gtk4-wayland + libadwaita + glib + gdk-pixbuf + gvfs(opz.)`.

## 9. Cosa non fare
- libpanel; Miller columns (non reintrodurre); `std::fs` bloccante in UI;
  Qt/Electron/tracker/DBMS metadati; temi/icone GNOME obbligatori;
  delete permanente senza conferma.

## 10. Packaging
- Dev: `GDK_BACKEND=wayland cargo run`.
- `cargo-dist` + deb/rpm Wayland-only; Flatpak con runtime GNOME bundled.
- Flatpak: `--socket=wayland` (no X11), `--filesystem=host`,
  `--talk-name=org.freedesktop.FileManager1`, portal FileChooser/OpenURI.
