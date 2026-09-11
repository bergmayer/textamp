//! Single source of truth for the help / keyboard-shortcuts screen.
//! The Help screen renders this with unavailable sonic commands omitted.

pub const HELP_TEXT: &str = r#"
ESSENTIAL
  :              Command palette
  :q Enter       Quit (q or Q selects Quit first)
  /              Filter the focused column (type to narrow, ↑↓ to filtered results, Enter activates)
  Ctrl+F         Search (current-folder files for Local/WebDAV)
  F1   Ctrl+H    Toggle this help
  Tab / Shift+Tab Switch Library ↔ Now Playing (keyboard focus in split mode)
  Click           Focus the clicked pane; wheel scrolls under the pointer
  Dialogs         Capture input; close them before navigating the main panes
  Esc            Back / close

VIEW MODES
  \              Toggle scrolling Miller layout (Browse)
  |              Toggle tall-mode split (Library top / Now Playing bottom)
  ,    F2        Toggle Settings

NAVIGATE
  ↑↓ ←→          Move / drill in / drill out
  Enter          Drill into selection (or open track-details pane)
  Backspace      Drill out
  PgUp/Dn  Home/End   Big jumps
  A–Z            Jump to first item starting with that letter (shift+letter to first item with that as second letter)

PLAY-ALBUM ROW  (tracks columns)
  After drilling into an album or playlist the cursor parks on a
  pinned "▶ Play album / Play playlist" row at the top of the
  column. Enter plays the whole album / playlist; ↓ drops to the
  first track; ↑ from the first track jumps back to the play row.
  Click the row to play; click an item below to select it.

VIEWS  (Ctrl+key)
  L  Library     G  Genres        O  Folders
  U  Queue       N  Now Playing
  P              Command palette  (same as `:`)

COMPILATIONS (Subsonic / Navidrome)
  Library → Compilations groups tagged multi-artist releases.
  An artist's Compilations entry shows albums they appear on.
  Single-artist collections stay with the performer. No AI needed.

PLAYBACK
  Space          Play / Pause
  < >            Previous / next track
  Shift+←→       Seek ±10 s
  Ctrl+Shift+↑↓  Volume

VISUALIZERS (Queue / Now Playing)
  Up, then ←→    Focus tabs and switch modes; tabs are also clickable.
  Waveform / Spectrum / Spectrogram / Vectorscope / Landscape / Meters
  Landscape shows a 12-second spectral trail. Meters use local, pre-volume
  PCM: RMS, sample peaks, peak holds, correlation (not LUFS or true peak).
  Click / drag waveform or transport scrubber to seek; streams may rebuffer.
  Landscape / Meters canvases do not seek. Pause holds the meter display.

QUEUE
  Ctrl+E         Add selection to end of queue
  Ctrl+Shift+E   Insert next (after current track)
  Del            Remove from queue
  Shift+↑↓       Reorder selected
  Ctrl+S         Save active queue / station as playlist (Subsonic / Navidrome)
  Ctrl+X         Clear queue
  Ctrl+Z         Undo last remix / edit

MULTI-SELECT  (track lists)
  Ctrl+A         Select all tracks
  v              Enter select mode (clears prior selection)
  V              Enter select mode, keeping prior selection
  ↑↓             Extend selection while in select mode
  Esc            Clear

