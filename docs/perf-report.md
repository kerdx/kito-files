# Kito Files — performance report

Baseline first, then changes. Only justified optimizations were kept.
No commit/push from this work.

## Environment

- CPU: 6 cores, RAM 16 GiB, `/tmp` on tmpfs, Fedora, rustc 1.98.1.
- GTK 4.22.5, libadwaita 1.9.4, Wayland session (`WAYLAND_DISPLAY=wayland-0`,
  `DISPLAY=:0`, Plasma). No `perf`/`valgrind`/`hyperfine` available:
  wall time via `std::time::Instant`, resident memory via `/usr/bin/time -v`
  where noted.
- Fixtures: fresh `tempfile` dirs with 100 / 10.000 / 50.000 entries
  (files + ~10% dirs, hidden dotfiles, Unicode names incl. `café ☃ 日本語 😀`).
  Never personal files; no system caches were cleared.
- Reps: 5 runs after 1 warmup; median quoted (min/max alongside).
  Warm = page cache hot; first (cold) run is slower and reported separately
  where observed.
- Repro: bench sources live outside the repo (scratch crate); rerun with
  `cargo run --release --bin <name>` from the scratch dir:
  - `kito-bench list` — end-to-end `kito_core::list_dir` + sort-only split
  - `kito-bench sortbench` (`src/bin/sortbench.rs`) — old vs cached-key sort
    on deterministically shuffled names
  - `kito-bench modelbench` (`src/bin/modelbench.rs`) — `ListStore`
    append-loop vs single `splice` vs chunked `splice` + notification counts
  - `kito-bench busywait` (`src/bin/busywait.rs`) — idle invocations during
    a 1 s worker job, old polling vs pipe-wakeup principle
  - `kito-bench iconcost` / `oldnew` — icon serialization isolation,
    old-vs-new `list_dir` interleaved on the same fixture

## Baseline (before changes, release)

End-to-end `list_dir` (enumerate + sort, warm tmpfs):

| entries | median | min | max |
|---|---|---|---|
| 100 | 0.75 ms | 0.64 ms | 1.14 ms |
| 10.000 | 81.9 ms | 76.5 ms | 84.6 ms |
| 50.000 | 475 ms | 442 ms | 494 ms |

All of it ran on the UI thread (`tabs → file_list::reload → list_dir`),
so a 50k folder froze the interface for ~0.5 s.

Sort-only on already-sorted input is negligible (~2.4 ms at 50k);
on shuffled input the old comparator costs much more (see §3).

Attribute cost at 50k (same fixture, warm):

| attrs | median |
|---|---|
| `name,type` | 48 ms |
| `name,type,size` | 187 ms |
| full (`+icon,content-type`) | 394 ms |

Model application at 50k (50k `GObject` news excluded from timer):

| method | median | `items-changed` emissions (1000 rows) |
|---|---|---|
| `remove_all` + N `append` | 15.0 ms | 1001 |
| single `splice` | 5.1 ms | 1 |
| chunked `splice` (1000/turn) | 5.9 ms | ~50 |

Worker wakeup: the old `run_in_thread` polls `try_recv` from an idle
callback. During a 1 s job the probe counted **≈850k–1.1M idle
invocations/s** — the main loop never sleeps.

## 1. Async directory loading — implemented

`FileTab::navigate` now resolves the target, bumps a per-tab generation id
(`LoadGen`), shows a discreet loading page (spinner + `loading-folder`,
distinct from the empty-folder page) and enumerates+sorts in a worker
thread. Results travel via a bounded `async-channel` (wakes the loop once)
and are applied only if the tab is alive and the id is still current.
History commits only on success, so failed loads leave URI, stacks, store
and previous content untouched; store and current URI always move together
in `apply_loaded`, and chunked inserts check the id on every turn.

Effect: UI-thread blocking per navigation goes from ~475 ms (50k) to ~0
plus ≤500-row main-thread chunks (sub-ms each). Total wall time unchanged
(worker still enumerates), but the interface stays responsive with explicit
loading state. Cancellation = newer id discards older results; tab close =
weak upgrade fails; partial loads can only reference the single new folder.

## 2. Bulk model update — implemented

`file_list::reload` and the async path use one `splice` per batch instead
of per-row `append`: ~3× faster at 50k (15.0 → 5.1 ms) and 1001 → 1 model
notifications per 1000 rows. Large loads stream in `LOAD_CHUNK = 500`
splices so no single main-loop turn grows with folder size
(chunked ≈ single-splice total time). `FileObject` creation stays on the
main thread but is likewise bounded per turn. Selection restore
(`select_uri`), counts, view mode and per-tab state are preserved; empty
vs list page logic unchanged.

## 3. Cached sort keys — implemented

`list_dir` used `to_lowercase()` twice per comparison (O(n log n)
allocations). Now `sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()))`:
one key per element, dirs-first and Unicode/equal-key order verified
identical (stable sort; `BETA` still precedes `beta`).

