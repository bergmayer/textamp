# Engineering review — 2026-09-04

This covers the initial review and the requested follow-up implementation.
Both baselines include the user's existing uncommitted changes. No commits,
history rewrites, publication, or account mutations were performed.

## Findings and implementation

### Reliability defects

- **Failed refreshes looked successful.** Request failures previously became
  `changed = false`, after which the reducer stamped the category fresh.
  Category completions now carry a Result, library generation, and operation
  ID. Only the current successful result changes freshness. Failures clear
  progress, preserve cached data, and reach the error/connection surface.
  Replacing a request cancels its predecessor.

- **Startup, foreground loading, and refresh disagreed.** They duplicated
  artist/playlist requests and reduction, allowed old same-library responses
  to win, sometimes retained empty-but-authoritative results, and disagreed
  about cross-library playlists. Artist/playlist loading now has one request
  path and reducer. All three entry points load server-wide audio playlists.
  `RefreshCategory` also identifies preload work; the redundant
  `PreloadType` enum and its conversion table are gone. Album-artist refresh
  is explicitly the same underlying artist operation.

- **Cache failures and ordering were unsafe.** Periodic writes now report
  their actual result; failed writes restore the dirty flag. Completion is
  library-scoped, and edits made after a snapshot remain dirty. Periodic and
  quit snapshots use one field mapping, cloning for periodic saves and moving
  large collections on quit. In-process revisions prevent a late older
  snapshot from overwriting a newer save or undoing an explicit cache clear.
  Revisions are not serialized, so the disk format remains compatible.
  Quit reports final-write failures/timeouts with a nonzero process result.

- **Incremental folder crawling could erase valid data.** A partial crawl
  cannot establish that unseen folders were deleted. It now only merges
  results; the existing age policy and explicit clearing remain. Four
  concurrent request futures replace one task per queued folder behind a
  semaphore. An owning task lease replaces the separate active/cancel flags,
  cancels in-flight requests, and is dropped on stop/library change. Request
  IDs reject batches already queued by an older crawl.

- **Search cancellation and errors were incomplete.** Shortening/clearing a
  query invalidates pending results; reopening Adventure does not reuse old
  request IDs. Main and Adventure searches share transport and retain errors
  instead of converting them to empty results. Replacing search state cancels
  the request, including its debounce delay.

- **Radio responses could interrupt newer playback.** Playback-context
  generations now scope station/refill results. Station navigation has its
  own generation: backing out must reject a late drill response without
  abandoning an active radio refill. Starting radio before visiting Stations
  also no longer slices an empty navigation vector. Error/empty-result paths
  clear fetching state.

- **Shutdown/task failures lacked ownership.** The terminal event-loop session
  owns application effects, propagates ownership to descendants, removes
  completed handles, and cancels outstanding async work on exit/error.
  Async/blocking-worker panics reach the event loop as fatal errors rather
  than leaving a permanent spinner. Accepted blocking work is scheduled
  immediately, counted, and given a bounded shutdown drain. Cache writes have
  a five-second exit deadline; other accepted blocking work has a three-second
  drain. Missed deadlines are reported, not treated as successful persistence.
  Native actor/decoder joins are bounded and log a stuck/panicked worker.
  Shutdown signaling does not depend on space in the audio command mailbox.
  Rust cannot safely terminate an arbitrary blocked native/filesystem call.

- **Prefetch crossed layers and outlived its context.** Plex URL resolution
  and orchestration moved from `audio/cache.rs` into app helpers. Audio no
  longer imports Plex models/client code. Session ownership covers prefetch
  tasks, and cache-generation changes immediately cancel waiting/downloading
  work. Admission of completed bytes is ordered against cache flushing.

Other boundary fixes from the initial review remain: terminal input errors
terminate with context, resize events survive a full inbox, invalid-config
backups must succeed and use unique names, and NaN/infinite volume is rejected.

### Structural simplification

- Rendering accepts immutable `AppState` and returns layout/marquee feedback.
  Only the terminal adapter installs feedback after a successful draw.
  Render-local mutation and artwork caches remain renderer-owned.
- Terminal ownership and crossterm/ratatui glue live in `src/tui.rs`.
  Command-palette behavior, hit-region data, and scrollbar calculations live
  in app core. App-core source no longer imports UI implementation modules.
