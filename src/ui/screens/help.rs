//! Help screen with keybindings (CUA-style).

use crate::app::AppState;
use crate::ui::theme::theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

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

    let help_text = r#"
NAVIGATION
  Arrow keys      Navigate lists
  Tab             Next view (Library→Playlists→Genres→Folders→Queue→Now Playing)
  Shift+Tab       Previous view
  Shift+Down      Cycle modes within category (e.g., Library→Album Artists)
  Shift+Up        Cycle modes backwards
  Enter / Right   Select / Drill down / Play
  Left / Bksp     Go back / Move focus left
  Esc             Cancel / Go back
  Page Up/Down    Scroll by page
  Home/End        Jump to top/bottom
  A-Z             Jump to first item starting with letter
  Shift+A-Z       Refine: same first letter, 2nd char matches
  /               Activate inline filter (type to filter list)

CATEGORIES (Ctrl+key - works from any view)
  Ctrl+L          Library (cycles: artists/album artists)
  Ctrl+P          Playlists
  Ctrl+G          Genres (Tab to switch: All/Library/Artist/Album/Mood/Style)
  Ctrl+O          Folders
  Ctrl+U          Queue (track list with stations panel)
  Ctrl+N          Now Playing (visualizer: waveform/spectrum/spectrogram)

VIEWS
  F1 / ?          This help screen
  F2              Settings

COMMANDS (Alt+/ to see available commands)
  Ctrl+F          Search popup (floating dialog)
  Alt+E           Add selection to queue (enqueue)
  Alt+V           Cycle view mode (context-dependent)
                  Albums: list → shuffled → covers → covers shuffled
                  Playlist tracks: tracks → shuffled → by album → shuffled → covers → covers shuffled
                  Genres: cycle through tabs (All/Library/Artist/Album/Mood/Style)
                  Now Playing: cycle visualizer (Waveform→Spectrum→Spectrogram)
  Alt+M           Show similar albums/tracks
  Alt+G           Go to Album (navigate to track's album in Library)
  Alt+W           Save queue/radio as playlist

STATIONS & SHORTCUTS (Alt+/ twice to see available shortcuts)
  Ctrl+Alt+A      Play track album (play album of current track)
  Ctrl+Alt+L      Library Radio (station based on your library)
  Ctrl+Alt+R      Random Album Radio (shuffled albums)
  Ctrl+Alt+S      Quick library switcher

SEARCH POPUP (Ctrl+F)
  Type to search library (instant local search)
  Tab / Shift+Tab Switch result tabs (All/Artists/Albums/Tracks/Playlists/Genres)
  Left / Right    Switch result tabs
  Enter / Down    Move to results
  Up (at top)     Back to search input
  Enter           Navigate to selected item in library
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
  Tab             Toggle focus: track list / stations
  Left/Right      Switch focus between panels
  Del             Remove track from queue
  Shift+Up/Down   Move selected track up/down in queue
  Ctrl+Z          Undo last queue remix
  Alt+W           Save queue/radio as playlist
  Enter on playing track → opens Now Playing view
  Stations panel  Browse Plex radio stations, DJ modes, and remix tools
                  Enter/Right     Drill into category or play station
                  Left/Backspace  Go back (at root: return to track list)

NOW PLAYING (Ctrl+N)
  Album art, track info, and visualizer panel
  Visualizer tabs Waveform / Spectrum / Spectrogram
  Tab             Cycle visualizer tab forward
  Shift+Tab       Cycle visualizer tab backward
  Alt+V           Cycle visualizer tab
  Up              Focus tab bar (then Left/Right to switch, Down to return)
  Left/Right      Seek ±1 second
  Esc             Return to Queue view

ARTIST RADIO
  In Library, drill into an artist to see "Artist Radio" above "All Tracks".
  Press Enter to start Plex radio seeded from that artist.
  Multi-artist radio: select "Artist Radio" in the stations panel to open
  the picker. Enter count (2-12), filter and select artists, press Tab to
  launch blended radio from multiple artists.

SONIC ADVENTURE (Alt+A or via stations panel)
  Creates a sonic bridge between two tracks using Plex sonic analysis.
  Alt+A method:
    1. Select start track, press Alt+A
    2. Navigate to end track, press Alt+A
    3. Enter length (5-100 tracks)
    4. Adventure replaces queue, starts playing
  Stations panel: "Sonic Adventure" item provides a self-contained UI
  Tracks can be selected from Browse or Search (Tracks tab)

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
  Ctrl+Z undoes the last remix operation.

    Remix: Gemini   Insert similar tracks between each queue pair
    Remix: Twofer   Insert same-artist tracks between each queue pair
    Remix: Stretch  Insert sonic bridge tracks between each queue pair
    Remix: Shuffle  Shuffle the queue (press again to undo)

PLAYLISTS (Ctrl+P)
  Miller columns navigation (drill down into playlists)

GENRES (Ctrl+G)
  Tab             Focus tab bar (All / Library / Artist / Album / Mood / Style)
  Left/Right      Switch tabs (when tab bar focused)
  Down/Enter      Return to content from tab bar
  Up (at top)     Focus tab bar
  All tab         Merged list of all genre types with type suffix

FOLDERS (Ctrl+O)
  ♪ icon shows currently playing track

SETTINGS (F2)
  F2              Account, libraries, playback, output, themes
  Output section  Select playback target (local or remote Plex player)
                  Remote players: Apple TV, Plexamp on phone, etc.
                  Audio plays on remote device; textamp acts as controller
  About section   Themes, artwork mode (Auto / Halfblocks / Braille)
                  Braille: 2x4 dot-art rendering for Apple Terminal

GENERAL
  F5              Refresh current view (updates cache)
  Ctrl+Q          Quit
"#;

    // Count lines for scroll clamping
    let line_count = help_text.lines().count() as u16;
    let visible_lines = inner.height;
    let max_scroll = line_count.saturating_sub(visible_lines);
    let scroll = state.help_scroll.min(max_scroll);

    let paragraph = Paragraph::new(help_text.trim())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);
}
