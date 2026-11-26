# Runtime boundaries and concurrency

`AppState` owns application state. Input adapters produce actions; dispatch
executes reducers and schedules provider effects. Rendering consumes state.
Domain services do not import terminal or desktop UI modules.

## Catalog search

Catalog snapshots share immutable metadata through `Arc`. Their normalized search
fields are prepared on a bounded blocking worker and reused until a snapshot is
replaced. Category results have a separate index, so they remain available while
track fields are prepared. Visible results are refined immediately as input changes;
the cancellable catalog scan fills the remaining matches. Generation, query and
catalog identities reject superseded completions. Unmatched records are excluded
before ranking, preserving ranked result limits and boolean-query semantics.

## Explicit offline downloads

`services::downloads` owns the durable audio index and atomic file installation.
`app::downloads` owns queued, transferring and failed jobs, cancellation leases,
operation identities and local playback routing. Provider HTTP stays in the
source boundary. Audio and cover retrieval overlap; at most two tracks transfer
at once. If audio fails, the transfer releases its optional cover request and
its staging file. Cancelling album resolution invalidates its lease before its
track list can enqueue new downloads. A completion with an obsolete operation
is ignored. Effective offline status is derived from manual Offline Mode and
network reachability.

Downloads use the data directory rather than the evictable cache. A track is
indexed only after its staged audio is committed. Startup excludes missing or
truncated files, and source identity separates tracks with equal provider IDs.
Removal touches only indexed copies. Clear commits the empty index first, then
attempts every indexed directory and reports any cleanup failure. Refreshing the
Downloads column retains its
sort, filter query and selected track, while invalidating obsolete filter workers.
The list summary includes the visible count and committed audio size in every
presentation.

All presentations use this same store and dispatch path. The mobile adapter uses
its own sandbox paths and projects shared state into native SwiftUI controls;
it does not create a second player or implement a separate music policy. See
[iOS](IOS.md) for platform boundaries.

## Source and request ownership

`ActiveSource` distinguishes disconnected, Navidrome and folder sources.
Provider capabilities determine the available operations. Source routing consumes
provider requests before shared reducers. An unsupported request is an error;
it cannot fall through into another provider. Account edits and cache jobs do
not activate a library or alter playback.

Library generations reject completions from a previous library. Connection
generations independently protect account validation. Replaceable tasks own
leases; dropping a lease cancels its work. Provider operation IDs reject queued
completions from a replaced request even within the same library. Async success
and failure both retain their source and operation identities.

Modal content is exclusive. Similarity requests explicitly distinguish Ready,
Loading with a request key, and Failed with an error. File preparation explicitly distinguishes Idle,
Preparing and Ready. Artwork fetch and navigation states distinguish requests
from motion. Radio refill distinguishes idle, fetching and a prepared batch.
Playback scrobbling and full-screen viewer modes have named states. The backend clock tracks each playback attempt by identity through Monitoring, Mixing and Ended, retaining its phase while paused. Biography
loading and radio preparation derive directly from their task leases rather than
maintaining a second busy flag.

## Concurrent and ordered work

Account discovery overlaps independent checks. Catalog collections load together
after account discovery establishes folder scope. Artist album tracks and
recommended track resolution use bounded, order-preserving streams. Artwork
batches run at concurrency four. Completed foreground artwork is published without waiting for its owned disk-cache write. WebDAV requests share a connection pool with per-request authentication and redirects disabled. AudioMuse metadata refresh batches are bounded;
manifest pagination remains ordered so incomplete scans cannot commit success.

Directory-tree scans maintain a bounded pool and expand completed branches
without waiting for slower siblings. Library radio also reads at most four
directories concurrently. Play Random Albums retains randomized depth-first
traversal with one listing in flight, returning a complete leaf album in filename
order. These paths request listings, never audio.

Startup reads the local Downloads index while starting the selected library.
Ordinary folder browsing publishes a listing while its accepted cache write
finishes under session supervision. Explicit refresh reports completion after
the write. Cache commits and invalidation share an ordered section so Clear
cannot be undone by an older write. A failed scan retains the last complete cache.
Source discovery precedes requests whose folder scope depends on it.

