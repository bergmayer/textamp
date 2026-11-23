//! Single source of truth for the help / keyboard-shortcuts screen.
//! The Help screen renders this with unavailable sonic commands omitted.

pub const HELP_TEXT: &str = r#"
ESSENTIAL
  :              Command palette
  F7             Full-screen visualizer (global); 1 artwork, 2–7 visualizers; Esc returns
  :q Enter       Quit (q or Q selects Quit first)
  /              Filter the focused column; title click or Up to header, Enter also opens it
  Ctrl+F         Search (current-folder files for Local/WebDAV)
  Audio Player: Review Sync shows pending changes. On this player searches synced libraries. / filters either list.
  WebDAV Audio Player: / filters descendants, Enter or row click opens, Space or checkbox click selects, Left goes up.
  Search boxes: AND, OR, NOT; parentheses; quotes for exact phrases. Adjacent terms imply AND.
  F1   Ctrl+H    Toggle this help
  Tab / Shift+Tab Next / previous panel; arrows navigate inside
  Click           Focus the clicked pane; wheel scrolls under the pointer
  Alt+A          Select visible artist names; arrows choose, Enter opens Artist Details
  Dialogs         Tab/Shift+Tab traverses fields and buttons; Enter activates
  Esc            Back / close

VIEW MODES
  ⌘⌥T           Cycle GUI Multiple Windows → GUI Single Window → Text
  Now Playing names open Track, Artist or Album Details in the library.
  ⌘⇧T / Ctrl+Shift+T Also cycle application modes
  [ / ]          Show / hide sidebar / queue
  ^              Library ↔ Now Playing
  Ctrl+2 / Ctrl+U / |  Show / hide the queue
  Alt+T          Focus transport; arrows choose controls, Enter activates
  Sliders        Left/Right adjusts track position or volume
  ,    F2        Toggle Settings

NAVIGATE
  ↑↓ ←→          Move / drill in / drill out
  Enter          Open selection in the library; never starts playback
  Double-click   Play a track, album or playlist
  Artist Details Biography, photos, related artists and Artist Mix
  Artist View Options: Show compilation-only artists; hidden name jumps appear dimmed.
  Backspace / Alt+Left   Back through the last six library locations
  Ctrl+Z         Undo the latest library jump or queue edit
  Ctrl+Shift+A   Search Apple Music in the app (⌘⇧A on Apple keyboards)
  R              Open Album Details for a random album
  Ctrl+Shift+R   Also Go to Random Album (Alt+Shift+R works)
  Left           Drill out
  PgUp/Dn  Home/End   Big jumps
  A–Z            Go to first item starting with that letter (shift+letter to first item with that as second letter)

COLUMN HEADERS
  Up at top      Focus the title and controls; Left/Right moves between them
  Enter / click title   Open the column filter; Esc or × closes it
  ≡ / ×          View Options / close column
  Album Details  Row above an album track list; Enter opens its details
  Detail panes   ▷ Play · ▹ Play Next · + Add to Queue
  Shift/Alt+click Sidebar heading opens subitems in a column
  :              Track-and-following operations are contextual commands
  Shift+wheel    Scroll columns horizontally (scrollbar also supports click/drag)


The lower-right shortcut labels are clickable; Tab/arrows never focus them.

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
  Drag a row     Reorder tracks; dragging a selected row moves the selected group
  Ctrl+S         Save active queue / station as playlist (Subsonic / Navidrome)
  : Create New Playlist                              Create an empty server playlist
  : Add Track / Add Selected Tracks / Add Album to Playlist   Choose or create a server playlist
  ✎ Playlist header, then Space                    Check tracks or whole album groups
  : Move Up / Move Down / Remove from Playlist      Edit checked entries in playlist order
  : Export       Export playlist / queue as M3U8 or CSV
  Ctrl+X         Clear queue
  Ctrl+Z         Undo last remix / edit

DOWNLOADS
  : Download Selection / Download Play Queue saves audio for offline use.
  Library → Downloads supports track details, filters, playback and queue actions.
  : Manage Downloads opens Downloads; pending rows show progress and × to cancel.
  x / Delete cancels the highlighted pending download.
  Context menu / palette: Remove Download for saved items, Cancel Download for pending.
  Toggle Offline Mode keeps local playback; network-only discovery is unavailable.
  Removing a download never removes source music.

MULTI-SELECT  (track lists)
  Ctrl+A         Select all tracks
  v              Enter select mode (clears prior selection)
  V              Enter select mode, keeping prior selection
  ↑↓             Extend selection while in select mode
  Esc            Clear

