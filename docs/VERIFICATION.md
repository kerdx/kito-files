# Verification guide

[Development guide](DEVELOPMENT.md) · [User guide](USAGE.md)

The Phase 0 matrix below records the current results. The remaining cases are
procedures for additional checks; expected results are acceptance criteria, so
a missing feature or unmet roadmap requirement is not a pass.

## Phase 0 verification matrix (2026-10-08)

The automated run used the workspace checkout on Linux with Rust 1.98.1,
GTK 4.22.5 and libadwaita 1.9.4. `cargo fmt --all --check`,
`./scripts/check.sh` and `cargo build --workspace --locked` were run; the check
script passed 36 `kito-core`, 91 `kito-gtk` and 11 `kito-i18n` tests plus
Clippy. It filtered out the session-Trash integration test. No GTK interaction
was observed. The host exposed `WAYLAND_DISPLAY=wayland-0`, `DISPLAY=:0` and
`GDK_BACKEND=x11`, but those variables do not establish which backend the app
used. A UI automation inventory exposed no controllable app/window, so the
matrix does not claim a Wayland or X11 pass. A generated 20,000-entry fixture
and isolated XDG config/data/cache were prepared under `/tmp`; no personal
files, real Trash contents or real clipboard were used.

| ID | Requisito e risultato atteso | Tipo | Ambiente/backend effettivo | Esito | Evidenza, limiti e lavoro residuo |
|---|---|---|---|---|---|
| B01-1 | Menu sullo sfondo con icone e senza barra di scorrimento, con e senza selezione; Proprietà descrive la cartella e lascia intatta la selezione. | Automatica, sorgenti, grafica | Test headless Linux; backend GUI non osservato | NON ESEGUITO | Struttura del modello e codici azione coperti dai test; layout custom ispezionato nei sorgenti. Controllare entrambi i casi e la selezione in GUI. |
| B01-2 | Sottomenu Nuovo file con icone per ciascun tipo, clic fuori, Escape, navigazione da tastiera e aperture ripetute senza chiusure errate o callback residue. | Automatica, grafica | Test headless Linux; backend GUI non osservato | NON ESEGUITO | Azioni e icone (file vuoto, testo, HTML) coperte dai test; interazioni GTK non esercitate. |
| B01-3 | Il menu del cestino mantiene le azioni dedicate e le conferme distruttive. | Automatica, sorgenti, grafica | Test headless Linux; test Trash reale escluso | NON ESEGUITO | Test modello menu passato; non è stato aperto né svuotato il cestino. Eseguire in sessione isolata. |
| B02-1 | Sotto 200 ms non compare l’indicatore; dopo 200 ms compare overlay piccolo con spinner, testo e Interrompi; nessuna durata minima. | Automatica, grafica | Logica unit test headless Linux; backend GUI non osservato | SUPERATO (logica) | Testa generazione e soglia della logica; tempi di rendering e risposta visiva non misurati. |
| B02-2 | Navigazioni rapide/out-of-order applicano solo la destinazione più recente; cronologia e vista restano coerenti. | Automatica, sorgenti, grafica | Test headless Linux; backend GUI non osservato | SUPERATO (stato) | Test di generazioni, risultati fuori ordine, errore e cronologia passati; manca prova di navigazione interattiva. |
| B02-3 | Errore o annullamento durante lettura/inserimento a blocchi lascia la vista precedente coerente; annullamento interrompe il lavoro quando possibile. | Automatica, sorgenti, grafica | GIO e stato testati headless Linux; backend GUI non osservato | SUPERATO (unità) | Cancellazione GIO preannullata, invalidazione e risultati obsoleti coperti; annullamento durante un chunk non è testato direttamente. Il sort non è interrompibile a metà e la UI non è stata esercitata. |
| B02-4 | Chiusura scheda/finestra durante il caricamento non causa crash o mutazioni tardive. | Automatica, grafica | Test di stato headless Linux; backend GUI non osservato | SUPERATO (stato) | Testa lo scarto quando la scheda è chiusa; distruzione finestra e cancellazione nel runtime grafico da verificare. |
| B03-1 | Successi, fallimenti e successo parziale mostrano un riepilogo, dettagli per elemento e cause, senza un dialogo per file. | Automatica, sorgenti, grafica | Test headless Linux; backend GUI non osservato | NON ESEGUITO | Record e costruzione del dialogo implementati; manca test del dialogo con errori indotti e controllo dei testi/accessibilità. |
| B03-2 | Appunti sostituiti durante un’operazione non vengono sovrascritti; retry del taglio resta spostamento e include solo fallimenti. | Automatica, grafica | Test appunti simulati headless Linux; clipboard reale non toccato | SUPERATO (unità) | Test di cambio generazione, taglio parziale, retry solo falliti e duplicati concorrenti passati; manca prova da sessione desktop. |
| B03-3 | Cancellare un symlink non modifica il target. | Automatica | GIO su directory temporanee Linux | SUPERATO | Test su link valido, rotto, circolare e alberi con link esterni passati. |
| B03-4 | Copiare una directory dentro sé stessa o discendenti, anche via symlink, viene rifiutato prima di creare una copia ricorsiva. | Automatica | GIO su directory temporanee Linux | SUPERATO | Test sorgente, sottodirectory e destinazione via symlink passati. |
| B03-5 | Ripristino gestisce collisioni, URI speciali e nomi non UTF-8 senza sovrascrivere o perdere i byte del percorso. | Automatica | GIO su directory temporanee Linux | SUPERATO | Test di collisione, metadata e percorso non UTF-8 passati; flusso integrato dal cestino e backend GVfs non provati. |
| B04-1 | Inglese/italiano, tema chiaro/scuro, tastiera e accessibilità restano utilizzabili. | Automatica, grafica | Cataloghi testati headless Linux; backend GUI non osservato | NON ESEGUITO | Test dei cataloghi e delle chiavi passati; temi, focus, screen reader e navigazione menu da verificare graficamente. |
| B04-2 | L’app funziona su Wayland e X11, provati separatamente. | Grafica | Endpoint Wayland/X11 presenti; backend app non confermato | NON ESEGUITO | Nessuna prova visiva/interattiva affidabile disponibile; ripetere la matrice manuale su entrambi i backend. |