Reducers are synchronous; scheduling an owned background effect does not require
an async reducer. Actions and their follow-ups execute in order because they
mutate shared state. Audio chunks, pagination and fallback attempts remain sequential where
each step depends on its predecessor. CPU-intensive media analysis and disk work
run on blocking workers. Playback IDs protect streaming, cancellation and seeks.

## Streaming and media analysis

Remote playback writes one progressive temporary file while the decoder consumes
published bytes on its worker. Completed compressed data is retained for indexed
seeks and waveform analysis. A pending seek replaces the prior target without
replacing playback identity; pause state survives the seek. Stop and player drop
invalidate the transfer and pending backend work. Analysis waits for transfer
completion or cancellation via notifications. A failed seek clears its pending
target before publishing the error and leaves current playback in place. Body
reads have a 20-second idle timeout. Streaming storage is limited to 512 MiB per
track; analysis input is limited to 128 MiB.

Current and speculative media analysis share two CPU permits. Each permit remains
with its blocking worker even if its asynchronous observer is cancelled. Next-track
waveform preparation begins after the current audio transfer completes. Its
Idle, Loading, Ready and Failed states retain the next track identity; changing
that track cancels its lease. Completion retains source and operation guards.

## Cache freshness

Compact browsing snapshots retain the full catalog's original timestamp. Reading
a fresh cache does not extend its freshness or rewrite the compact snapshot.
Playlist snapshots retain provider revision, count and duration; a same-length
playlist edit therefore invalidates the cached contents. A completed warming
summary skips unchanged playlist and artwork traversal until refresh is due.
Clearing metadata or artwork invalidates that summary. Catalog fingerprinting runs off the event loop without cloning the catalog; a fresh-summary check does not publish a busy notification. Completion is published
after both playlist and artwork work finish; partial failures retain usable data
and report the incomplete operation.

## Input ownership

Text capture and modal input take precedence over global navigation. Pointer
selection pins the current viewport; keyboard navigation releases the pin.
Settings category and control focus are separate. Opening Settings, selecting
an account row or saving its connection does not switch libraries.

Provider-specific HTTP belongs under `app::sources`; shared queue, playback and
radio reducers consume normalized results. Media caches and analysis are imported
from `media`. Pure sorting, filtering and catalog lookup belong under `services`. Browse drilling,
track-context navigation, aggregate artwork requests and filter-record projection
belong under `app`, where they can consume shared application state.
Concrete enums represent the finite set of application states and supported
providers. Backend traits isolate audio implementations where substitution is
needed for native output and DummyBackend tests.

## Media analysis and desktop projection

Waveform and spectrogram analysis use exclusive Idle, Running, WaitingToRetry,
Ready and Failed phases. Progress belongs to Running; decoded data belongs to
Ready. Track keys and phase checks reject stale retries and failures. Shared task
leases keep a combined download alive until its waveform and spectral consumers
finish. Immutable analysis results use shared ownership; desktop projection retains
analysis data by its shared identity.

Playlist warming pipelines bounded playlist downloads into bounded, deduplicated
artwork requests. A slow playlist does not hold up artwork from completed lists.
Catalog persistence, playlist warming and independent analysis refresh overlap;
required persistence errors and optional warming errors retain their separate
handling.

Desktop overlay hosting distinguishes Closed, Visible and Dismissing states.
A closing window retains its identity until the application acknowledges dismissal,
so a late close cannot dismiss a newer overlay. Empty visualizers leave their
content area blank; progress and analysis failures remain visible.

Mobile startup distinguishes an absent snapshot and pending catalog from an empty
saved profile. The root list becomes visible after compact browsing metadata,
without waiting for the complete track catalog. Cached startup preserves source
freshness timestamps and does not rewrite an unchanged compact catalog. Completed
playlist/artwork warm summaries skip traversal until catalog identity or freshness
changes, or either cache is cleared. Playlist revisions include server modification
timestamps as well as track count and duration.