TOOLS
  F3  Switch library    F4  Artist bio    F5  Refresh    F6  Sort
  Biography: ↓ past text or Tab focuses Search Google; Enter selects. G searches, B opens source.

  Startup opens your last library. F3: Enter switches, Esc cancels, F2 manages.
  F2 Settings → Libraries: add libraries, manage accounts and credentials.
  Settings: Tab / Shift+Tab switches sidebar/content; ↑↓ selects items.
  Settings → Textamp: Enter / Space hides or shows sidebar sections for this library type.
  Playback is local; streaming quality is configurable for server libraries.
  Navidrome system lists and connected AudioMuse views are individually configurable.
  Hidden collections can be restored in Settings → Textamp.
  Libraries are grouped by type/account in one list. ← returns to the sidebar.
  [Active] marks the current library; Enter opens options, including Make active.
  Add library / A: choose Local, WebDAV, or Subsonic / Navidrome.
  R rename · C credentials · Del remove from Textamp (never server music)
  I toggles Sonic features for the selected server library; click also works.
  M on a Navidrome library opens its AudioMuse connection form (also clickable).
  Removing a server account removes its libraries/sign-ins from Textamp, not music.
  WebDAV / Navidrome: one connection form; Enter checks/saves, Esc cancels.
  C edits WebDAV; blank password keeps it. Use the server's exact http:// or https:// URL.
  Folder libraries: filename order, / filter, F5 reload, F4 artist biography.
  Library options: cache size, Clear and Re-scan (again cancels); no library switch.
  Shared artwork has a separately labelled clear control.
  Saved catalogs/folder trees open first; weekly refresh plus F5. Album listings load on demand.
  Re-scan refreshes enabled AudioMuse too; it never scans audio on the server.
  Navidrome Folders is an artist/album tree, not disk directories.
  System lists query the server on opening; F5 refreshes the open system list.
  Sidebar: arrows skip headings; wheel scrolls long lists without moving keyboard focus.
  Navidrome Ctrl+G: all tagged album genres → albums → tracks; not AI moods/styles.
  Navidrome palette (:): favorite/unfavorite selection, ratings, lyrics,
  random mix, saved playback queues and playlist rename/replace/delete.
  Search… sits above browse: highlight for the logo, Enter for the Ctrl+F search popup.
  Settings → Textamp: arrows preview theme colors on the right; Enter applies.
  Local/WebDAV search: current-folder audio filenames; / also filters directory names.
  browse includes Navidrome system lists; audiomuse and playlists have separate headings.
  AudioMuse: AI labels, tempo, energy and key browse analyzed tracks, not file tags.
  AudioMuse: analysis cache updates weekly with library metadata; F5 updates it now.
  AudioMuse: Describe music… needs CLAP; Search lyrics… needs lyrics analysis.
  AudioMuse: use the command palette's AudioMuse analysis for the selected track's details.
  Bios: provider first, Wikipedia fallback. ↑↓ scroll, ←→ photos, B source.
  Ctrl+R  Related artists
  Ctrl+M  Similar
  Ctrl+J  Open in library                  Ctrl+W  Close column
  Alt+F   Activate filter (same as `/`)
  Alt+R   Random album radio (leaf-folder albums for local / WebDAV)
  External search: palette → Search in Apple Music / Spotify / YouTube.
    Music app search needs terminal Automation and Accessibility permissions;
    web search is the fallback. Textamp does not change these permissions.
  Sonic Adventure: open the palette and pick "Sonic Adventure".
  Sonic Radio: continues the current song, then plays sonic matches.
    Track menu (: / right-click): Start Sonic Radio based on track.
    Sonic Radio needs enabled analysis; a different seed plays first.
  Random Artist Radio: artists with 25+ tracks in the active library.

QUIT
  Ctrl+Q / Ctrl+C       Quit

MOUSE
  Click          Select / drill (click an already-selected row to
                  drill, click a track row to open its details pane)
  Right-click    Track menu (play, queue, open in library, …)
  Double-click   Play immediately — album / playlist / folder
  Drag           Reorder rows in the queue

CONNECTIONS
  Background refresh failures keep cached data and use the status bar.
  Temporary failures retry after 30s, then 2 minutes. F5 retries the
  current section immediately, including after automatic retries stop.

STATIONS & DJ MODES  (Queue / Now Playing → Radio / DJ Modes)
  Every library shares Radio controls. Local/WebDAV offer Library Radio and
  Random Album Radio: bottommost music folders are albums, in filename order.
  Navidrome metadata stations and Twofer/Contempo do not need analysis.
  Sonic modes use Navidrome's plugin or a direct AudioMuse connection.
  AudioMuse: Mood Radio offers the server's available moods when connected and enabled.
  On This Day needs release dates; Navidrome Style Radio uses genre tags.
  Stations fetch more tracks as they play, without skipping the current track.
  Starting a station keeps the current queue until replacement tracks arrive.
  Failed or empty requests preserve that queue.
  Palette: Stop Playback cancels loading; Play/Pause also cancels a pending start.
  If a refill returns no new tracks, use Next to retry or select another station.
  Mood, Style, and Decade open choices in the palette. Select
  "Radio: Back" to return to the parent list; Esc closes the palette.
  DJ modes weave picks into the queue while you listen:
    Twofer                    — one same-artist pick between tracks
    Contempo, Groupie          — era / artist picks after each track
    Gemini, Stretch           — one sonic pick between each track
    Freeze                    — sonic picks after every track
  Remix tools rewrite the existing queue once
    Twofer, Shuffle
    Gemini, Stretch, Doppelganger
  Request failures are reported separately from valid empty results.
"#;

/// Total line count, used by the TUI scroll-clamp and the scrollbar
/// hit-test. Computed once per call (the string is short enough that
/// memoisation isn't worth the complexity).
pub fn total_lines() -> usize {
    HELP_TEXT.lines().count()
}

/// Hide unavailable command documentation along with the commands themselves.
pub fn for_library(sonic: bool) -> String {
    HELP_TEXT
        .lines()
        .filter(|line| {
            sonic
                || !(line.contains("Sonic Adventure")
                    || line.contains("Sonic Radio")
                    || line.contains("AudioMuse:")
                    || line.contains("Sonic paths")
                    || line.contains("Sonic modes")
                    || line.contains("Ctrl+M")
                    || line.contains("Gemini")
                    || line.contains("Freeze"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
