# Library sources

Textamp supports Subsonic/Navidrome, local folders and WebDAV. AudioMuse adds
optional analysis and discovery to server libraries.

## Ownership

- `app/sources`: selection, provider routing, operation identities and task leases.
- `navidrome`: OpenSubsonic requests, catalog normalization and account credentials.
- `audiomuse`: analysis, labels, similarity, paths and descriptive/lyrics discovery.
- `library`: shared catalog/track models, folder access and persistent caches.
- `media`: provider-independent artwork, waveform and spectrogram analysis/caches.
- `audio`: local decoding/output from URLs or owned files.

Shared reducers manage navigation, queue and playback state. Providers handle I/O;
unsupported requests produce an explicit error instead of reaching another
backend. Capabilities determine available UI actions. Sonic preferences are scoped
to each server library; availability is distinct from whether analysis has finished.

Track origin includes the source identity. Renaming a source does not change its
identity. Old unscoped tracks are unavailable, not implicitly playable through a
different server. Serialized shared metadata fields remain compatible with existing
Navidrome and folder caches.

Switching sources stops playback, clears the previous queue and invalidates
library/connection generations. Task leases cancel superseded work; completions
also check their operation, track or navigation identity. Remote file spools live
until their last decoder/analysis consumer releases them. An OS-level filesystem
operation already in progress cannot be forcibly cancelled.

The [server feature requirements](../README.md#features-that-need-more-than-stock-navidrome) list the Navidrome metadata agents, AudioMuse indexes and AudioMuse Distant Rank plugin used by individual features.

## Settings and startup

F3 opens the compact library switcher. F2 → Libraries manages Libraries, Accounts
and Add library. Keyboard and mouse use the same choices. Managing or cancelling
does not start playback. Startup opens the last selection, or another saved source
if that selection was removed. With no sources, normal Browse offers an add-library
prompt. There is no authentication screen for a retired backend.

Server accounts can contain multiple music folders. A single folder has one entry;
“All music” is useful only for multiple folders. Passwords are private stored secrets
or environment overrides, not URL components. Removing an account never deletes
music. Existing personal Plex credential files are left untouched but never read.

## Caches and discovery

Server libraries persist their catalog and supported extensions per account,
server and selected music folder. Local/WebDAV libraries persist a directory-only
index of the root and the next two directory levels. Background scans
never preload track listings or audio. Opened folder listings use a separate
512 MiB disk cache shared across libraries. After 15 minutes, a visited listing
opens from disk while its refresh runs. Catalog and other source snapshots share
a separate 512 MiB budget. Under pressure, item snapshots are evicted before
the indexes used to open libraries. Cached directory metadata
appears first; weekly refresh and manual F5 update it. Failed refreshes retain usable
prior data. Navidrome playlist contents preserve server order and duplicates in
separate snapshots.

Background artwork warming covers catalog albums and unique playlist albums,
including albums outside the selected music folder. Available playlist cover
requests take priority over the remaining catalog covers. All frontends use the same
library-scoped image cache. Album and cover identities reference one stored image.
Each library retains up to 2 GiB of artwork: background fills stop at capacity,
while requested artwork evicts least recently used entries. Successful complete
scans retire previously indexed images that are no longer
referenced by any playlist or the catalog. Failed or superseded refreshes cannot
perform this cleanup. Up to eight playlist requests and four artwork requests run concurrently;
browsing can read saved contents while refresh runs. These caches are not offline
audio downloads.

AudioMuse results follow the same weekly/manual policy. An interrupted sync page gets one retry. If the refresh still fails, Textamp keeps the available analysis and records the cause in the activity log. Opening an analysis category retries the failed refresh.
During a first import, Textamp saves groups of eight metadata pages. After a restart
it reads the first full page to establish the page size, then checks saved rows
against the server’s current IDs and fingerprints. Changed pages and the remaining
tail are fetched in full. These temporary files belong to the library and connection,
expire for reuse after a week, and are evicted first under cache pressure.
Saving the finished index removes them. Textamp consumes analysis;
it does not start or alter server scans. Standard file tags remain separate from
inferred labels. Missing analysis does not prevent ordinary browsing/playback or
metadata-only radio. Folder radio uses accessible leaf folders as albums.

Artist biographies prefer Navidrome metadata on explicit lookup, with an
eight-second artist-info request limit. Missing text, errors and timeouts fall
back to strict Wikipedia matching, constrained by MusicBrainz IDs where available.
Conflicting server identities, Wikipedia ambiguity and failures remain explicit.
No custom biography sidecar convention is used.

## Verification boundaries

Automated tests use loopback servers, temporary libraries and isolated XDG paths.
The Navidrome protocol adapter also serves compatible Subsonic servers, but a
passing Navidrome fixture is not proof of compatibility with every server/version.
Opt-in live tests and native audio require a suitable environment. Never test
playlist writes, favorites, ratings or scrobbling against a personal server.

Navidrome browsing indexes album and artist track positions when installing a
prepared catalog. These indexes contain positions into the current catalog, not
independent copies of tracks. Visited album listings have a private, source-scoped
persistent metadata cache with the weekly freshness policy and an in-memory cache
for repeated browsing. Manual album refresh bypasses the saved listing and replaces
both caches. Failed requests preserve the prior usable column.

Navidrome restores a compact browsing snapshot before decoding its complete track
catalog. The snapshot includes artist classification, albums and playlists, so
artist visibility rules apply immediately. Full track metadata loads on a blocking
worker; normal weekly refresh and failure preservation still apply. Large catalog searches share immutable artist, album, playlist, genre, alias and
track snapshots, debounce input and scan on a worker.
Replacing or clearing the query cancels the previous request and rejects stale
completions. Compilation excerpts open the complete release only when their songs
match a larger compilation with the same title.

Track-dependent aggregate navigation waits for full metadata when only the
browsing snapshot is available. It resumes only while the same selection and
request still own that navigation. Failed catalog loads retain browsing metadata
and expose an explicit retry instead of reporting an empty track collection.