The decoder publishes PCM in bounded blocks. The output callback commits to a
complete multichannel frame before consuming it, or emits an entire silent frame
on starvation, so recovery cannot swap channels. Completion is observed before
checking ring occupancy to retain the final published frames. Normal logs retain
audio/network warnings for stall and underrun diagnosis.

Artwork viewport requests share one active, source-owned batch. Up to 100 newer,
deduplicated requests wait for its owned completion; when full, the backlog retains
the newest requests. Cache clearing also clears this backlog. Each completed
image publishes with that batch identity instead of waiting for the whole batch.
Mosaics are cached per library and cover specification; composition has one
process-wide permit that remains held until its blocking work finishes. Preview
profiles optimize image resizing, PNG encoding, and JPEG decoding.

## UI semantics and terminal composition

`app::preferences` defines preference identities, groups, descriptions, typed
values, defaults and scope; `SettingsAction::ActivatePreference` uses the shared
settings reducer and existing configuration keys. `app::playback_controls`
defines playback placement and track ranges. Frontends translate physical
modifiers into those semantic choices. `app::queue_controls` resolves queue-tool
ordering, capability requirements, busy state and continuous state. Queue changes
run through shared actions. `app::details` projects detail documents, metadata
order, related content and artist-document commands. Journey choices, labels,
count validation and stages are shared, as are search state and library jumps.

Ratatui controls publish typed hit targets from their rendered geometry. Tab
switches panels; arrows traverse controls inside a panel. Terminal playback
actions occupy detail content, while desktop headers may present compact actions.
Contextual terminal commands use the palette; desktop commands use context menus.
Terminal Settings is a popover; desktop Settings is an overlay in single-window mode
and a native window in multi-window mode. Native
window configuration and terminal image protocols remain frontend-specific.

A terminal composition layer owns its cells and hit targets together. Now Playing
starts on a cleared workspace with its own interaction regions; only the queue
regions outside that workspace are retained. Its presented rectangles are clipped
and translated with the visible layer. Panel movement owns composition while
active: structural column/popup transitions discard their cached frame instead
of replaying it over a moving panel. Terminal image protocols are suspended in
favor of cell artwork while layers move. Reversals continue from the presented
position. Key-repeat layout toggles and key releases do not activate commands.
Rendering checks cover transition reversal, hidden targets, narrow layouts and
resize boundaries; new layout defects should be investigated against these
ownership rules before adding a local geometry workaround.

Named themes resolve through `app::theme_palette`, which has no Ratatui or GPUI
dependency. Both adapters consume the same semantic foreground, background,
selection, border, notification and transport colors. The track-card surface is
derived once. System appearance uses native semantic colors in both windowed presentations.
The external TUI uses host terminal defaults; system-owned terminal colors need
not match a Mac window.

## Live presentations

`Session` owns AppState, Config, AudioPlayer, PlaybackClock, completion channels
and the cancelling TaskSession. Startup is explicitly Pending or Ready and runs
once. TaskContext is a non-owning execution scope; dropping a frontend context
does not cancel the session. Terminal and desktop adapters dispatch through the
same source-aware reducers and preserve generation and operation identity.

Windowed presentation is Graphical or Text, with Idle, Preparing and Deferred
transitions. The old adapter keeps input ownership until preparation commits.
Activation commits frontend feedback first and saves the preference only after
success. A failed or superseded preparation cannot alter that preference.
External terminals cannot request windowed switching.
Full-screen Help, Search and discovery surfaces remain active across presentations.
Restored workflow and Settings windows take foreground priority over restored
Library, Now Playing and Queue windows.

The desktop actor retains native window geometry and auxiliary-window identity.
AppState retains navigation, filters, selection, queue, discovery, playback and
staged Settings forms. Captured workflow contexts retain logical popup content
and request identity without window geometry. Source-scoped item anchors bridge
terminal-cell and native-pixel scroll positions, including filtered row order.
Presentation identity rejects stale input/feedback; viewport revision rejects
mouse events from an obsolete cell layout. Switching never reconnects a source,
recreates an audio backend or serializes the session.

