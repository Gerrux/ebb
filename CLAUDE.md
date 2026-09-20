# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Ebb — a Windows-only Rust/egui app: a "glass" layer of note cards on a second monitor, living
under other windows. Product vision is in [product.txt](product.txt) (Russian); architecture
decisions and the feature backlog are in [docs/specs](docs/specs/README.md) (also Russian) — read
`docs/specs/README.md` before any non-trivial change, it has the current status table, the
performance budget, and the data model. `docs/specs/11-tech-debt.md` tracks known issues.

## Commands

```powershell
cargo build --release                        # target\release\ebb.exe
cargo test --release --locked                # unit tests (logic: parsing, scheduling, SQL)
cargo check --release --locked --no-default-features --features wgpu   # other renderer stays buildable
cargo clippy --release --locked              # warnings reported in CI, not enforced (continue-on-error)
powershell -File installer\build.ps1         # release exe + dist\Ebb-Setup-*.exe + zip (needs Inno Setup 6)
```

Run a single test: `cargo test --release --locked <test_name>`.

CLI flags on the built exe: `ebb --quit`, `ebb --autostart-on|off|status`.

Icon regeneration: `python scripts/make-icon.py` (Pillow) writes into `assets/`.

### Manual / UI verification

- `scripts/verify-shell.ps1` drives the shell layer (tray, hotkeys, single instance, hide/show,
  `--quit`) through Win32 calls against a running release build — no synthetic input into other
  apps. Uses a real `ebb.exe`; close other running instances first. The idle-CPU check is only
  valid on a profile where the Sticky Notes import offer isn't scanning (3s after start) — point
  `LOCALAPPDATA` at a test dir if needed.