Per completare le righe non eseguite, seguire i casi manuali sottostanti in una
sessione grafica reale e registrare backend, controlli osservati e limiti.

## Automated checks

From the repository, run:

```bash
./scripts/check.sh
```

The script also works from another directory when invoked by its full path.
It stops at the first failure, does not format files, and uses `Cargo.lock`
without updating dependency resolution. Install the dependencies listed in the
development guide, including Rust's rustfmt and Clippy components.

The default run excludes `restore_roundtrip_through_trash`, which creates a
temporary directory under the user's home and uses the session's real Trash.
In a disposable user/session with an isolated Trash, include it with:

```bash
./scripts/check.sh --with-trash-test
```

A green test run does not prove every case executed: inspect output and test
prerequisites. Some GTK tests return early when no display is available; the
Trash round-trip can return early when its backend is unavailable. Record these
as **not exercised**, not successful integration tests.

For documentation-only changes, check links and `git diff --check`; compilation
is unnecessary. Changes to the check script require shell syntax and invocation
checks as well as a real run when the environment permits.

## Environment and safe fixtures

Record revision, local changes, distribution, desktop/compositor, actual GTK
display backend, Rust/GTK/libadwaita versions, build profile and GVfs availability.
Use a Wayland session and an X11 session when available. Environment variables
alone do not establish the backend used. If backend identity cannot be confirmed,
record it as unknown; do not label that run as a backend compatibility pass.

Build with `cargo build --locked` and start a fresh app process with
`target/debug/kito-files PATH`. An already running application can receive the
request instead, retaining its existing environment and preferences.