TOOLS
  ≡ in a column header opens its View Options (sorting, covers, grouping when available).
  Recently Added: Date added restores newest first; reverse shows oldest first.
  F3  Switch library    F4  Artist Details    F5  Refresh    F6  View options
  Biography: ↓ past text or Tab focuses Search Google; Enter selects. G searches, B opens source.

  Startup opens your last library. F3: Enter switches, Esc cancels, F2 manages.
  F2 Settings → Libraries: A adds, Enter manages, F5 re-scans the selected library.
  Settings: ↑↓ chooses a category; Enter / → opens it; Tab / Shift+Tab switches panes.
  Settings → Sidebar: Enter / Space hides or shows sidebar sections for this library type.
  Playlist names appear in the leftmost sidebar.
  Queue header: Queue Type (list) / Queue Tools (sliders); Enter opens a menu.
  Queue tools: Settings → Audio toggles Allow same artist; Twofer always permits it.
  Added tracks exclude queued albums.
  Streaming quality sets the next server stream’s bitrate limit; Original requests raw audio.
  Navidrome system lists and connected AudioMuse views are individually configurable.
  Hidden collections can be restored in Settings → Sidebar.
  Singles: search commands for Singles; Settings → Sidebar changes its maximum track count (default 2).
  Settings: F10 categories; 1–7 jumps in the category list; Enter/Space selects.
  Commands: selection actions first, then menu groups. Enter/Right opens; Left goes back.
  Typing searches all commands, including commands inside other submenus.
  : / Ctrl+P opens commands. Left / Esc returns to categories; Esc there closes settings.
  Click changes controls. Click a theme to preview; click it again or press Enter to apply.
  Narrow terminals show one settings pane at a time. Changes save automatically.
  Libraries are grouped by type/account. [Active] marks the current library.
  Up from the first saved library returns to categories. Enter opens its options.
  Left/Right selects General, Analysis or Cache.
  General: Make active, rename, connection, re-scan and removal. Saving does not switch libraries.
  Analysis: Sonic checkbox and AudioMuse connection, where supported.
  Cache: measured metadata and artwork size; refresh metadata or clear either cache
  for the selected library. Other libraries' artwork stays in place.
  F5 again cancels a running scan. Status appears below the library list. Clear asks first.
  Add library / A: choose Local, WebDAV, or Subsonic / Navidrome.
  Server setup includes optional AudioMuse fields; blank AudioMuse URL skips it.
  Saving a new library starts its metadata scan without switching playback.
  Enter checks/saves; Esc cancels. Blank password keeps the existing sign-in.
  Removing a server account removes its libraries/sign-ins from Textamp, not music.
  Settings categories: Libraries, Appearance, Audio, Sidebar, Search, About.
  Library Cache clears that library's metadata or downloaded artwork separately.
  Folder libraries: filename order, / filter, F5 reload, F4 artist biography.
  Saved catalogs/folder trees open first; weekly refresh plus F5. Album listings load on demand.
  Re-scan refreshes enabled AudioMuse too; it never scans audio on the server.
  Navidrome Folders is an artist/album tree, not disk directories.
  System lists query the server on opening; F5 refreshes the open system list.
  Sidebar: ↑↓ selects; Enter toggles groups; Right opens; Left collapses.
  Navidrome Ctrl+G: all tagged album genres → albums → tracks; not AI moods/styles.
  Queue Type: manual, related tracks, or radio; Auto-Mix overlaps songs.
  Lyrics: S toggles playback following, L changes language, R reloads, G searches the web; arrows scroll.
  Navidrome loads synchronized lyrics; local files display embedded text.
  Navidrome palette (:): favorite/unfavorite selection, ratings, lyrics,
  random mix, saved playback queues and playlist rename/replace/delete.
  Search… sits above browse: highlight for the logo, Enter for the Ctrl+F search popup.
  Settings → Appearance: highlight previews a music list; Enter applies.
  Settings → Appearance → Artwork: choose rendering for this presentation; column artwork visibility is remembered by type.
  Local/WebDAV search: current-folder audio filenames; / also filters directory names.
  browse includes Navidrome system lists; audiomuse and playlists have separate headings.
  AudioMuse: AI labels, tempo, energy and key browse analyzed tracks, not file tags.
  AudioMuse: analysis cache updates weekly with library metadata; F5 updates it now.
  AudioMuse: Describe music… needs CLAP; Search lyrics… needs lyrics analysis.
  Describe music: Enter opens a new prompt; Right browses saved description history.
  Similar tracks opens over the current view; Esc returns to that view.
  Similar request failures stay in results; F5 retries (also clickable).
  Description history: Delete removes the selected saved search.
  Description results: : → Save as playlist, or click the results header button.
  History keeps 30 searches per library across restarts. Arrows browse; Enter opens track details.
  AudioMuse background failures keep cached results; F5 in an analysis category retries.
  Results show title, then artist and album, like playlists.
  Describe Music: Tab to its search field; Enter searches, arrows browse history and results.
  Search and Similar Tracks: Enter/Right opens the actual details in the library; Alt+Left returns.
  Details: ▷ Play · ▹ Play Next · + Add to Queue. Shift+Enter includes following tracks. These actions are also in the command palette.
  Page Up/Down or the mouse wheel scrolls details. Enter on a queue track opens its details.
  AudioMuse: use the command palette's AudioMuse analysis for the selected track's details.
  Bios: Navidrome first, Wikipedia fallback. ↑↓ scroll, ←→ photos, B source.
  Ctrl+J  Open in library                  Ctrl+W  Close column
  Alt+F   Activate filter (same as `/`)
  Alt+R   Random album radio (leaf-folder albums for local / WebDAV)
  Palette: Settings stays first; available music searches and selected-item actions follow.
  External search: palette → Search Apple Music / Spotify / YouTube.
  Searches the selection or focused details, then artist/album context, then playback.
    Apple Music search opens the Music app through the operating system.
  Journey: choose endpoints, preview the route, then Enter to play; Esc edits.
  Sonic Mix: continues the current song, then plays sonic matches.
    Track menu (: / right-click): Start Sonic Mix based on track.
    Sonic Mix needs enabled analysis; a different seed plays first.
  Ctrl+Alt+R opens Radio; Ctrl+Alt+L starts Library Mix.
  Ctrl+Alt+A plays random albums; Ctrl+Alt+J opens Journey.
  Random Artist Mix: artists with 25+ tracks in the active library.

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

