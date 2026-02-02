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
FIRST-RUN LOGIN
  Tab / Arrows    Navigate between fields
  Enter           Edit field / Submit / Select server
  Esc             Cancel editing

NAVIGATION
  Arrow keys      Navigate lists
  Tab             Toggle focus between panels
  Enter / Right   Select / Drill down / Play
  Left / Bksp     Go back / Move focus left
  Esc             Close view / Cancel adventure
  Page Up/Down    Scroll by page
  Home/End        Jump to top/bottom
  A-Z             Jump to first item starting with letter

CATEGORIES (Ctrl+key - works from any view)
  Ctrl+A          Artists (cycles: artists/album artists/albums)
  Ctrl+P          Playlists (cycles: all/recently added/recent)
  Ctrl+G          Genres (cycles: genres/artist/album/moods/styles)
  Ctrl+O          Folders (Miller columns)
  Ctrl+T          Stations (radio)

VIEWS (Ctrl+key)
  Ctrl+N          Now Playing (cycles: queue/recently played/visualizer)
  Ctrl+F          Search / Filter (tabbed)
  F1 / ?          This help screen
  F2              Settings

COMMANDS (Alt+key) - Act on Selection
  Alt+R           Create radio from selection
                  Track: similar individual tracks
                  Album: similar albums played in order
  Alt+E           Add selection to queue (enqueue)
                  Track: adds single track
                  Album: adds all tracks from album
  Alt+S           Show similar albums/tracks
  Alt+P           Save queue as playlist (Now Playing)
  Alt+V           Start Sonic Adventure (see below)
  Alt+]           Next track
  Alt+[           Previous track
  Alt+O           Cycle filter tabs (in Search view)
                  Cycle sort order (in Genres view)

SEARCH / FILTER (Ctrl+F)
  Tabs: All | Artists | Album Artists | Albums |
        Playlists | Tracks | Genres
  Tab / Shift+Tab Switch between tabs
  Alt+O           Cycle filter tabs
  Up / Down       Navigate results
  Type text       Enter search query
  Enter           Search (if query changed) or select
  Esc             Cancel / close
  All tab:        3-column view (Artists | Albums | Tracks)
                  Left/Right to switch column focus

PLAYBACK
  Space           Play / Pause
  Ctrl+Left       Previous track
  Ctrl+Right      Next track
  Ctrl+Up/Down    Volume up/down
  Shift+Left      Seek backward 10 seconds
  Shift+Right     Seek forward 10 seconds
  Play history syncs to Plex server

NOW PLAYING (Ctrl+N)
  Ctrl+N cycles:  Queue -> Recently Played -> Now Playing
  Queue mode      Current queue or radio tracks
                  Scroll up to see play history (~20 tracks)
                  Del to remove from queue
                  Alt+P to save as playlist
  Recently Played Albums played on this server
  Now Playing     Album art, track info, waveform seekbar
                  Left/Right seeks ±1s, click to seek

RADIO (Alt+R on selection)
  Track Radio     Similar individual tracks (sonic)
  Album Radio     Tracks from similar albums in order
  Station Radio   Plex curated stations (via Ctrl+T)

SONIC ADVENTURE (Alt+V)
  Creates a sonic bridge between two tracks.
  1. Select start track, press Alt+V
  2. Navigate to end track, press Alt+V
  3. Enter length (5-100 tracks)
  4. Adventure replaces queue, starts playing
  Tracks can be selected from Browse or Search (Tracks tab)
  Esc             Cancel adventure mode

PLAYLISTS (Ctrl+P in Browse)
  Ctrl+P cycles:  All -> Recently Added -> Recent Playlists
  All             All playlists
  Recently Added  Albums recently added to library
  Recent          Recently accessed playlists

GENRES (Ctrl+G in Browse)
  3-column Miller columns (Genre | Albums | Tracks)
  Ctrl+G cycles:  Genres -> Artist Genres -> Album Genres -> Moods -> Styles
  Alt+O           Cycle sort: artist/album artist/album
  Left / Right    Move focus between columns
  Up / Down       Navigate within current column
  Enter           Play selected track, or focus next column

STATIONS (Ctrl+T in Browse)
  Miller columns style navigation
  Enter / Right   Drill into category or play station
  Left / Bksp     Move focus to previous column
  › suffix indicates drillable category

FOLDERS (Ctrl+O in Browse)
  Miller columns style navigation
  Enter / Right   Open folder or play track
  Left / Bksp     Go back to parent folder
  ♪ icon shows currently playing track

SETTINGS (F2)
  Server          Username/password, server selection
                  Enter to edit credentials, restart to apply
  Libraries       Switch between music libraries
  Playback        View playback settings
  Interface       Change theme (Dark/Solarized/Borland)
  Data            Clear Cache & Reload / Sign Out

MOUSE SUPPORT
  Transport Bar   Click ▶/⏸ icon to play/pause
                  Click anywhere on seek bar to jump
                  Drag the ● indicator to scrub
  Bottom Bar      Click to switch views/categories
                  Click active item to cycle modes
  Browse View     Click left panel to select item
                  Click right panel to select
                  Double-click to drill down or play
  Now Playing     Click queue item to select
                  Double-click to jump to track
                  Click waveform to seek, drag ● to scrub
  Search          Click tabs to switch
                  Click results to select
  Settings        Click sections or items
  Scroll Wheel    Scroll lists (all views)

GENERAL
  F5              Refresh current view (updates cache)
  Ctrl+Q          Quit
  Ctrl+C          Quit
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