| shuffled n | old median | new median | speedup |
|---|---|---|---|
| 1.000 | 0.81 ms | 0.10 ms | 8.1× |
| 10.000 | 12.5 ms | 1.57 ms | 7.9× |
| 50.000 | 79.6 ms | 16.0 ms | 5.0× |

(On already-sorted input the sort is O(n) either way and enumeration I/O
dominates; the win materializes on real unsorted folders, e.g. −64 ms at
50k.)

## 4. Busy-wait removed — implemented

`ops::run_in_thread` no longer polls `try_recv` from `idle_add_local`.
The worker `send_blocking`s into a bounded `async-channel(1)` and a
`spawn_future_local` future delivers the result on the main thread; the
loop sleeps until then (no sleeps or blocking waits on the UI thread).
Same `Send`-only crossing (`R: Send`), same completion/error/close
semantics (sender loss = silent no-op, as before), clipboard
generation guards and cut-in-flight protections untouched.
Probe: 1 s job went from ~1M idle wakeups to zero while waiting.

## 5. On-demand metadata — measured, NOT implemented

Full enumeration costs ~8× the minimal `name,type` pass at 50k
(394 vs 48 ms). A two-phase load (names fast, metadata later) would need
either a second full enumeration (2× I/O, 2 round-trips on remote
backends) or per-file `query_info` (N round-trips — unacceptable on
`smb`/`sftp`), plus visible-row tracking, concurrency caps and
cancellation for the fill-in phase. With §1 the full cost already runs
off-thread behind a loading indicator, placeholders already exist in the
factories, and Details view needs sizes anyway. Verdict: keep the single
full enumeration (one round-trip, remote-safe); the async move captures
the responsiveness win without the N+1 risk.

## 6. Release profile — measured, KEPT at `z`

Same new-code binary, only `opt-level` differs (all other options equal,
no target-CPU flags):

| profile | size | `--help` startup | max RSS |
|---|---|---|---|
| `z` (current) | 911.264 B | 0.04 s | ~34 MB |
| `3` | 1.193.888 B (+31%) | 0.04 s | ~34 MB |

Same-data CPU bench (deterministic shuffled sort, `sortbench`):

| n=50k | `z` | `3` |
|---|---|---|
| old comparator | 59.6 ms | 60.2 ms (noise) |
| new cached-key | 11.26 ms | 11.25 ms (identical) |

(10k shows ≤0.5 ms absolute deltas, same noise scale.)
Verdict: `3` buys nothing measurable here and costs +31% size, so
`Cargo.toml` keeps `opt-level = "z"`. Repro:
`cargo build --release` vs
`cargo build --release --config 'profile.release.opt-level=3'`,
then `/usr/bin/time -f "elapsed=%e maxrss=%M KB" ./target/release/kito-files --help`
and the `sortbench` binaries built under each profile.

## Regression tests (all headless, temp dirs only)

- `kito-core`: dirs-first/case-insensitive/Unicode/equal-key order;
  icon strings parse via `Icon::for_string`.
- `kito-gtk file_list`: single-notification `replace_all`, chunked ==
  single-shot content/order, empty-chunk no-op.
- `kito-gtk ops`: `run_in_thread` delivers values and errors on the loop.
- `kito-gtk tabs`: `LoadGen` supersede chains; out-of-order + failed
  results never commit (history/stacks intact); `settled_child` keeps
  loading/empty/list distinct incl. failed-first-load legacy look;
  closed-tab weak-drop contract.
- Existing history/hidden/selection suites unchanged and passing.

`cargo test --workspace` (35 + 78 + 11), `cargo check --workspace`,
`cargo fmt --check` green. `cargo clippy -- -D warnings` still fails only
on pre-existing lints in untouched code (`kito-core` doc indentation,
`kito-i18n` `question_mark`); no new lints from this work.
One new test initially raced two threads for default-context ownership and
was merged into a single sequential delivery test (stable over repeats).

## Limits / not covered

- No `perf`/flamegraphs (tool unavailable); CPU claims rest on idle-count
  probes and wall-time splits, not sampled profiles.
- Graphical check on Wayland only, no pixel capture: a private-bus instance
  with a 3000-entry folder stayed alive with no app errors (only sandbox
  a11y warnings from the isolated bus); spinner position and chunk
  smoothness were not captured frame-by-frame. X11 not tested. No cold-cache
  system-wide numbers (caches of other apps never cleared by design).
- One worker thread per navigation: rapid retargeting can briefly run two
  enumerations concurrently; stale ones exit after delivery without
  touching the UI. No thread pool / unbounded parallelism added.
- Folder size counting in Properties still enumerates synchronously;
  single-file trash stays synchronous (fast path, unchanged behavior).
- Deferred metadata (§5) not implemented by measurement-backed decision;
  `opt-level` (§6) kept at `z` by measurement.