RADIO & QUEUE EDITS  (Queue / Now Playing → Radio / Queue Edits)
  Every library shares Radio controls. Local/WebDAV offer Library Mix and
  Play Random Albums: bottommost music folders are albums, in filename order.
  Navidrome metadata stations and the Twofer queue edit do not need analysis.
  Sonic modes use Navidrome's plugin or a direct AudioMuse connection.
  AudioMuse: Mood Mix offers the server's available moods when connected and enabled.
  On This Day needs release dates; Navidrome Style Mix uses genre tags.
  Stations fetch more tracks as they play, without skipping the current track.
  Manual Queue stops adding tracks and keeps the existing queue playing.
  Starting a station keeps the current queue until replacement tracks arrive.
  Failed or empty requests preserve that queue.
  Palette: Stop Playback cancels loading; Play/Pause also cancels a pending start.
  If a refill returns no new tracks, use Next to retry or select another station.
  Mood, Style, and Decade open choices in the palette. Select
  "Radio: Back" to return to the parent list; Esc closes the palette.
  Sonic Mix follows each new song outward; Stay Close Mix retains its seed.
  Both seed-based stations use one track chooser, then explicit Start Radio.
  Artist Mix uses one dialog: choose up to 12 artists, then start with Tab.
  Queue Edits rewrite the queue once; repeat an edit to apply it again.
    Twins, Twofer, Stretch, Doppelganger, Opposite Day, Enemies, Shuffle; Ctrl+Z undoes the last edit.
    Opposite Day and Enemies require the AudioMuse Distant Rank plugin.
  Request failures are reported separately from valid empty results.
JOURNEY
  Start / End    Search either endpoint directly; Down enters results, Enter selects
  Tab            Traverse endpoints, maximum tracks, result types, results and buttons
  Ctrl+Enter     Preview Journey; preview never replaces the queue
  Alt+S / Alt+E  Edit Start / End; Alt+M edits maximum tracks
  Alt+W          Swap Tracks; Alt+P previews or plays the ready Journey
  Esc            Return from preview to the form, or close the form

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
                || !(line.contains("Journey")
                    || line.contains("Sonic Mix")
                    || line.contains("AudioMuse:")
                    || line.contains("Sonic paths")
                    || line.contains("Sonic modes")
                    || line.contains("Ctrl+M")
                    || line.contains("Twins")
                    || line.contains("Freeze"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
