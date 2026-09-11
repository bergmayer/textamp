# Provider-boundary review — September 7, 2026

Scope: active-library routing in keyboard/mouse/palette entry points, settings,
catalog browsing, radio/DJ/remix effects, playback and analysis, refresh/cache,
biographies, and asynchronous source changes. This is a targeted engineering
review of the current multi-provider application, not a claim that all possible
server responses or terminal behaviors have been exercised.

## Confirmed defects corrected

1. **Random Album Radio shortcuts constructed Plex URLs for every provider.**
   The keyboard and old command-bar click route now resolve the active library's
   registered station and emit the same `PlayStation` action as the Radio palette
   used by Now Playing. Parent station columns remain searchable after drilling
   into a category. Both dispatchers use the same station-title lookup.
   The original Plex-only parity test was extended and failed on the Navidrome
   key before the fix. The shortcut is unavailable until a station exists.
2. **A missing Navidrome virtual-folder catalog could trigger a Plex fetch.**
   The shared enqueue fallback now permits Plex folder requests only for Plex.
   Missing Navidrome tracks produce a refresh instruction and leave the queue
   unchanged. The normal catalog-backed path remains local and fast.
3. **Saving a Navidrome radio station read the inactive queue.** Both provider
   implementations and the save dialog now use `AppState::playback_tracks()`.
   Navidrome rejects blank playlist names before making a write request.
4. **Settings exposed Plex output/discovery controls on other providers.**
   One ordered `TextampSetting` list now drives rendering and activation.
   Plex gets local/remote output and player discovery; Plex/Navidrome get
   streaming quality; plain folders get neither server-only control group.
   Themes, artwork and external search remain available everywhere.
5. **The settings mouse mapper duplicated an obsolete layout.** It did not
   account for scrolling or newer controls. Actual rendered hit targets replace
   the Textamp-panel row arithmetic. Clicks pin the viewport; keyboard navigation
   releases it. Wheel, Home/End and page navigation work with this list.
6. **Folder-library Clear Queue was disabled by the server-playlist save gate.**
   Those capabilities are now independent. The obsolete Ctrl+W-as-Save command
   descriptor was removed; Ctrl+W remains Close Column.
7. **Navidrome folder playback ignored the selected row**, starting at the first
   track instead. It now starts at the selection in displayed order. The legacy
   selected-album action also now accounts for its leading All Tracks row.
8. **Plain-folder palettes offered unsupported server-library operations.**
   Those entries are hidden, while playback, queue editing, search and
   visualizers remain. The dispatcher continues rejecting unsupported actions.
   The shared artist-pagination helper is explicitly Plex-only.

## Boundaries inspected

| Area | Shared behavior | Provider-owned behavior |
|---|---|---|
| Input and radio | `handlers/key_input`, `command_palette`, `dispatch_radio`; one station key/action | Plex station creation; Navidrome `sources/navidrome/radio` and shared catalog recipes |
| Browse/search | Miller navigation, local search/filter, track/album presentation | Navidrome intercepts album/artist/playlist/genre loads; file sources list directories; Plex uses its client |
| Playback | Queue state, transport, decode backend, seeking and meters | Track-origin-aware URL/auth handling, Navidrome stream/scrobble, Plex timelines/casting, prepared local/WebDAV files |
| Waveform/spectrogram | Shared bounded decoder and source-scoped analysis cache | Authenticated Navidrome raw stream, Plex stream, or prepared file |
| Radio/DJ/remix | Queue reducers, cancellation/request checks and metadata recipes | Navidrome similar-song/sonic-path endpoints; Plex sonic/station endpoints |
| Biography | Readable content and strict Wikipedia fallback | Plex biography or Navidrome `getArtistInfo2` first |
| Cache/refresh | Persisted source identities and retained data on failed refresh | Plex category cache, Navidrome catalog, visited local/WebDAV directories |
| Settings/source changes | One active source; generation-bound completions | Separate account credentials, library identity and Sonic preference |

