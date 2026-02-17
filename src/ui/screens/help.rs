//! Help screen with keybindings (CUA-style).

use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// Return the total number of lines in the help text (for scrollbar hit-testing).
pub fn help_total_lines() -> usize {
    HELP_TEXT.lines().count()
}

const HELP_TEXT: &str = r#"
NAVIGATION
  Arrow keys      Navigate lists
  Tab             Next view (Library→Playlists→Queue→Now Playing)
  Shift+Tab       Previous view
  Enter           Select / Drill down; on tracks: add to queue + play
  Shift+Enter     On tracks: add track + following to queue + play
                  On album/artist: add to queue + play (skip drill)
  Right           Select / Drill down (no playback)
  Left / Bksp     Go back / Move focus left
  Esc             Cancel / Go back
  Page Up/Down    Scroll by page
  Home/End        Jump to top/bottom
  A-Z             Jump to first item starting with letter
  Shift+A-Z       Refine: same first letter, 2nd char matches
  /               Activate inline filter (type to filter list)

CATEGORIES (Ctrl+key - works from any view)
  Ctrl+L          Library (artists with alias-aware display)
  Ctrl+P          Playlists
  Ctrl+G          Genres (Left/Right or Ctrl+V to switch tabs)
  Ctrl+O          Folders
  Ctrl+U          Queue (track list with stations panel)
  Ctrl+N          Now Playing (visualizer: waveform/spectrum/spectrogram)

VIEWS
  F1 / ?          This help screen
  F2              Settings