- Browse rendering derives its category/filter/selection settings from state,
  removes dummy zero-sized rectangle arguments, and shares category setup.
  Scrollbar hit testing passes rectangle/geometry values rather than four
  independent coordinates.
- Queue reorder operations share one mutation that keeps the playing index
  attached to its track. Rotation touches only the affected range, so adjacent
  keyboard moves remain constant-time. Multi-selection preserves relative order and stops
  at boundaries. Radio undo stores either queue contents or radio contents,
  not duplicate copies of both.
- The write-only legacy `RadioState` disappeared. Its never-cleared fetching
  flag also caused unnecessary redraws. The unreachable old radio-search
  popup had no constructor/open action; its state, renderer, key handler,
  search actions, and modal exceptions were removed. The supported Artist
  Radio picker and station paths are retained.
- Removed unused `PlexService`, `QueueManager`, `PlaybackService` methods,
  `PreloadService`, `ConnectionParams`, `CacheService` snapshot wrappers,
  obsolete uncompiled app files, the uncompiled old now-playing bar, and the
  unused direct `tokio-stream` dependency. The live shuffle and folder-cache
  selection operations are plain functions.
- The TUI uses the shared iterative dispatcher, eliminating its duplicate
  boxed-recursive router.
- Repository-wide rustfmt and Clippy debt is resolved without blanket lint
  allowances. Large Track action payloads are boxed; unnecessary cloning,
  borrowed Vec arguments, redundant checks, and duplicate branches were
  corrected. Formatting expands many formerly long lines: physical line
  count increased during this follow-up, while Lizard's source-token count
  decreased from 334,122 to 328,875. Line count is not the success criterion.

Public Rust module/export paths changed where obsolete implementations were
removed or terminal code moved. Repository consumers were updated; unknown
external crate consumers would need to migrate. Supported application keys,
configuration names, browsing behavior, and playback semantics are retained
except for the identified defects above. No shortcut/help changes were
needed for the internal refactors; README documents the verification tools.

## Execution and ownership

| Path | Source of truth / owner |
|---|---|
| Startup | Binary holds the process lock through runtime shutdown, initializes config/stored identity/audio before raw mode, and owns terminal restoration. |
| Authentication | PlexAuth/stored identity feed connection-scoped events. AppState owns the installed account/server context. |
| Configuration | Terminal adapter owns Config; app state contains display-relevant preferences. Serialized revisioned writes protect newer settings. |
| Library | AppState owns live data and navigation. Disk caches are startup inputs/snapshots, not competing live state. Category request slots own in-flight loads. |
| Navigation/UI | AppState owns cursor, columns, and scroll pins. The renderer returns geometry; input consumes the last successfully installed geometry. |
| Playback | AppState owns requested queue/radio transport. AudioPlayer owns network cancellation and the native actor; the actor owns device/decoder resources and observed playback state. |
| Background work | TaskSession owns app effects and descendants. Search/folder/category leases additionally cancel superseded operations. Playback/cache generations own their cancellation boundaries. |
| Persistence/shutdown | One snapshot mapping, serialized revision-aware atomic cache writes, explicit Result propagation, bounded blocking/native shutdown, and terminal guards. |

## Complexity measurements

Lizard 1.24.0, `uvx lizard src -l rust --csv`, was run with the same method
on the original tree, the follow-up baseline, and final source. CCN is a
lexical function-level estimate, not a compiler control-flow graph. It does
not measure macro expansion, asynchronous interleavings, ownership, or
correctness. The scan includes source-local tests and uncompiled files;
integration tests in `tests/` are excluded. File length was not substituted
for cyclomatic complexity.

| Function/group | Original | Before follow-up | Final |
|---|---:|---:|---:|
| Main event reducer | 468 | 467 | 442 |
| Queue dispatcher | 174 | 174 | 147 |
| Settings dispatcher | 130 | 130 | 129 |
| System dispatcher | 104 | 104 | 95 |
| Search dispatcher | 95 | 93 | 87 |
| Browse layout/render setup | 77 | 77 | 66 |
| Miller-column renderer | 113 | 113 | 113 |
| Folder crawler launcher | 24 | 21 | 18 |
| Periodic snapshot/save | 19 | 19 | 10 |
| Shared snapshot mapping, new | — | — | 9 |
| Shared queue move, new | — | — | 9 |
| TUI action router | 15 | 1 | 1 |

