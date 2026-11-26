# MP3 players on macOS

Open **Audio Player** in the main sidebar of the Mac GUI or TUI. In the GUI,
**Window → Audio Player…** opens the same workspace column. Connect the player in MTP (file transfer) mode. Standalone card readers and mounted disks are ignored.

Y2Linux exposes Internal Storage and SD Card separately over MTP. Select **SD Card** to reuse libraries previously synced to the card. The Mac uses ImageCaptureCore to read and write objects over USB. Textamp uses the profile and library manifests on the player. After an MTP sync finishes, the player can be disconnected.

Enable playback logging on supported firmware. During sync, Textamp reads completed listens from the AudioScrobbler 1.1 log and matches them to managed tracks. Navidrome receives the original playback time. WebDAV listens are recorded locally. The on-player receipt prevents resubmission. If a connection drops during a scrobble request, an unconfirmed receipt is retained because the server may already have counted it. Older indexes without track tags may require a recognizable filename to identify a listen.

Choose the storage volume when multiple MTP storage areas are available, then check or
uncheck entries in **Playlists**. The Mac checklist includes a filter for long
lists. In the TUI, use Up and Down to move between controls and Enter to toggle
a selection. Selection saves immediately on the selected player storage. Its hidden `.textamp-player.json`
stores the player name, library associations, playlists, WebDAV folders, media-scanner
preference and reconnect setting. On another Mac, configure the same server URL and
username (including the share path for WebDAV). Textamp uses that Mac’s sign-in
and the existing library folder on the player. Each library’s `.textamp-sync.json`
stores its file list and fingerprints.
**Review Sync…** shows the audio files grouped by Add, Update, Remove and Already on player. Expand a group or filter by filename. Updates show their reason. **Apply Sync** uses that plan. Cancel dismisses it. Automatic reconnect sync proceeds directly. **On this player…** searches indexed files across libraries and includes Navidrome playlist names. The column shows the total source size of all
selected playlists, with shared tracks counted once. The size is unavailable
if the server omits any file sizes.

To create a playlist during GUI setup, enter a player name such as `Y2` and a **Server playlist**
name, such as `NeoPod Synchronizer`. A blank playlist field defaults to `Sync: Y2`.
Choose **Set Up Player**. Textamp links the volume's UUID to the current Navidrome
account and creates the named playlist if it does not already exist. Add music to this playlist
using the normal playlist controls. Use exactly one playlist with this name.

**Apply Sync** copies original audio onto the player. For both Navidrome and
WebDAV, Textamp uses a complete matching copy from Downloads when available.
Other tracks transfer directly from the source. Player sync leaves saved
downloads in place.
**Sync when connected** starts a sync when an enrolled volume reconnects as the only available player storage. With multiple storage areas, choose the storage volume and select **Review Sync…**, then **Apply Sync**. Editing the server playlist while the player remains attached
requires a manual sync. The connection remains bound to the account used at setup,
even when another library is active.

The progress display shows the current track, transferred bytes, completed files
and reused files. **Cancel Sync** discards an incomplete file and retains
completed copies for the next attempt. Unchanged audio is reused using its
recorded metadata and local file size. Changes reported in Navidrome’s catalog and changes to a
WebDAV file’s server version refresh the synced copy. A change in play count
does not cause a new transfer.

## Files and firmware

Each source has a `Textamp <library ID>` folder on the player. Audio goes under
`Artist/Album` with filenames such as `01 Title.flac`. Textamp uses the album
artist when available, orders multi-disc albums by disc and track, and pads
all numbers consistently for albums with 100 or more tracks. Tracks without
a number follow the numbered tracks on their disc. A short suffix distinguishes two tracks that would otherwise
have the same filename.

On Y2 Linux, open **Settings → Customize → Browsing**. Set **Artists List**
to **Album Artists Only** to hide performers credited only on compilation tracks.
Set **Sort Albums** to **By Artist** to browse compilations together under their
shared album artist. Reborn groups album entries by album title and album artist
from the audio tags. Compilations with inconsistent album-artist tags can appear
as separate releases even when their files share a folder.

