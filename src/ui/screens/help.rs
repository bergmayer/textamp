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
  Tab             Next category (Library→Playlists→Genres→Radio→Folders→Now Playing)
  Shift+Tab       Previous category
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
  Ctrl+R          Radio (Plex curated stations)
  Ctrl+O          Folders

VIEWS
  Ctrl+N          Now Playing (cycles: queue/now playing)
  F1 / ?          This help screen
  F2              Settings

COMMANDS (Alt+/ to see available commands)
  Ctrl+F          Search popup (floating dialog)
  Alt+R           Start Plex radio from selection (track, album, or artist)
  Alt+Q           Add selection to queue (enqueue)
                  Track: adds single track
                  Album: adds all tracks from album
  Alt+S           Library: cycle all albums / shuffle / artists
                  Other views: shuffle column / queue
  Alt+M           Show similar albums/tracks
  Alt+B           Show Album (navigate to track's album)
  Alt+G           Go to Artist (navigate to track's artist)
  Alt+A           Sonic Adventure (see below)
  Alt+W           Save queue/radio as playlist
  Alt+C           Toggle cover art view (album grid with artwork)

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

NOW PLAYING (Ctrl+N)
  Ctrl+N cycles:  Queue -> Now Playing
  Queue mode      Current queue or radio tracks
                  Del to remove from queue
                  Alt+W to save as playlist
  Now Playing     Album art, track info, waveform seekbar

PLEX RADIO (Alt+R on selection)
  Starts Plex radio seeded from the selected track, album, or artist.
  Uses Plex server intelligence (sonic analysis, taste, popularity).
  Station Radio   Plex curated stations (via Ctrl+R Radio)

SONIC ADVENTURE (Alt+A or via Radio section)
  Creates a sonic bridge between two tracks.
  Alt+A method:
    1. Select start track, press Alt+A
    2. Navigate to end track, press Alt+A
    3. Enter length (5-100 tracks)
    4. Adventure replaces queue, starts playing
  Radio section: "Sonic Adventure" item provides a self-contained UI
  Tracks can be selected from Browse or Search (Tracks tab)

PLAYLISTS (Ctrl+P)
  Miller columns navigation (drill down into playlists)

RADIO (Ctrl+R)
  Miller columns navigation (browse Plex curated stations)

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
