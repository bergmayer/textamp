# Radio, DJ parity and backend capabilities — September 7, 2026

## Findings and changes

This increment follows the provider-boundary audit and the requests for folder
radio, Navidrome/Plex feature parity, and a clearer contract for future backends.
The baseline checkout already contained the earlier multi-library work; unrelated
changes were retained. No commits, server playlists, scans or file tags were changed.

1. **Radio lifecycle was duplicated by provider.** Plex start/refill and Navidrome
   start/refill separately managed timeouts and completion events. A single runtime
   now prepares a provider-owned request, owns its abortable task, and commits its
   normalized result. Starting, refilling, stopping and changing libraries share
   the same rules. A pending new station no longer competes with an old refill.
   Current audio/queue survives failed or empty startup; obsolete completions are
   rejected by library/radio generation. Dropping the task also cancels actual I/O.
2. **Folder radio was blanket-disabled despite needing no analysis.** Local/WebDAV
   now offer Library Radio and Random Album Radio. A leaf directory with audio is
   an album, including a leaf root. An album is returned whole, in natural filename
   order; already queued albums are excluded whole rather than truncated. Library
   Radio includes loose files too. Only directory listings are requested. WebDAV
   reuses the browser's source-scoped persistent cache; local listings remain fresh.
3. **Navidrome sonic functions depended solely on the plugin.** One recommendation
   adapter now serves stations, DJs, remixes, Similar and Sonic Adventure. It tries
   native sonic endpoints first (eight-second limit), then the configured AudioMuse
   connection for a missing/broken/empty native result. Each analysis lookup has a
   thirty-second deadline. AudioMuse sessions are initialized once per operation,
   including remembering sign-in failures within a multi-track operation. No scan
   or playlist-creation endpoint is called. Mood choices/centroid indices come from
   AudioMuse itself. Exact IDs, library membership, path order/endpoints and duplicate
   constraints are checked; no title matching or random sonic fallback is used.
4. **Capability policy was scattered across provider-name checks.** Backends now
   declare supported operations in `library/capabilities.rs`; standard station
   menus, command visibility, Now Playing DJ rows and output settings consume the
   declarations. Rendering, keyboard and mouse activation share the same rows.
   Support is distinct from the user's per-library Sonic preference and from a
   ready/reachable analysis index. Disabled Sonic prevents direct AudioMuse and
   native sonic requests, including fallback from otherwise non-sonic artist modes.
   Artist Radio and Groupie retain a logged same-artist catalog fallback if the
   related-artist service fails; sonic-only stations never substitute random music.
5. **Shared domain data lived in the Plex module.** Artist, album, playlist, track
   and station metadata now live under `library/`; Plex retains wire response
   envelopes and compatibility re-exports. No parallel copies or cache migration
   were introduced. An unused `Album::from_track` helper that manufactured Plex
   paths and an unused serde pass-through module were removed.
6. **Radio event handling also owned playlist paging.** These are now separate
   event domains and reducers. Plex station-category and DJ effects moved into its
   adapter; shared radio dispatch no longer implements Plex HTTP operations.
   Folder playback preparation now updates the active radio track, not an inactive
   normal queue, including when opening the audio output fails.

Removed duplicate Navidrome start/refill/task dispatch, the old shared Plex
start/refill branches, obsolete radio task-slot names and three nonfunctional
palette umbrella entries. Actual station/DJ/remix commands and their bindings remain.

## Feature parity

| Feature | Navidrome implementation | Local/WebDAV |
|---|---|---|
| Library, Random Artist, Random Album | Shared catalog recipes; full album order | Library and leaf-folder albums |
| Deep Cuts, Time Travel, Decade | Play counts and release-year metadata | Not inferred from filenames |
| On This Day | Full album release dates | No invented dates |
| Style/Genre, Mood | Actual tags; additional labeled AudioMuse mood choices | No inferred artist/genre tags |
| Artist Radio/Mix, multiple artists | Catalog artists, server affinity; analyzed seed fallback when enabled | No invented artist identities |
| Album Mix, Sonic Radio | Native sonic similarity or direct AudioMuse | Unavailable |
| Twofer, Contempo | Same artist / same decade | Unavailable without metadata |
| Groupie | Same/related artists; cached same-artist fallback if discovery fails | Unavailable |
| Gemini, Freeze | Native sonic similarity or direct AudioMuse | Unavailable |
| Stretch, Sonic Adventure | Validated native or AudioMuse sonic paths | Unavailable |
| Twofer/Gemini/Stretch/Doppelganger remixes | Same candidate adapter; shared queue edits/undo | Existing non-analysis queue edits only |
| Playback, seek, visualizers, queue navigation | Shared | Shared |

These recreate the supported behaviors, not Plex's proprietary recommendation
ranking or taxonomy. Navidrome needs matching metadata; sonic functions need indexed
tracks. Enabling a feature does not fabricate missing results. Existing Plex station
selection, playback and DJ algorithms remain in its adapter.

## Adding a backend

```text
Backend declaration + adapter
    ├── capabilities → station catalog / palette / settings / sidebar rows
    ├── normalized catalog or folder listings → browse / metadata radio recipes
    ├── scoped playable tracks → shared queue / audio / analysis / visualizers
    └── radio request → shared task / timeout / commit / refill lifecycle
```