Aggregate detected functions: **1,615 → 1,509 → 1,504**.
Summed CCN: **7,924 → 7,664 → 7,539**.
Functions above CCN 10: **149 → 146 → 144**; above CCN 20:
**59 → 59 → 58**.

The snapshot/move rows explicitly expose newly shared logic; their callers'
reductions are not claimed as pure elimination. Ownership, cancellation, and
ordering checks add code and branches for real failure boundaries. Initial
aggregate reductions were mostly dead/duplicate implementation removal;
the follow-up also removes repeated live loading and queue-control paths.

## Verification

Baseline: release build passed with zero warnings; 171 unit tests and eight
doctests passed. Repository formatting failed; strict Clippy reported 182
library / 196 library-test findings. Cargo's shell shim misrouted some
subcommands, so verification uses the installed stable toolchain bin directory
first on PATH.

Current final-source verification:

- `cargo test`: 162 source-local unit tests, 21 integration tests, and eight
  doctests pass. Tests removed with unused implementations are not counted as
  retained coverage; replacement/live-path regressions are included.
- `cargo clippy --all-targets -- -D warnings`: passes with zero warnings.
- `cargo fmt --all -- --check` and `git diff --check`: pass.
- Live Plex: saved-account authentication and the saved server route pass;
  library, album/playlist tracks, search, actual Down-key input, and loaded-data
  headless rendering pass. The first plex.tv request had a connection failure;
  retry succeeded. These checks send no playback/scrobble/remote commands.
- Local audio: the silent WAV smoke test passes on CoreAudio, including play,
  pause, resume, buffered seek, and natural completion.
- `cargo build --release --bin textamp`: passes with zero warnings; a cached
  recheck confirms the final source builds successfully.
- Actual release-executable pseudo-terminal checks: login renders, Ctrl+C and
  SIGTERM each exit successfully, and terminal attributes are restored exactly.

Regression coverage includes refresh failure/freshness, same-generation
ordering, shortened/reopened searches, stale cache completions, empty
authoritative categories, partial folder crawling, immediate cancellation,
task panic supervision, queue reordering/undo, stale station responses,
station navigation versus radio refill, snapshot equivalence, late cache
writes/clear barriers, prefetch invalidation, read-only rendering, and click
scroll-pin behavior. Headless rendering covers principal views at small and
normal terminal sizes.

Raw evidence and both working-tree snapshots:
`/tmp/textamp-review.WOhBBE/` and `/tmp/textamp-followup.DB30J4/`.
Deleted source is recoverable from Git or these snapshots.

## Deliberately retained design and verification limits

Large event/settings/render matches still exist. These route many supported
actions and explicit state transitions; slicing them into many tiny handlers
would mostly move branches and make ordering harder to follow. The genuine
duplicated/stateful paths found during this review were changed. No generic
event framework or new dependency was introduced.

The local regression suite cannot prove audio fidelity across every codec,
sustained playback over an unstable server, remote-player behavior/scrobbling,
or every terminal graphics protocol. Live-service/hardware results above are
reported separately from source and automated-test coverage. There are no
known failing local checks intentionally deferred; external verification
limits are not represented as completed tests.

## Post-install correction: Random Album Radio

The user reported a 404 when choosing Random Album Radio from the command
palette. The palette translated `PlayStation` into `StartPlexRadio`, passing
a station URL where the artist-radio API requires a metadata rating key.
Alt+R already used the correct station dispatcher. The initial review and
client-level station tests missed this input-to-dispatch mismatch.

Palette station selection now dispatches `RadioAction::PlayStation`, retaining
the original station URL for loading and continuous refill. Its command payload
no longer duplicates the station title. No shortcuts or layout changed.

`tests/radio_palette.rs` reproduces the incorrect action before the fix and
checks both palette/Alt+R equivalence and the complete palette -> dispatcher ->
HTTP -> queue transition. The live `--random-album-radio` check passed against
the saved Plex server with nine tracks; it deliberately stops before audio
playback or reporting listening activity. This supplements, rather than
extends the claims of, the earlier audio smoke test and complexity snapshot.

After this correction: 185 unit/integration tests and eight doctests pass;
strict all-targets Clippy, formatting, and diff checks pass; the optimized
`textamp` release build completes with zero warnings.

## Related routing fixes after the targeted audit

The follow-up audit reproduced additional failures that the earlier suite did
not cover. These are corrected together:

- Palette radio categories now open choices and offer a Back command. Cached
  and asynchronous choices update an open palette without reopening a cancelled
  one. Filtered Mood/Style/Decade stations are classified as playable leaves.
- Synthetic DJ, Remix, and launcher rows are excluded from radio entries;
  native commands handle those actions. DJ/Remix commands are available in both
  Queue and Now Playing, and Artist Radio opens the artist picker.
- Artist-radio refill strips the internal artist marker and uses the same
  metadata-radio API as startup, rather than building an invalid station URL.
- A single visible-list resolver supplies palette track targets and playback
  tails. This removes the incomplete per-category playback routing table and
  prevents retained Library/Similar selections from targeting hidden tracks in
  Queue, Now Playing, and unrelated views. Artist-bio/Open-in-Library context
  also respects the visible view.
- Palette Refresh delegates to the F5 current-view implementation. Palette
  removal uses the active queue/radio list and preserves multi-selection and
  radio-to-queue undo behavior.

Ten focused tests in `tests/palette_routes.rs` cover all browse categories,
queue/radio contexts, native command availability, refresh equivalence,
removal, the artist-refill HTTP request, actual rendered palette mouse input,
async/cached station navigation, leaf classification, and cancellation.
No shortcuts changed; the shared help text and README explain station choices.
The original complexity table remains a dated review snapshot, not a claim
about the expanded regression-test suite. No claim is made that every remaining
application path is bug-free.

Verification for this set: 195 unit/integration tests and eight doctests pass;
strict Clippy, formatting, and diff checks pass. The release build completes
with zero warnings. Actual release-executable Ctrl+C and SIGTERM smoke checks
restore terminal attributes and exit successfully. Live Plex checks returned
305 Mood, 994 Style, and 14 Decade choices, passed Back navigation, and loaded
eight Random Album Radio tracks without playing or reporting playback. Artist
refill is covered by a local HTTP fixture, not a live artist-radio session.


## Plex robustness follow-up — 2026-09-04

This pass keeps Plex and the existing app/service/audio boundaries. The baseline
for this increment was the already-modified working tree, not Git HEAD:
195 unit/integration tests and eight doctests passed before the edits.

Confirmed defects and corrections:

- **Refill could skip a playing track.** Being on the last buffered row was
  incorrectly treated as waiting for more tracks. `RadioRefill` now distinguishes
  idle, prefetching, and waiting. Local and remote Next share one advancement
  path. Pause, seek, previous/jump, Stop, and new playback contexts supersede
  pending automatic advancement. Refills deduplicate within a response as well
  as against existing tracks. Empty/duplicate-only results report an error
  without starting an automatic retry loop; Next is an explicit retry.
- **Failed station selection discarded working playback.** Startup now prepares
  a candidate while retaining the current queue/audio. Only nonempty successful
  results commit the new station, reset DJ state, and switch playback. Old
  results are rejected after another station, Stop, pause, or a context change.
  This deliberately changes station-selection behavior: current music continues
  during loading. It does not promise rollback after a subsequently failing
  media download/decoder; those failures still use normal playback recovery.
- **Startup and refill disagreed about source identity.** Both now consume
  `RadioSource::Artist` or `RadioSource::Station`; the `plex_radio:` convention,
  duplicated artist-startup flow, and duplicated refill completion handling
  are removed. Startup and refill each have a 30-second operation timeout.
  Time Travel refills use the source station's library, not mutable browse state.
- **Category navigation could start unexpected playback.** Empty categories
  now stay empty categories instead of falling through to a PlayQueue request.
  Category completion/failure and loading state are independent of playback
  preparation. Cached drills clear superseded loading indicators, and library
  changes clear loading flags for discarded work.
- **Malformed station paths silently chose library 5.** Station requests now
  validate library/path/filter components before HTTP. Category responses with
  missing containers or invalid directory-list shapes fail instead of appearing
  to be successful empty responses.
- **HTTP errors exposed server HTML.** Authentication, access, missing
  item/endpoint, rate-limit, server, malformed-data, and missing-media failures
  now have actionable user-facing messages. Existing bounded GET retries and
  connection-route recovery remain; HTTP authentication/404 errors do not
  trigger connection-route recovery.
- Removed unused radio history and ancestor-indicator state (the latter was
  written but never read). Removed eager artist-artwork loading during station
  preparation; normal track artwork loading remains, preserving the old
  artwork until the candidate succeeds.