Use only generated files. One optional fixture setup (Bash and Python 3):

```bash
fixture_root="$(mktemp -d -t kito-verification.XXXXXXXX)"
mkdir -p "$fixture_root"/{source/child,destination,empty,large,protected,config,data,cache}
python3 - "$fixture_root" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
for name in ['plain.txt', 'with spaces.txt', 'caffè 日本語.txt', 'hash#percent%.txt', '.hidden']:
    (root / 'source' / name).write_text('verification fixture\n')
(root / 'source' / 'child' / 'nested.txt').write_text('keep this content\n')
(root / 'destination' / 'plain.txt').write_text('existing destination\n')
for n in range(5000):
    (root / 'large' / f'entry-{n:05}.txt').touch()
(root / 'source' / 'destination-link').symlink_to(root / 'destination', target_is_directory=True)
(root / 'child-alias').symlink_to(root / 'source' / 'child', target_is_directory=True)
(root / 'source' / 'broken-link').symlink_to(root / 'missing')
PY
printf 'Fixture directory: %s\n' "$fixture_root"
```

For preference tests, launch a fresh process with disposable config/data/cache:

```bash
XDG_CONFIG_HOME="$fixture_root/config" \
XDG_DATA_HOME="$fixture_root/data" \
XDG_CACHE_HOME="$fixture_root/cache" \
target/debug/kito-files "$fixture_root/source"
```

These variables isolate app files, **not necessarily the session's GVfs daemon
or `trash:///`**. Test Trash, emptying Trash and root-shell actions only in a
disposable account/session or VM. Do not empty the personal user's Trash.
Clipboard tests replace session clipboard contents; perform them in a test
session when the current clipboard must be preserved. Bookmark tests likewise
need isolated configuration and must not modify personal bookmarks.

Close the test app, restore any fixture permissions, and remove only the exact
generated directory after checking its path. Do not use broad cleanup commands.

## Manual cases

Use **PASS**, **FAIL**, **BLOCKED** (environment prevents execution) or
**NOT RUN**. A planned but absent function is a failing acceptance criterion;
describe the gap rather than marking it verified. Selection currently supports
one item; do not assume multi-selection is implemented.

