# Textamp Project Instructions

## Build and verification

Build with `cargo build --release --bin textamp`. Do not say "done" until it
succeeds with zero warnings. Run `cargo test`, `cargo fmt --all -- --check`, and
`cargo clippy --all-targets -- -D warnings` for functionality changes.

Use isolated XDG configuration/data/cache/state directories for automated app
tests. Live tests that change playlists, favorites, ratings or play history must
use the dedicated loopback fixture, never a personal music server. Audio tests
use `DummyBackend` or explicitly opt into native hardware.

## UI consistency

Update relevant shortcut bars in `src/ui/app.rs`, the shared help text in
`src/util/help_text.rs` (rendered by `src/ui/screens/help.rs`), layout comments,
and README when functionality changes.

Clicking to highlight must not recenter the viewport. Set the `scroll_pin` on
click and clear it on keyboard navigation. Settings/library management must
support both mouse and arrows; opening it must not switch libraries or play.

## Architecture

Textamp is a pure terminal app. Supported sources are Subsonic/Navidrome,
local folders and WebDAV, with optional AudioMuse analysis. Plex is retired;
its source archive is not a runtime dependency. Do not restore Plex fallbacks.

```text
UI: src/ui (pure render-from-state)
  ↓ input actions / render feedback
App: src/app (state, reducers, task ownership, event loop adapters)
  ├─ src/app/sources (provider routing and orchestration)
  │    ├─ src/navidrome (OpenSubsonic HTTP and normalized catalog)
  │    ├─ src/audiomuse (analysis/discovery HTTP)
  │    └─ src/library (shared models, local/WebDAV, persistent caches)
  ├─ src/services (UI-independent business logic)
  ├─ src/media (artwork, waveform and spectrogram caches/analysis)
  └─ src/audio (local decoding/output; no UI or library API imports)
```

`AppState` is the source of truth; events carry user input or asynchronous
completions, actions change state, and UI rendering consumes state. UI does not
own state or effects. App-core modules must not import `src/ui`.

Provider capabilities determine available operations. Shared queue, radio and
playback reducers must not issue provider-specific HTTP requests. Use the source
boundary, retaining library and operation identity on successes and failures.
Task leases cancel superseded work. Never convert an error into a successful
empty catalog or overwrite a newer selection with an older response.

Avoid adding wrappers or traits without a concrete benefit. Reuse shared models
and preserve their serialized compatibility with existing non-Plex caches.

## Libraries, views and caches

F3 switches libraries; F2 → Libraries manages sources/accounts. Startup opens the
saved selection, then another saved source if needed; no source opens normal
browsing with an add-library prompt. Startup must not start playback.

Settings has Libraries, Textamp and About. Libraries is a single list
grouped by provider/account, with Add library and inline connection forms. Enter
opens per-library options (Make active, connection/AI and cache size/clear/re-scan).
F3 remains switch-only. Cache jobs must not switch the active source or playback.
Sidebar visibility belongs in Textamp settings. Theme artwork uses the active
theme unless a different theme row has content focus. Cache sizes are measured
on disk per library (including AudioMuse), not estimated from in-memory structs.

Browse uses Miller columns. Ctrl+L opens Library, Ctrl+G genres, Ctrl+O folders,
Ctrl+F search, Ctrl+U queue, Ctrl+N Now Playing, F1 help. Tab switches Library /
Now Playing focus, including split mode. `:` is the command palette; `:q` quits.
Keep the help text authoritative for all other bindings.

All sources have persistent metadata caches: server catalogs, directory trees and
visited album listings. Cached startup comes first, weekly refresh plus manual F5.
AudioMuse shares that policy. Local/WebDAV tree scans are bounded and keep only
directory structure; leaf track listings remain lazy. Never download audio as
part of a cache scan or replace a complete tree with a partial failed scan. Failed refreshes preserve the last usable cache. These are metadata
caches, not offline music downloads.

Honor XDG overrides before platform defaults. macOS configuration, credentials and
logs default to `~/Library/Application Support/textamp`; caches to
`~/Library/Caches/textamp`. Credentials are separate private files. Do not print
secrets, alter personal credentials during tests, or delete music when removing
an account/library.

## Installation

When asked to update the installed build, replace `~/Applications/textamp` only
after verification. Keep the launcher separate. Do not retain old binary backups
or commit/publish without instruction.