COMMANDS (Alt+/ or Ctrl+/ to see available commands)
  Ctrl+F          Search popup (floating dialog)
  Ctrl+E          Add track + following to TOP of queue and play
  Ctrl+Shift+E    Add track + following to END of queue
  Click track name in transport bar → Now Playing
  Ctrl+S          Sort options popup (sort, direction, artwork, group-by-album)
  F3              Switch library
  F4              Artist bio popup
  Ctrl+V          Quick sort cycle (context-dependent)
                  Artists: default ↔ shuffled
                  Albums: default → by artist → shuffled → default
                  Album tracks: default → by title → by duration → shuffled
                  All tracks*: default → by artist → by album → by title
                    → by duration → shuffled → default
                  Genres: cycle through tabs
                  Now Playing: cycle view (waveform→spectrum→spectrogram)
                  * playlist, all tracks, compilation tracks columns
  Ctrl+M          Show similar albums/tracks (popup overlay)
  Ctrl+B          Go to Album (navigate to track's album in Library)
  Ctrl+W          Save queue/radio as playlist
  Ctrl+X          Clear queue/radio
  Alt+L           Library Radio (station based on your library)
  Alt+R           Random Album Radio (shuffled albums)

SEARCH POPUP (Ctrl+F)
  Type to search library (instant local search)
  Tab / Shift+Tab Switch result tabs (All/Artists/Albums/Tracks/Playlists/Genres)
  Left / Right    Switch result tabs
  Enter / Down    Move to results
  Up (at top)     Back to search input
  Enter           Navigate to selected item in library
  Shift+Enter     Play selected item (add to queue + play)
  Esc             Close search

INLINE FILTER (/ key)
  /               Activate filter in transport bar
  Filter stays on original column when drilling down

PLAYBACK
  Space           Play / Pause
  <               Previous track
  >               Next track
  Ctrl+Shift+Up/Dn Volume up/down
  Shift+Left      Seek backward 10 seconds
  Shift+Right     Seek forward 10 seconds
  Play history syncs to Plex server

QUEUE (Ctrl+U)
  Left panel: artwork + station browser
  Right panel: current queue or radio tracks
  Left/Right      Switch focus between panels
  Del             Remove track from queue (radio auto-converts to queue)
  Shift+Up/Down   Move selected track up/down (radio auto-converts to queue)
  Ctrl+Z          Undo last remix/edit (restores radio if applicable)
  Ctrl+W          Save queue/radio as playlist
  Enter on playing track → opens Now Playing view
  Stations panel  Browse Plex radio stations, DJ modes, and remix tools
                  Enter/Right     Drill into category or play station
                  Left/Backspace  Go back (at root: return to track list)

NOW PLAYING (Ctrl+N)
  Album art, track info, and visualizer panel
  Visualizer tabs waveform / spectrum / spectrogram
  Ctrl+V          Cycle view (visualizer tab)
  Up              Focus tab bar (then Left/Right to switch, Down to return)
  Left/Right      Seek ±1 second
  Esc             Return to Queue view

ARTIST RADIO
  In Library, drill into an artist to see "Artist Radio" above "All Tracks".
  Press Enter to start Plex radio seeded from that artist.
  Multi-artist radio: select "Artist Radio" in the stations panel to open
  the picker. Enter count (2-12), search and select artists with Enter.
  Auto-launches when all artists are selected.

COMPILATIONS
  Artists with tracks on multi-artist compilation albums show "Compilations"
  in their album list. Drill in to see compilation albums for that artist.
  "All Tracks" at top shows all tracks from those compilation albums.
  Compilation-only artists are hidden from the artist list; filter (/)
  redirects matches to the "Compilations" entry instead.

SONIC ADVENTURE (Alt+A or via stations panel)
  Creates a sonic bridge between two tracks using Plex sonic analysis.
  Alt+A method:
    1. Select start track, press Alt+A
    2. Navigate to end track, press Alt+A
    3. Enter length (5-100 tracks)
    4. Adventure replaces queue, starts playing
  Stations panel: "Sonic Adventure" item provides a self-contained UI
  with tabbed search (All/Artists/Albums/Tracks/Playlists/Genres).
  Drill into artists/albums to find tracks, or select directly from Tracks tab.

DJ MODES (via stations panel)
  Guest DJ modes that modify playback while active.
  Toggle on/off by pressing Enter on a DJ mode in the station panel.
  Only one DJ mode can be active at a time.
  Activating DJ while a station is playing converts it to a queue.
  Starting a station deactivates the active DJ mode.

  Interleaving modes (insert tracks between original queue tracks):
    DJ Gemini       Inserts a sonically similar track after each original track
    DJ Twofer       Inserts a same-artist track (skips if next is same artist)
    DJ Stretch      Inserts a sonic bridge between current and next track
    Pattern: original -> DJ pick -> original -> DJ pick -> ...

  Continuous modes (insert after every track, original queue keeps getting pushed down):
    DJ Freeze       Keeps the mood with sonically similar tracks
    DJ Contempo     Keeps the mood with tracks from the same era
    DJ Groupie      Keeps queueing from current and related artists
    Pattern: original -> DJ pick -> DJ pick -> DJ pick -> ...

  Active DJ mode shows a dot prefix in the station panel.

QUEUE REMIX (via stations panel)
  One-time operations that modify the entire queue at once.
  Available in the Remix section of the stations panel.
  If a radio station is playing, remix auto-converts to queue first.
  Ctrl+Z undoes the last remix (restores radio if it was auto-converted).

    Remix: Gemini       Insert similar tracks between each queue pair
    Remix: Twofer       Insert same-artist tracks between each queue pair
    Remix: Stretch      Insert sonic bridge tracks between each queue pair
    Remix: Doppelganger Replace each track with similar by different artist
    Remix: Shuffle      Shuffle the queue (press again to undo)

PLAYLISTS (Ctrl+P)
  Miller columns navigation (drill down into playlists)
  Ctrl+S → Group by album  Group playlist tracks by album

GENRES (Ctrl+G)
  Up (at top)     Focus tab bar (All / Library / Artist / Album / Mood / Style)
  Left/Right      Switch tabs (when tab bar focused)
  Down/Enter      Return to content from tab bar
  All tab         Merged list of all genre types with type suffix

FOLDERS (Ctrl+O)
  ♪ icon shows currently playing track

SETTINGS (F2)
  F2              account, textamp, about sections
  textamp section themes, artwork mode, playback output (local/remote)
                  remote players: Apple TV, Plexamp on phone, etc.
                  braille: 2x4 dot-art rendering for Apple Terminal
  about section   version, credits, graphics info

GENERAL
  F5              Refresh current view / album tracks (updates cache)
  Ctrl+Q          Quit
"#;

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area
    );

    let block = Block::default()
        .title(" help (↑↓ PgUp/PgDn to scroll, Esc to close) ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Count lines for scroll clamping
    let line_count = HELP_TEXT.lines().count() as u16;
    let visible_lines = inner.height;
    let max_scroll = line_count.saturating_sub(visible_lines);
    let scroll = state.help_scroll.min(max_scroll);

    let paragraph = Paragraph::new(HELP_TEXT.trim())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);

    // Scrollbar for long help text
    if line_count > visible_lines {
        crate::ui::widgets::render_scrollbar(
            frame, area,
            line_count as usize,
            visible_lines as usize,
            scroll as usize,
        );
    }
}