The palette now exposes **Stop Playback**, rather than treating “stop” as an
alias for Play/Pause. README and the shared Help content document cancellation,
loading, and refill behavior. Existing key bindings and screen geometry are
unchanged; the obsolete `render_shortcuts()` function named in AGENTS.md is
not present in the current renderer. Queue titles show pending station loading
without presenting the candidate as the already-active queue.

### Incremental complexity measurement

Same command on both snapshots: `uvx lizard src -l rust --csv`, Lizard 1.24.0.
These are lexical cyclomatic counts, not file lengths or nesting estimates.

| Function | Before | After |
|---|---:|---:|
| Radio dispatch | 40 | 29 |
| Shared station startup (new) | — | 4 |
| Local playback dispatch | 36 | 36 |
| Remote playback dispatch | 27 | 22 |
| Radio refill | 9 | 6 |
| Shared radio advancement (new) | — | 4 |
| Above group, including new helpers | 112 | 101 |
| Main event reducer | 443 | 445 |
| Plex station queue creation | 10 | 16 |

The selected startup/playback/refill group shrank from 1,136 to 936 measured
non-comment lines because actual duplicated workflows were consolidated.
New cancellation/error and external-input validation branches intentionally
increase some counts. No claim of a whole-program complexity reduction is made:
Lizard's Rust parser misses the tuple-returning `station_parts` helper and
several following functions, so its whole-tree totals are not comparable.
Embedded unit tests are included by a `src` scan; integration tests are not.

### Verification and remaining limits

Sixteen new integration tests in `tests/radio_lifecycle.rs` cover last-track
prefetch, waiting/advancement, transport cancellation, duplicate batches,
overlapping requests, empty/failed startup, 401/404/malformed responses,
actual network timeouts, independent/cached navigation, library changes, and
palette Stop. Existing radio/palette tests were updated to use typed sources.

Live saved-server checks passed Mood (305 choices), Style (994), Decade (14),
Back navigation, and Random Album Radio (25 tracks on the final rerun). No live music or playback
reports were sent. Authentication used the saved server token; the Plex account
endpoint was intentionally skipped.

Native audio verification did **not** pass in this environment. Debug and
release smoke tests opened CoreAudio and exercised play/pause/seek, but the
device position never advanced beyond the seek offset (200 ms), with zero PCM
underruns. A direct Rodio decoder/sink, bypassing Textamp's buffering/decoder
pipeline, also stalled at zero. The smoke tool now has useful failure diagnostics
and a silent `--direct` isolation mode. This is an unresolved native-output
verification limitation, not evidence that the radio fixes caused it; no
speculative audio-driver change or system audio-service restart was made.

The existing caching, connection selection, task supervision, and playback
failure limits are retained. Live sustained radio/refill playback, remote-player
hardware, server-version-specific Sonic capabilities, and native audio output
are not certified by these tests.


Final gate: **211 unit/integration tests and eight doctests pass**, with no
ignored tests. Strict all-target Clippy, formatting, and diff checks pass.
The final release build completed with zero warnings. That exact executable
rendered the login screen and exited successfully on both Ctrl+C and SIGTERM,
restoring terminal attributes exactly. The native-audio smoke failures above
remain explicitly excluded from the passing gate.

Installed atomically to `/Users/bergmayer/Applications/textamp`, without an
old-build backup. Installed/release SHA-256:
`fb5f038b343a00d63b40b70bd9471763d16aad016eed8fde603ac1546c82cbde`.
No commits or publication were performed. A running old process must be quit
and relaunched to use the new executable.

## Navidrome integration — September 5, 2026

Navidrome is now a separate OpenSubsonic transport (`src/navidrome/`) with an
application adapter (`src/app/sources/navidrome/`). It does not use Plex
authentication or send Navidrome IDs to Plex. The shared player, queue, Miller
navigation, visualizers, and completion reducers are reused; native provider
DTOs are converted to the existing display models at the adapter boundary.
`ActiveSource` replaces the ambiguous active-folder flag with mutually exclusive
Plex, folder, and Navidrome variants. No generic provider framework was added.

Accounts have distinct IDs and private credentials; music-folder selection is
remembered per default source. Catalog loading is bounded and transactional,
and source/request generations reject stale completions. Writes are serialized
by operation family instead of cancelling an in-flight mutation. Album refresh
updates its open column, and account-wide playlist navigation can fetch albums
and artists outside the selected catalog. Failure in the shared similar-tracks
pane is retained until refresh, fixing a request retry on every UI tick.