## Responsiveness and projection ownership

The desktop actor distinguishes content, selection and playback projection work.
Playback-only updates retain catalog, Settings and workflow models. Immutable
artwork bytes are shared by snapshots. Idle workspaces publish no redundant
frames; playback, visible progress and panel animation continue updating.
Native windows, Settings and workflow adapters subscribe to snapshot changes.
Settings draft replies have an owned asynchronous receiver rather than a polling
loop, and retain the editor's existing request-ownership checks.

Inline filtering checks cancellation between batches of 64 records. Filter
versions remain monotonic across saved navigation views; restoring a view cannot
accept a completion belonging to an earlier query. Cancellation returns no result,
rather than publishing a partially filtered list.

Navidrome startup restores the compact browsing snapshot before loading or
migrating the complete track catalog. A sole folder can reuse an account-wide
compact snapshot only when the stored folder directory identifies that exact
folder. Fresh catalog preparation publishes its compact projection before the
large persistent write. Both deliveries retain library and operation identity.

## Measuring responsiveness

Run the opt-in measurements with the normal development profile and target:

```sh
cargo test --features desktop-tests --lib responsiveness_tests:: -- --ignored --nocapture --test-threads=1
```

The generated-data measurement reports median and p95 for a 25,000-artist column,
250,000-record filtering, shared snapshot cloning, cached metadata reads and a
2 MiB artwork read. The live measurement starts the dedicated loopback server with
5,000 artists, 10,000 albums and 50,000 tracks, using silent playback and Cargo's
isolated XDG profile. It reports cold/cached browsing readiness, artist-column
requests and GUI/Text projection transitions, and stops its server afterward.

Projection timings exclude the native window compositor and display latency.
Disk reads use a warm operating-system cache after their first sample. Results
are diagnostic measurements, not fixed timing assertions for CI or promises for
remote servers. Compare runs on the same machine under similar load and retain
request-identity and interaction checks alongside timing measurements.

Queue shuffle pins the playing occurrence, or the first pending entry when idle, and randomizes the remaining entries. Reordering retains
the playing occurrence by index identity, and insertion before it advances its
queue index without restarting audio. Replacing or deleting the playing entry
uses the corresponding explicit playback policy. Saved and undo queues contain
the complete queue. Changing queue contents or order cancels an in-flight edit
immediately; playback position and pause changes do not. A new edit replaces the
previous request. Request identities reject late completions, and edits have a
90-second overall deadline. Distance-ranking workers retain one of two CPU permits
until their blocking computation finishes, including after observer cancellation.

Radio and related-track continuation share the station buffer, refill state, task
lease and library/radio request generations. Manual queues are finite. Selecting a
station commits a replacement only after its first batch succeeds; selecting
Continue with Related Tracks preserves the entire buffer and the audible occurrence.
Refills append when the second-to-last track starts. Related continuation stays close to the
last track of the existing queue. Queue edits retain the selected generation policy;
switching to Manual Queue cancels discovery without restarting playback.

Opposite Day and Enemies depend on Distant Rank v1, a custom plugin installed
separately on the configured AudioMuse server. The server ranks its index by
exact cosine distance. Opposite Day uses the mean of the original queue and
supports up to 1,000 occurrences.
Enemies ranks each occurrence independently in batches of up to 256 and inserts
one result after each matched track. Both use Textamp's active Navidrome catalog
to enforce library membership, paginate under a single index generation, and
exclude original and already chosen tracks. A generation change restarts ranking
once. The queue edit is published only while its library and queue snapshot
still match, and it can be undone. Opposite Day reports missing seed analysis as
an error. Enemies can keep matches for seed positions that have usable analysis.
The first request can take longer while the server loads its index.

Twofer selects candidates through the catalog's artist index while retaining ID,
legacy-name and exclusion checks.
