//! Single source of truth for the help / keyboard-shortcuts screen.
//! Both the TUI's full-screen Help view and the GUI's modal Keyboard
//! Shortcuts popup render this verbatim, so the two front-ends can't
//! drift apart.

pub const HELP_TEXT: &str = r#"
ESSENTIAL
  :              Command palette
  /              Filter the focused column (type to narrow, ↑↓ to filtered results, Enter activates)
  ?              Search popup (same as Ctrl+F)
  F1   Ctrl+H    Toggle this help
  Tab            Switch Library ↔ Now Playing
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

PLAYBACK
  Space          Play / Pause
  < >            Previous / next track
  Shift+←→       Seek ±10 s
  Ctrl+Shift+↑↓  Volume

QUEUE
  Ctrl+E         Add selection to end of queue
  Ctrl+Shift+E   Insert next (after current track)
  Del            Remove from queue
  Shift+↑↓       Reorder selected
  Ctrl+S         Save queue as playlist
  Ctrl+X         Clear queue
  Ctrl+Z         Undo last remix / edit

MULTI-SELECT  (track lists)
  v              Enter select mode (clears prior selection)
  V              Enter select mode, keeping prior selection
  ↑↓             Extend selection while in select mode
  Esc            Clear

TOOLS
  F3  Switch library    F4  Artist bio    F5  Refresh    F6  Sort
  Ctrl+F  Search (same as `?`)        Ctrl+M  Similar   Ctrl+R  Related
  Ctrl+J  Open in library                  Ctrl+W  Close column
  Alt+F   Activate filter (same as `/`)
  Alt+R   Random album radio
  Streaming search (Apple Music / Spotify / YouTube): open the
    palette and pick "Search …".
  Sonic Adventure: open the palette and pick "Sonic Adventure".

QUIT
  Ctrl+Q                Linux / Windows / macOS terminal (TUI)
  Cmd+Q                 GUI on macOS
  Alt+F4                GUI on Windows

MOUSE
  Click          Select / drill (click an already-selected row to
                  drill, click a track row to open its details pane)
  Right-click    Context menu (play, queue, open in library, …)
  Double-click   Play immediately — album / playlist / folder
  Drag           Reorder rows in the queue

GUI EXTRAS
  Ctrl++ / Ctrl+=    Zoom in
  Ctrl+-             Zoom out
  Ctrl+0             Reset zoom to 1.0×

STATIONS & DJ MODES  (Now Playing → Radio / DJ Modes)
  Stations play continuously, fetching more tracks as they go.
  DJ modes weave picks into the queue while you listen:
    Gemini, Twofer, Stretch    — one DJ pick between each track
    Freeze, Contempo, Groupie  — DJ pick after every track
  Remix tools rewrite the existing queue once
    (Gemini, Twofer, Stretch, Doppelganger, Shuffle).
"#;

/// Total line count, used by the TUI scroll-clamp and the scrollbar
/// hit-test. Computed once per call (the string is short enough that
/// memoisation isn't worth the complexity).
pub fn total_lines() -> usize {
    HELP_TEXT.lines().count()
}