- egui-driven UI (the layer/root viewport only — bar and library are separate deferred viewports
  egui_mcp can't see): build with `--features inspection`, run with
  `EGUI_INSPECTION=127.0.0.1:5731`, drive via the `egui` MCP tools. Use `EBB_INSTANCE=test` plus a
  separate `LOCALAPPDATA=<dir>` to run a second instance next to your real one.
- `scripts/bench.ps1 -Variants 'name|exe|ENV=val;...'` — interleaved A/B runs against a CSV,
  reports medians. Use to check changes against the performance budget in
  `docs/specs/README.md` before merging anything perf-sensitive.
- Never let real user notes reach screenshots or logs — use a synthetic/test profile.

## Architecture

Single binary, modules under `src/`, wired up in [main.rs](src/main.rs). Each module has a
`//!` doc comment at its top explaining its own responsibility and constraints — read that first
when touching a file.

- **`app`** — the ambient layer itself (root viewport): cards on the desktop, drag/resize/snap,
  toasts, settings entry point, bench hooks. This is the largest file (~2500 lines) and is flagged
  as tech debt to split (`docs/specs/11-tech-debt.md` A1).
- **`bar`** — one pre-created always-on-top window shared by quick-capture and search (saves a GL
  surface vs. two windows; switching modes just resizes it). Runs in its own deferred-viewport
  callback with its own DB connection; sends changes to the layer through `Outbox`.
- **`library`** — archive/trash/settings window. Unlike `bar`, created only while open (not
  latency-critical) and fully torn down (window, GL surface, DB connection) on close. Sends
  changes to the layer through `Request`.
- **`shell`** — process-level integration: single-instance claim, tray icon, global hotkeys,
  `TaskbarCreated` (Explorer restarts). One background thread owns a hidden window and blocks in
  `GetMessageW` (zero idle cost); results reach the UI as `Event`s plus a repaint request.
- **`store`** — SQLite persistence (schema, migrations, search glue); see the data model below.
- **`search`** — turns free text + inline filters ("идеи прошлого месяца", `#tag`) into FTS5
  prefix queries; has a cheap Russian-inflection stripping step, not real stemming.
- **`resurface`** — deterministic, local-only selection of notes worth resurfacing (Rediscover,
  cooldowns). No ML, no network.
- **`sticky`** — Windows Sticky Notes import. Never opens the live `plum.sqlite` (Sticky Notes may
  be running); copies main+`-wal`+`-shm` to a temp dir (retrying on mid-copy changes) and opens
  the copy so SQLite replays the WAL, picking up notes that only exist there.
- **`import_ui`** — onboarding UI for the sticky import; scanning runs on a background thread at
  background I/O priority, well after first frame, UI thread only draws and commits one
  transaction.
- **`vault`** — DPAPI-based encryption for Private cards, bound to the Windows user profile via
  per-card entropy. Deliberately not a password-manager-grade vault.
- **`card`** — card model and the quick-capture text parser (`#tags` etc.).
- **`rich_text`** — small inline markup for note bodies, stored inline in the text (no schema
  migration needed); the editor never shows raw markup, it edits plain text with a per-character
  style and re-serializes on save.
- **`win`** — thin Win32 layer: acrylic/backdrop effects, monitor placement, hotkey registration,
  metrics.
- **`theme`** — fonts/colors; uses system Segoe UI Variable / Segoe Fluent Icons, nothing bundled.
- **`emoji`** — color emoji rendering: egui/epaint only rasterizes coverage glyphs, so a
  post-frame plugin replaces emoji glyphs (laid out via Segoe UI Emoji for correct advances/ZWJ)
  with Direct2D color output.
- **`autostart`** — per-user Task Scheduler logon task (deliberately not the `Run` registry key,
  which Explorer delays and batches). COM calls run off the UI thread.
- **`renderer`** — picks the eframe backend at build time: `glow` (default) or `wgpu` (DX12 only,
  `--no-default-features --features wgpu`); kept buildable for A/B benchmarking, not shipped.
- **`bench`** — scripted measurement run gated by `EBB_BENCH=<csv path>` (see `scripts/bench.ps1`).

### Data model (SQLite)

```sql
cards(id, kind, title, body, tags /*csv*/, pinned, archived,
      x, y, w, h, created_at, updated_at, last_viewed_at, deleted_at)
cards_fts  -- FTS5 external content (title, body, tags), unicode61 remove_diacritics 2
settings(key, value)  -- layer.monitor, layer.backdrop, layer.tint, layer.pin_bottom, layer.collapsed, sticky_import
imported(source_id, card_id, batch)
```

Notes live in `%LOCALAPPDATA%\Ebb\ebb.db`. Schema currently migrates via `IF NOT EXISTS` /
`pragma_table_info` checks in `store` rather than numbered migrations (tracked as tech debt A3).

### Cross-window communication

There is no single event bus yet (tracked as tech debt A2): `bar::Outbox`, `library::Request`, and
`shell::Event` are three separate channels, each carrying that window's messages back to the root
viewport/`app`.

## Engineering constraints (do not regress without discussion)

These come from measurements in `docs/specs/README.md`, taken with `scripts/bench.ps1` (release,
≥5 interleaved runs). Note: launching under the Claude shell's MSIX virtualization adds ~40ms to
startup — only compare A/B builds run the same way.

| Metric | Current | Budget |
|---|---|---|
| First frame from process start | ~290ms (254ms outside a container) | ≤350ms |
| Idle working set / private bytes | 2.5 / ~105 MB | WS ≤5MB, private ≤115MB |
| Idle CPU | 0ms / 3s | 0 (≤1 scheduler tick) |
| Hotkey → capture window frame | ~15ms | ≤30ms |
| Hotkey → search window frame | ~21ms | ≤40ms |
| Search hit, 10k notes | p95 ≤7ms | p95 ≤20ms |
| Search full miss | ~25ms | ≤50ms |
| Sticky import scan, 468 notes | 140–190ms in background | background only, never on UI thread |

Rules that follow from these budgets:

- **No polling.** Timers only use `request_repaint_after` to the next known event; background
  events use blocking waits (`GetMessageW`, waitable timers).
- **Nothing heavy before the first frame.** Background tasks start no sooner than 2–3s in, at
  `THREAD_MODE_BACKGROUND_BEGIN`.
- **Never call `DwmFlush`** near window presentation (measured to delay first show by ~170ms).
- **Windows are created on demand**, except `bar` (pre-created deliberately to remove capture
  latency). A window created on open is destroyed on close (see `library`).
- **The UI thread never blocks on network, COM, or disk for >5ms.**
- **User data never leaves the machine** without an explicit user action.

## Notes on shipping

- Version bump: raise `version` in `Cargo.toml`, commit, then `git tag vX.Y.Z && git push origin
  vX.Y.Z` — the `release` workflow builds the installer and publishes the GitHub release. The
  release workflow verifies the tag matches `Cargo.toml`'s version and fails otherwise.
- `installer/build.ps1` is the single source of truth for producing `dist/`: used identically
  locally, in `ci.yml`, and in `release.yml`.