Implement transport/authentication and convert responses to `library` models.
Declare capabilities, add the explicit source/track identity variant and implement
its source dispatch and radio request. Reuse `services/radio` for catalog recipes or
`library/radio` for directory traversal; an analysis adapter supplies scoped candidates
and paths. Use source-scoped cache tickets and generation-bound completion events.
Run the same station/DJ/input/lifecycle contracts with a fixture transport. Missing
provider requests must fail explicitly, never fall through to Plex.

This is a compiled Rust adapter boundary, not a runtime plugin loader. Declaring a
capability does not magically implement its HTTP protocol. Account setup, browse
loading and media preparation still require backend-specific integration. Several
Plex browse/queue effects remain in core handlers behind the explicit source gate;
they were not mechanically relocated merely to rename a directory. A new source
must implement its requests before passing through shared reducers. The radio
lifecycle and feature presentation no longer require such provider overrides.

## Complexity measurement

Lizard 1.24.0, same command before and after: `uvx lizard src -l rust --csv`.
The baseline is this turn's pre-edit checkout, not Git HEAD (which predates many
user changes). This is Lizard's Rust cyclomatic estimate: lexical decisions, not
rustc-expanded macros, coverage, or a proof of architectural quality. Closures and
pattern alternatives follow Lizard's counting rules. Test functions under `src`
are included in repository-wide totals; integration tests under `tests` are not.

| Hotspot | Before CCN | After CCN |
|---|---:|---:|
| `events::handle_app_event` | 454 | 400 |
| `dispatch_radio::dispatch` | 27 | 26 |
| `helpers::playback::fetch_more_radio_tracks` | 8 | Removed; callers use shared refill |
| Navidrome `radio::launch` | 10 | Removed |
| Shared radio task launch | — | 3 |
| Navidrome recipe selection | 16 | 17 |
| Navidrome DJ candidate selection | 13 | 12 |
| Navidrome DJ request orchestration | 6 | 4 |

The event-router decrease mostly reflects meaningful ownership separation, not
eliminated decisions: new radio and playlist reducers have CCN 15 and 43. The moved
Plex diversity selector remains CCN 20. Added directory traversal, fallback validation
and tests increase total program complexity; no overall complexity reduction is
claimed. The substantive simplification is one lifecycle instead of two, one
recommendation path for several features, and no copied domain models.
Repository totals (including in-source tests): 1,839 functions / summed CCN 9,179
before; 1,881 functions / summed CCN 9,326 after. Summed CCN is not a single
application control-flow graph; it grows with tests and function count too.

## Verification

Baseline: 399 tests passed, four ignored; formatting, Clippy and release build passed.
Final: **410 tests passed, zero failed, four ignored** (including eight doctests).
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
`git diff --check`, and `cargo build --release --bin textamp` passed. The release
build had zero warnings. Source/test/example checksums remained unchanged through
the final verification pass.

The four intentionally ignored tests require live Wikimedia, native Music app
automation, native Navidrome audio, or a dedicated writable Navidrome fixture.
They are not counted as passed. The visualizer contract compares actual visualizer
geometry/content across providers and separately checks the capability-dependent
sidebar rows; unsupported DJ controls need not appear in a folder sidebar.

Installed atomically at `/Users/bergmayer/Applications/textamp`, without an old-build
backup. Build and installed SHA-256 both:
`1134d7282b9c6d5f86039f389d0b9f5e76dfb7fedac1524d332ed0db1f57581f`.
The adjacent `Textamp.app` launcher continues to use that binary. It was not launched.

Focused contracts cover local leaf-album ordering/exhaustion, WebDAV listing-only
requests and persistent offline cache reuse, startup/refill/stop/library changes,
file metadata in radio mode, all six native DJ modes, Groupie failure fallback,
all existing remixes, shortcut/menu parity, direct/native analysis fallback,
strict path/library scope, malformed AudioMuse responses and capability-driven
station generation for an arbitrary new backend name. Tests use isolated HTTP
fixtures and do not deliberately start audible music or control Apple Music/AirPlay.

Directory radio samples randomized branches, not a uniform draw over every album.
Traversal is bounded to 256 directories, depth 128 and 100,000 pending directories
per request; exhausted or very sparse trees report a useful error. No full-library
scan was added. Native terminal mouse behavior/audio-device output and analysis
quality across the unfinished production scan require separate live verification.

The read-only production AudioMuse check passed authentication, the configured
Navidrome binding and analysis export, six available moods, 100 mood results,
100 sonic neighbors, and a five-track path with the requested endpoints.
CLAP and lyrics search reported disabled with zero indexed tracks. This verifies
live discovery APIs, not an end-to-end audible playback session or recommendation
quality across every track. No production scan, tag, playlist or server setting changed.

Protocol references: [AudioMuse similarity and path algorithms](https://github.com/NeptuneHub/AudioMuse-AI/blob/main/docs/ALGORITHM.md),
[similarity/mood API](https://github.com/NeptuneHub/AudioMuse-AI/blob/main/app_ivf.py),
[path API](https://github.com/NeptuneHub/AudioMuse-AI/blob/main/app_path.py).
