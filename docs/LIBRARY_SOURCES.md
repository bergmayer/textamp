# Library sources

Textamp supports Subsonic/Navidrome, local folders and WebDAV. AudioMuse adds
optional analysis and discovery to server libraries. Plex is retired; the
pre-removal source and historical notes are preserved in `archive/`.

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
server and selected music folder. Local/WebDAV libraries persist visited folder
listings. They browse on demand, not through a recursive startup scan. Cached
metadata appears first; weekly refresh and manual F5 update it. Failed refreshes
retain usable prior data. These caches are not offline audio downloads.

AudioMuse results follow the same weekly/manual policy. Textamp consumes analysis;
it does not start or alter server scans. Standard file tags remain separate from
inferred labels. Missing analysis does not prevent ordinary browsing/playback or
metadata-only radio. Folder radio uses accessible leaf folders as albums.

Artist biographies use server metadata when available, then strict Wikipedia
matching on explicit lookup. No custom biography sidecar convention is used.

## Verification boundaries

Automated tests use loopback servers, temporary libraries and isolated XDG paths.
The Navidrome protocol adapter also serves compatible Subsonic servers, but a
passing Navidrome fixture is not proof of compatibility with every server/version.
Opt-in live tests and native audio require a suitable environment. Never test
playlist writes, favorites, ratings or scrobbling against a personal server.