| ID | Steps | Expected result |
|---|---|---|
| NAV-01 | Open `source`, `child`, go back, forward and up; repeat in Icons, Compact and Details. | Folder, path, rows and history agree; view switching preserves location. |
| NAV-02 | Click empty path-bar space; edit with Ctrl+L; try spaces, Unicode, `#`, `%`, relative paths and `~/`; use arrows, Tab, Enter and Esc. | Editor opens across the bar; completion and navigation preserve the intended path; Esc exits editing. |
| NAV-03 | Enter a nonexistent folder or an inaccessible fixture folder. | Error is visible; previous folder and history remain valid; actions target the visible folder. |
| NAV-04 | Open `large`, immediately navigate elsewhere, change tabs, then close the loading tab. Repeat. | Latest navigation wins; no stale rows/history, crash or callback affecting a closed tab. |
| LOAD-01 | Compare a small folder with `large`; interact during loading; interrupt if the action exists. | Roadmap B02: indicator only after 200 ms, no minimum wait, small overlay and interruption; previous content is not misrepresented as the new folder. Record missing behavior. |
| TAB-01 | Open two tabs, give each a different folder and view; navigate and switch repeatedly. | Location, view and history stay per tab; toolbar and actions follow the active tab. |
| VIEW-01 | Toggle hidden files, switch tabs, open `empty`, resize the window. | Hidden-file policy applies consistently; empty state differs from loading; controls remain reachable. |
| MENU-01 | Select a file, then right-click empty space, including near window edges. Inspect Properties. | Background menu has no decorative arrow; properties describe the folder despite file selection; menu stays on screen. |
| MENU-02 | Open the creation submenu, move pointer into it, create a folder/file; dismiss with Esc and outside click. Repeat, then switch/close tabs. | Submenu is usable; action occurs once; popovers close cleanly and do not leave stale actions or widgets. |
| MENU-03 | Inspect menus on writable/read-only folders, a remote location when available, and Trash. | Actions reflect context and capability; unsupported terminal/creation actions are not offered; Trash has its dedicated menu and confirmations. |
| FILE-01 | Create a folder/file, rename via F2; try empty names, separators and existing names. Open a fixture file. | Validation prevents invalid operations and silent overwrite; errors identify the problem; default app receives the intended file. |
| COPY-01 | Copy `plain.txt` into `destination`, which already contains it; paste again. | Existing content stays unchanged; copies get distinct safe suffixes and correct contents. |
| COPY-02 | Copy `source` into itself, into `source/child`, then into `child-alias`. | Each destination is rejected before recursive copying; no partial directory tree is left behind. |
| CUT-01 | Cut and paste to another fixture folder; invoke paste repeatedly during the move. Change the clipboard in another app before a later paste. | Cut is not dispatched concurrently; completed items are not moved twice; paste follows the current system clipboard. |
| ERR-01 | Make a fixture destination unwritable as an unprivileged user; attempt copy/cut, then restore permissions and retry. | Error names the failed item/reason; failed cut remains retryable; UI stays usable. A root process cannot validate this permission case. |
| ERR-02 | When a multi-item operation is available, include successful and failing fixture items. | Partial failure lists failed items; successful items are not retried accidentally. If unavailable in UI, verify backend coverage and record the UI limitation. |
| DELETE-01 | Permanently delete a fixture symlink to `destination`; cancel once, then confirm. Repeat with `broken-link` and a folder containing a symlink. | Cancel preserves items; confirmation removes only links or selected tree; external target and its contents survive. |
| TRASH-01 | In an isolated session, trash a fixture file, restore it, then repeat with a conflicting original filename. | Original data returns; existing destination is preserved with a safe collision suffix. |
| TRASH-02 | In the isolated Trash, cancel permanent deletion/emptying, then confirm on generated items only. Repeat where the backend is unavailable. | Cancel preserves data; confirm removes only test data; backend failure is reported without fallback to permanent deletion. |
| PREF-01 | Change default view, single/double click and window buttons; test existing/new tabs and windows; restart with the same disposable config. | Preferences apply at the documented scope and persist; custom button layout and system-follow mode work without changing desktop settings. |
| LANG-01 | Switch English/Italian/System with menus and Preferences open; restart in supported, unsupported and missing message locales. | App text updates and persists; automatic mode follows supported system locale and otherwise uses English. Standard GTK strings may follow system locale separately. |
| TERM-01 | Choose an installed terminal, open it from a local path with spaces; test a saved unavailable terminal and a remote URI. | Local working directory is correct; missing choice falls back visibly; unsupported context is handled. Root shell, if tested in isolation, does not run the file manager as root. |
| BOOK-01 | In disposable config, add/remove a fixture bookmark; edit that bookmark file externally. | Sidebar and freedesktop bookmark file remain consistent; external changes appear without restart. |
| INPUT-01 | Exercise documented keyboard shortcuts, tab focus, Escape and mouse back/forward; use a narrow window and system light/dark appearance. | Controls remain usable by keyboard; focused editor does not trigger unrelated file actions; text and icons stay readable. |

For performance work, measure enumeration, sorting and UI application separately.
Record folder size, filesystem, cold/warm cache, build profile and repeated timings;
compare the same fixture and environment before/after. Visual observation alone
does not establish the 200 ms threshold or a numerical speedup.

## Result template

Attach a concise record to the change or its verification report; do not check
off roadmap entries just because this guide exists.

```text
Revision and local changes:
Change under verification:
Environment / actual backend / build profile:
Automated command and result (including skips):
Manual case IDs and PASS / FAIL / BLOCKED / NOT RUN:
Evidence and reproduction steps for failures:
Wayland coverage:
X11 coverage:
Remaining gaps:
```
