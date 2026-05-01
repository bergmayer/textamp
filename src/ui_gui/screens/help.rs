//! Help screen — full keybindings reference.
//!
//! Mirrors the TUI's `src/ui/screens/help.rs::HELP_TEXT` content (same
//! source-of-truth text for both UIs). Rendered as a scrollable monospace
//! block so the columnar layout reads the same way it does in the TUI.

use iced::widget::{column, container, scrollable};
use iced::{Background, Border, Color, Element, Font, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
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
  Click track       Open track-details pane to the right (no playback)
                    Pane has a "Play Track" button.
  Right-click track Play track / Play track and following / queue ops
  Click album       Drill into album tracks (with "Play Album" button)
  Double-click      Play immediately (replaces queue):
                    Album -> play album, Playlist -> play playlist,
                    Folder -> play folder
  Ctrl+E            Add to end of queue (track + following, or album)
  Ctrl+Shift+E      Play next in queue (after current track)

QUEUE REORDER (Queue view)
  Drag the "=" handle on a row up/down to reorder by mouse.
  Or use the up/down arrows on each row.

  Space             Play / Pause
  Shift+Left/Right  Seek ±10 seconds
  Ctrl+Shift+Up/Dn  Volume

  F1 Help  F2 Settings  F3 Switch library  F4 Artist bio
  F5 Refresh           F6 Sort

  Ctrl+F  Search        Ctrl+M  Similar   Ctrl+R  Related
  Ctrl+J  Jump to album
  Ctrl+S  Save queue as playlist
  Ctrl+W (Cmd+W on Mac)  Close current column / details pane
  Quit:   Ctrl+Q (Linux/Windows) | Cmd+Q (Mac) | Alt+F4 (Windows)
  Ctrl+X  Clear queue   Ctrl+Z  Undo

  Alt+R  Random Album Radio

  Search streaming services for selected or now-playing album/artist:
  Tools menu → Search Apple Music / Spotify / YouTube

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

pub fn view(_state: &AppState) -> Element<'_, GuiMessage> {
    let body = scrollable(
        container(
            text(HELP_TEXT.trim_start())
                .size(15)
                .font(Font::MONOSPACE),
        )
        .padding([4, 12]),
    )
    .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
    .style(crate::ui_gui::widgets::chunky_scrollable_style)
    .height(Length::Fill);

    container(
        column![
            text("Keyboard shortcuts  (Esc to close, ↑↓ PgUp/PgDn to scroll)").size(16),
            body,
        ]
        .spacing(8)
        .padding(12),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(palette.background.base.color)),
            border: Border { color: palette.background.strong.color, width: 0.0, radius: 0.0.into() },
            text_color: Some(palette.background.base.text),
            ..container::Style::default()
        }
    })
    .into()
}

// Unused if iced silently strips it; kept to avoid wiring warnings.
#[allow(dead_code)]
const _IGNORE_COLOR: Color = Color::BLACK;