`dispatch_action` routes Navidrome effects before the Plex-backed handlers.
`sources/routing.rs` now identifies requests that must be consumed by a provider:
a missing Navidrome handler produces an explicit error instead of falling through
to Plex. Data, Miller, browse, folder and radio classifications are exhaustive;
new variants in those groups require a routing decision at compile time. Other
groups mix local behavior and requests and still require review when extended.
The same classification replaces the incomplete folder queue-operation blocklist.
Artist-radio intent is now named `StartArtistRadio`, not `StartPlexRadio`; input
code does not choose a transport. No provider trait or parallel queue framework
was added.

Shared handlers that can initiate I/O were checked as well: queue folder fallback,
pagination, artwork, playback preparation, analysis, biography, refresh and radio
refills. Checking only the top-level action match would miss those paths.

Plex account discovery in the library manager is intentionally still available
while browsing a different provider: adding or refreshing saved Plex libraries is
an explicit account-management operation, not Navidrome playback/catalog traffic.

## Differences retained deliberately

- Navidrome genres are actual album metadata. Plex's proprietary artist-genre,
  mood/style taxonomy and casting are not presented as native Navidrome features.
- Ordinary Navidrome radio works from the catalog without Sonic Analysis.
  Metadata-dependent stations still require the relevant dates/tags. Missing
  metadata is not replaced with unrelated random music.
- Sonic operations use the advertised Navidrome extension and the per-library
  enable/disable preference. AudioMuse-specific discovery uses its own configured
  API. Enabling a feature does not imply the scan/model/index is ready.
- Some shared legacy model types still live under `plex::models`. They do not
  themselves make requests. Moving every type would create broad import churn;
  the provider-independent track model already lives in `library/track.rs`.
  The Plex request implementations also remain beside shared reducers in legacy
  dispatch modules. The new request boundary makes this limitation explicit;
  this is not a claim that the whole application has been rewritten into adapters.
- Folder libraries remain filename-ordered file browsers, not invented
  artist/genre databases or remote playlist services.

## Verification scope

### Complexity and scope limits

Lizard 1.24.0 (`uvx lizard src -l rust --csv`) measured the current source.
The request classification has CCN 7; the settings row model 3; settings rendering
18; the remaining settings mouse-row mapper 3; the shared random-album shortcut
1. These are lexical control-flow estimates, not measurements of async state
space, API correctness or user-interface complexity. Exhaustive enum matches and
macros are not equivalent to nested runtime decisions.

Large legacy handlers remain: event reduction CCN 454, queue dispatch 145,
system dispatch 116 and settings dispatch 112. This pass removes specific
duplicate decisions and closes provider leaks; it does not claim to have reduced
whole-program complexity. The older complexity files predate intervening user
changes and are not a valid before snapshot for this pass, so no before/after
comparison is asserted here.

### Behavioral checks

Final `cargo test`: 389 unit/integration tests and 8 doctests passed, zero failed.
Four opt-in checks remained ignored: live Wikimedia, native Music automation,
and the two dedicated live Navidrome fixture tests. Nested subprocess test runs
are not double-counted. `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --all -- --check` and `git diff --check` passed.
`cargo build --release --bin textamp` completed without warnings. The result was
installed atomically at `/Users/bergmayer/Applications/textamp`, without an
old-build backup; byte comparison and SHA-256 matched the release binary.

Focused regressions cover keyboard/palette/rendered-click radio parity, nested
station lookup, settings visibility/indexing/scrolled clicks, folder shortcut
availability, station playlist contents, selected-album and folder playback order,
and missing-catalog recovery. A configured Plex tripwire server checks that shared
Navidrome browse, queue, station, biography, recommendation, refresh and output
actions do not send it requests.

The existing suites cover native Navidrome endpoint/authentication behavior,
catalog/library scoping, genres, playlists, radio/DJ/remixes, Sonic paths,
stale completions, cache reload/failure behavior, waveform/spectrogram decoding,
seek input and all visualizer renderers. Tests use isolated servers and a
no-audio backend where applicable; server writes occur only against fixtures.

No production playlist edits, analysis jobs, credential changes or Apple TV/Music
playback were initiated. Native audio output, live Plex/Navidrome service behavior,
and actual Ghostty pointer/modifier delivery are distinct from the automated
renderer/input and mock-server checks.