Textamp keeps its file index on the player and writes a separate UTF-8 M3U for
each selected playlist, preserving its track order and repeats. Playlists
share audio files. After it writes the new playlists, sync removes the M3Us
that were deselected and the managed tracks that no selected playlist still
references. Files outside the index are preserved. An existing source folder
without a recognized index is reported as a conflict.

Navidrome sync compares the song fields returned by the catalog. When those
fields change, Textamp compares the audio and replaces the player copy if the
bytes differ. A same-size edit that leaves those catalog fields unchanged can
go undetected. After an interruption between publishing a track and updating
its index, the next sync checks the pending track against the digest on the
player before deciding whether it needs another transfer.

## WebDAV folders

Switch to the WebDAV library, then open **Audio Player** and select the storage
volume. Click a folder name or press Enter to open it. Use Up and Down to move
between folders, Left or Back to go up, and Space or a checkbox click to
toggle sync selection. The filter searches all subfolders of
the current folder. In the TUI, press `/` to focus the filter and Left to go up.
Textamp caches three WebDAV folder levels and caches deeper folders as you visit them. A dash marks a folder with selected descendants. **Synced folders** lists the roots copied to this player. Uncheck one there, review the changes, and apply the sync to remove it. **Entire library** includes all folders. Each sync preserves the remote folder hierarchy and filenames.
A matching WebDAV file version lets Textamp reuse the player copy even after
the computer’s cached download is removed. If the server supplies no usable
version for an existing file, Review Sync marks it for verification. Textamp
compares its bytes using a temporary file on the computer and leaves matching
player audio in place.
Deselecting a folder removes its managed tracks unless another selection still
includes them.

**Hide from media scanner (.nomedia)** defaults to on for each WebDAV
library and player volume. The next sync adds a marker to each synced directory
and its ancestors inside the library, or removes Textamp’s markers when disabled. Textamp records the markers it creates and leaves pre-existing markers in place. The Android media scanner honors the marker.
The stock Y2 Music app's behavior with this marker still needs device
verification. [Android MediaStore](https://developer.android.com/reference/android/provider/MediaStore#MEDIA_IGNORE_FILENAME)

The same volume can hold independent Navidrome and WebDAV libraries.
WebDAV sync runs manually from the active library. Existing Navidrome
playlist and reconnect settings remain separate.

## On the device

Disconnect the MTP player once the transfer finishes. The player reads the M3U files from each Textamp library folder.

Textamp copies originals; it does not transcode. Formats are checked against the
Y2's documented supported extensions before audio transfer. Codec details within
a container may still exceed a particular player's decoder support. Leave room
for new files alongside the old playlist: older managed copies are retained until
the new playlist is ready. An unknown file size or a device's filesystem limits
can still cause an individual copy to fail.

## Implementation and validation

`src/app/player_sync.rs` owns configuration, operation identity, cancellation and
the single active transfer. `src/services/player_sync` owns volume discovery and
file reconciliation. The desktop window projects that state through Kit controls;
it does not implement a second transfer policy. File → Devices commands share the
normal menu and palette catalog. The Mac GUI and TUI use the same sync service.

MTP object transfers are
sequential to limit device pressure and preserve simple cancellation and progress.
The on-device index records playlist ownership before publication so an
interrupted sync can resume. Session identity checks reject
a disconnected or replaced target. HTTP transfers have bounded header and idle
timeouts and reject empty audio and document responses.

Focused service tests cover ordering, duplicate tracks, reuse, owned-file cleanup,
partial failure, cancellation, path safety, replacement volumes and interrupted
playlist publication. `scripts/test-player-sync.sh` exercises setup and repeat
sync through the shared reducer against a loopback server and a disposable FAT32
disk image. It checks decoded audio, playlist paths and the absence of Downloads
entries, then detaches its image and removes its temporary profile. It does not
access a personal server or physical player.