The full test suite, formatting, strict all-target Clippy, and warning-free
release build passed. Twelve Navidrome protocol/state regressions cover auth,
malformed responses, pagination, library isolation, stale results, catalog
failure, browsing/order, provider biographies, similarity, sonic paths, and
queue writes. Two additional opt-in tests passed against an isolated official
Navidrome 0.63.2 server with generated silent FLACs: catalog and playlist CRUD,
favorites, ratings, scrobbles, saved queues, native playback, paused seeking,
waveform/spectrogram generation, saved-position restoration, and stopping.
The final release passed isolated PTY tests for Navidrome and local-folder
startup, navigation, playback controls, account setup/masking, multiple accounts,
private persistence, shutdown, and exact terminal restoration.
The read-only saved-server Plex regression also passed: authentication, three
libraries, catalog loading (18,462 artists / 44,004 albums / 23 playlists),
playlist/album fetches, search, real Down-key navigation, and headless rendering.
It did not contact the Plex account endpoint, play music, or submit play history.

These current native Navidrome fixture results supersede the earlier audio
environment limitation for this tested path, not for every audio device or
Plex remote player. Sonic extensions were verified with protocol fixtures,
not a configured AudioMuse-AI installation. Personal Navidrome credentials,
remote TLS/reverse proxies, large production catalogs, and external metadata
agent availability were not live-tested. Offline Navidrome catalog persistence
and administrative/video APIs are outside this music-player integration.

Installed atomically without an executable backup at
`/Users/bergmayer/Applications/textamp`; release and installed SHA-256 match:
`c7a5b42168b5bc3164a83b749e1b3f3cee0839c212ea5abc8dbf8591875b4090`.
The visualizer source rollback patch still reverse-checks successfully.
No commit or publication was performed.

## Production Navidrome and genres — September 6, 2026

Added the supplied Navidrome account as the default source, preserving existing
sources and storing its password only in the private credential file (0600).
Read-only production checks passed authentication, library/artist/album/playlist
retrieval, search, genre album queries, artwork decoding, and muted native
streaming, pause, seek, and stop. No server playlists, ratings, play history,
or analysis jobs were changed.

One full application catalog load completed in 123.6 seconds: 19,014 artists,
45,165 albums and 499,536 tracks. Real Ctrl+G, secondary-genre album membership,
ordered album tracks and headless rendering passed. Subsequent bulk reads hit
an interrupted response and a briefly refused connection; the server became
reachable again. These failures are not attributed to the scan without server
evidence. Further bulk checks were paused instead of continuously retrying.

Fixed confirmed genre defects: consume OpenSubsonic's complete `genres` array
(use the legacy single genre only when the array is absent/empty), discard blank
or repeated tags, and sort names without case-sensitive grouping. Preserve genre
labels and do not invent separators, artist genres, or AI moods/styles. An empty
Navidrome genre list no longer loops through synchronous load/refresh actions;
an already-open genre view receives the completed catalog. Interrupted read-only
API requests retry once after two seconds; mutations, API failures, malformed
responses and size-limit violations are not automatically retried.

All 15 Navidrome fixture tests, the full existing suite, formatting, and strict
all-target Clippy passed. These include multi-tag membership, legacy fallback,
empty/preloading genre views and bounded read retries without duplicate writes.
The final source changes were verified with fixtures; they do not imply a final
production catalog load passed despite the server interruptions above.

Navidrome advertises `sonicSimilarity`. AudioMuse's authenticated dashboard
showed active analysis workers; a direct lookup returned no indexed matches for
the sample track, while the corresponding Navidrome sonic lookup returned API 0.
Sonic results are not certified during this incomplete analysis. No AudioMuse
settings or jobs were changed, and its credentials/session were not saved by
Textamp. The separate AudioMuse URL is not required by Textamp's existing
Navidrome sonic integration.

Final release build succeeded without warnings, and the exact executable passed
the isolated folder PTY/navigation/shutdown regression. Installed atomically at
`/Users/bergmayer/Applications/textamp` without an old-build backup. Installed and
release SHA-256: `6cec1236329e21ec90c4b9173a14afade0b55d83c548f7fe46d9fd262964c4fd`.
Formatting, diff checks, and the visualizer rollback reverse-check also passed.
