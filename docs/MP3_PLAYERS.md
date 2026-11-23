# MP3 players on macOS

Open **Audio Player** in the main sidebar of the Mac GUI or TUI. In the GUI,
**Window → Audio Player…** opens the same workspace column. Connect a player as writable USB storage. For an
innioasis Y2 running stock firmware, press its center OK button to turn on USB
storage when the USB connection screen appears.

Choose the storage volume when multiple drives are mounted, then check or
uncheck entries in **Playlists**. The Mac checklist includes a filter for long
lists. In the TUI, use Up and Down to move between controls and Enter to toggle
a selection. Selection saves immediately on the mounted volume. Its hidden `.textamp-player.json`
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
**Sync when connected** starts a sync when an enrolled volume reconnects as the only mounted player volume. With multiple drives, choose the storage volume and select **Review Sync…**, then **Apply Sync**. Editing the server playlist while the player remains attached
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

Use **Eject** before disconnecting. On the stock Y2, turn off USB storage and use
**Music settings → Import M3U** to import the M3U files from the library’s Textamp folder. Copying an
M3U does not directly modify the firmware's private music database. Playback and
playlist import on the hardware require a separate check after ejecting.

For folder playback on the stock Y2, use **Display Song By → File name** and
**List Sort → A–Z** so the numeric filename prefixes determine playback order.
The stock firmware recognizes audiobooks under `Audiobooks`. Innioasis's
published Music settings do not describe a folder exclusion for the music
index. [Y2 Music settings](https://www.innioasis.com/a/docs/q3e/music-app)
and [audiobook folder behavior](https://www.innioasis.com/a/docs/y2/now-playing).

Textamp copies originals; it does not transcode. Formats are checked against the
Y2's documented supported extensions before audio transfer. Codec details within
a container may still exceed a particular player's decoder support. Leave room
for new files alongside the old playlist: older managed copies are retained until
the new playlist is ready. An unknown file size or a device's filesystem limits
can still cause an individual copy to fail.

Playback counts and last-played dates are **not synced**. The stock Y2's exposed
USB storage has not provided a usable playback-history export. Textamp does not
guess play counts from file timestamps or overwrite server history.

The manufacturer documents USB storage, supported formats and M3U import/export
in the [Y2 manual](https://www.innioasis.com/cdn/shop/files/Y2_EN_V1.21.pdf?v=6393700319299377419).

## Implementation and validation

`src/app/player_sync.rs` owns configuration, operation identity, cancellation and
the single active transfer. `src/services/player_sync` owns volume discovery and
file reconciliation. The desktop window projects that state through Kit controls;
it does not implement a second transfer policy. File → Devices commands share the
normal menu and palette catalog. The Mac GUI and TUI use the same sync service.

Volume information reads have concurrency limited to four. USB copies remain
sequential to limit device pressure and preserve simple cancellation and progress.
The on-device index records playlist ownership before publication so an
interrupted sync can resume. Mount and UUID checks reject
a disconnected or replaced target. HTTP transfers have bounded header and idle
timeouts and reject empty audio and document responses.

Focused service tests cover ordering, duplicate tracks, reuse, owned-file cleanup,
partial failure, cancellation, path safety, replacement volumes and interrupted
playlist publication. `scripts/test-player-sync.sh` exercises setup and repeat
sync through the shared reducer against a loopback server and a disposable FAT32
disk image. It checks decoded audio, playlist paths and the absence of Downloads
entries, then detaches its image and removes its temporary profile. It does not
access a personal server or physical player.
