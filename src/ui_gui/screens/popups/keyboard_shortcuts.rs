//! Keyboard Shortcuts popup — modal version of the existing Help
//! screen. Same body text the TUI shows under F1; rendered as a
//! scrollable monospace block so the columnar layout reads cleanly.

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Element, Font, Length, Theme};

use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

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
  A-Z               Alphabet jump (scrolls viewport in GUI; jumps + selects in TUI)
  Shift+A-Z         Refine: jump to first item matching 2nd character (TUI)
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

QUEUE REORDER (Now Playing view)
  Drag a queue row up/down to reorder by mouse.

  Space             Play / Pause
  Shift+Left/Right  Seek +/-10 seconds
  Ctrl+Shift+Up/Dn  Volume

  F1 Keyboard Shortcuts  F2 Settings  F3 Switch library  F4 Artist bio
  F5 Refresh             F6 Sort

  Ctrl+F  Search        Ctrl+M  Similar   Ctrl+R  Related
  Ctrl+J  Jump to album
  Ctrl+S  Save queue as playlist
  Quit:   Ctrl+Q (Linux) | Cmd+Q or Cmd+W (Mac) | Alt+F4 (Windows)
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

NOW PLAYING SIDEBAR (GUI)
  Each toggle button stays pressed while its panel/feature is active:
  Radio, Visualizer, DJ Modes, Remix Tools.
  Stations / DJ modes / Remix tools open as modal popups.

UI SCALE (GUI)
  Ctrl++ / Ctrl+=   Zoom in
  Ctrl+-            Zoom out
  Ctrl+0            Reset to 1.0x

STATIONS (Now Playing, "Radio" sidebar button)
  Stations play continuously, fetching more tracks as needed.
  DJ modes insert tracks into your queue as you listen:
    Interleaving (Gemini, Twofer, Stretch): one DJ pick between
      each of your original queue tracks.
    Continuous (Freeze, Contempo, Groupie): DJ picks after every
      track, so your queue keeps growing with new discoveries.
  Remix tools are one-time operations that process the whole queue
    at once (Gemini, Twofer, Stretch, Doppelganger, Shuffle).
  Sonic Adventure: via Tools menu or stations panel
"#;

pub fn view<'a>() -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseKeyboardShortcuts)
        .style(popout_button_style);

    let header = row![
        text("Keyboard Shortcuts").size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let body = scrollable(
        container(
            text(HELP_TEXT.trim_start())
                .size(13)
                .font(Font::MONOSPACE),
        )
        .padding([4, 12]),
    )
    .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
    .style(crate::ui_gui::widgets::chunky_scrollable_style)
    .height(Length::Fill);

    container(
        column![header, body].spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(720.0))
    .height(Length::Fixed(620.0))
    .style(frame_style)
    .into()
}

fn frame_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
