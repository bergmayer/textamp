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
VIEWS
  Ctrl+L  Library         Ctrl+P  Playlists       Ctrl+G  Genres
  Ctrl+O  Folders         Ctrl+U  Queue           Ctrl+N  Now Playing

BROWSE
  Left column shows categories (Library, Playlists, Genres, Folders).
  Right/Enter drills into a category. Left at root returns to categories.

COMMANDS
  Arrow keys, PgUp/PgDn, Home/End navigate lists
  Tab / Shift+Tab   Cycle views
  A-Z               Jump to first item starting with letter
  Shift+A-Z         Refine: jump to first item matching 2nd character
  / or Alt+F        Inline filter (type to narrow list, stays on column)

  Enter             Navigate to item (drill down)
                    On tracks: play track + all following (replaces queue)
  Double-click      Play immediately (replaces queue):
                    Album → play album, Playlist → play playlist,
                    Folder → play folder
  Ctrl+E            Add to end of queue (track + following, or album)
  Ctrl+Shift+E      Insert next in queue (after current track)

  Space             Play / Pause
  Shift+Left/Right  Seek ±10 seconds
  Ctrl+Shift+Up/Dn  Volume

  F1 Help  F2 Settings  F3 Switch library  F4 Artist bio  F5 Refresh
  
  Ctrl+F  Search        Ctrl+M  Similar   Ctrl+R  Related
  Ctrl+J  Jump to album
  Ctrl+S  Sort          Ctrl+Q  Quit      Ctrl+W  Save playlist
  Ctrl+X  Clear queue   Ctrl+Z  Undo
  
  Alt+R  Random Album Radio
  
  Search streaming services for selected or now-playing album or artist:
  Ctrl+Alt+A  Apple Music  Ctrl+Alt+S  Spotify  Ctrl+Alt+Y  YouTube

QUEUE (Ctrl+U)
  Del               Remove track(s) from queue
  Shift+Up/Dn       Reorder selected track(s)
  Ctrl+Shift+Up/Dn  Multi-select (toggle items while navigating)
  Ctrl+Z            Undo last remix/edit
  Esc               Clear multi-selection

STATIONS (Queue view, left panel)
  Stations play continuously, fetching more tracks as needed.
  DJ modes insert tracks into your queue as you listen:
    Interleaving (Gemini, Twofer, Stretch): one DJ pick between
      each of your original queue tracks.
    Continuous (Freeze, Contempo, Groupie): DJ picks after every
      track, so your queue keeps growing with new discoveries.
  Remix tools are one-time operations that process the whole queue
    at once (Gemini, Twofer, Stretch, Doppelganger, Shuffle).
  Sonic Adventure: Alt+A or via stations panel
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
            None,
        );
    }
}
