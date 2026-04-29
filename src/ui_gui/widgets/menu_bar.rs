//! Top-of-window menu bar with pull-down menus.
//!
//! Renders a classic desktop-style menu strip (File / View / Playback /
//! Queue / Tools / Help) entirely in-window — no native OS menu integration.
//! This keeps behaviour identical across Windows, macOS, Linux, and WSLg
//! without a GTK3 dependency.
//!
//! Each dropdown lists its items with a keyboard shortcut hint. Clicking an
//! item dispatches an `Action` through the normal `GuiMessage::Action`
//! pipeline. Clicking the matching top-level button a second time, pressing
//! Escape, or clicking elsewhere closes any open menu.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use iced::widget::{button, column, container, mouse_area, row, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Theme};

use crate::app::action::{
    NavigationAction, PlaybackAction, QueueAction, RadioAction, SearchAction, SettingsAction, SystemAction,
};
use crate::app::state::{BrowseCategory, DjMode, View};
use crate::app::{Action, AppState};
use crate::ui_gui::message::GuiMessage;

/// Top-level menus, in the order they appear in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopMenu {
    File,
    View,
    Playback,
    Queue,
    Radio,
    Tools,
    Help,
}

impl TopMenu {
    pub const ALL: [TopMenu; 7] = [
        TopMenu::File,
        TopMenu::View,
        TopMenu::Playback,
        TopMenu::Queue,
        TopMenu::Radio,
        TopMenu::Tools,
        TopMenu::Help,
    ];

    fn label(self) -> &'static str {
        match self {
            TopMenu::File => "File",
            TopMenu::View => "View",
            TopMenu::Playback => "Playback",
            TopMenu::Queue => "Queue",
            TopMenu::Radio => "Radio",
            TopMenu::Tools => "Tools",
            TopMenu::Help => "Help",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    /// Width of the top-level button for this menu. Sized to fit the
    /// label plus a small horizontal inset — fixed-width-per-menu
    /// keeps the dropdown positioning math trivial while letting
    /// "Playback" have more room than "File" without stretching every
    /// button to the widest.
    fn button_width(self) -> f32 {
        match self {
            TopMenu::File     => 44.0,
            TopMenu::View     => 48.0,
            TopMenu::Playback => 72.0,
            TopMenu::Queue    => 56.0,
            TopMenu::Radio    => 52.0,
            TopMenu::Tools    => 52.0,
            TopMenu::Help     => 46.0,
        }
    }

    /// Sum of button widths + gaps up to (but not including) this menu.
    /// Used to position the dropdown panel horizontally under the
    /// matching top-level button.
    fn x_offset(self) -> f32 {
        Self::ALL.iter()
            .take(self.index())
            .map(|m| m.button_width() + BTN_GAP)
            .sum()
    }
}

/// One item in a dropdown (or a separator).
enum Item {
    Sep,
    /// Fires a concrete `Action` via the shared dispatcher.
    Entry {
        label: String,
        shortcut: &'static str,
        action: Action,
        /// When false, the item renders dimmed and the click handler is
        /// suppressed. Used for context-sensitive items like
        /// "Group by Album" which only applies in Playlists view.
        enabled: bool,
    },
    /// Fires a synthetic keystroke through `key_input::handle_key`, so the
    /// menu item behaves identically to the TUI shortcut (including
    /// state-dependent context resolution, e.g. "similar to the highlighted
    /// artist/album/track").
    KeyEntry {
        label: &'static str,
        shortcut: &'static str,
        code: KeyCode,
        mods: KeyModifiers,
        /// When false, the row renders dimmed and the click handler is
        /// suppressed — same convention as `Item::Entry`'s `enabled`.
        enabled: bool,
    },
    /// Opens the About Textamp popup (GUI-only, no Action / no keystroke).
    About,
    /// Toggles cover-art thumbnails in Miller columns (GUI-only state flip).
    CoverArtToggle,
    /// Opens the User Guide modal (Help → User Guide). GUI-only.
    UserGuide,
    /// Opens the Keyboard Shortcuts modal (Help → Keyboard Shortcuts).
    KeyboardShortcuts,
    /// Generic GUI-only menu entry that dispatches a `GuiMessage`
    /// directly (rather than an `Action`). Used for things like
    /// "Stations…" → OpenStationsPopup that don't have a clean
    /// `Action` representation.
    Custom {
        label: String,
        shortcut: &'static str,
        message: GuiMessage,
        enabled: bool,
    },
}

fn entry(label: &'static str, shortcut: &'static str, action: Action) -> Item {
    Item::Entry { label: label.to_string(), shortcut, action, enabled: true }
}

fn entry_with(label: impl Into<String>, shortcut: &'static str, action: Action, enabled: bool) -> Item {
    Item::Entry { label: label.into(), shortcut, action, enabled }
}

fn key_entry_with(label: &'static str, shortcut: &'static str, mods: KeyModifiers, code: KeyCode, enabled: bool) -> Item {
    Item::KeyEntry { label, shortcut, code, mods, enabled }
}

fn items_for(menu: TopMenu, state: &AppState) -> Vec<Item> {
    use crate::app::state::{ConnectionState, PlayStatus};

    // Convenience flags for context-sensitive enable/disable. Each one
    // is derived from `state` so the menu reflects whatever the user
    // could actually do at the moment (e.g. "Next Track" is dim until
    // there's something queued).
    let connected = matches!(state.connection, ConnectionState::Connected { .. });
    let has_track = state.current_track().is_some();
    let is_playing = matches!(state.playback.status, PlayStatus::Playing | PlayStatus::Paused);
    let queue_has_tracks = !state.queue.tracks.is_empty();
    let radio_has_tracks = !state.radio.tracks.is_empty();
    let any_playable = queue_has_tracks || radio_has_tracks;
    let has_active_library = state.active_library.is_some() && connected;
    let has_multiple_libs = state.libraries.len() > 1
        || (connected && state.available_servers.len() > 1);
    // "Add to queue" / "Play next" need something selected in a browse
    // list. The shared `key_input::handle_key` ultimately gates this,
    // but for the menu we want to grey out the rows when the user is
    // looking at Now Playing / Queue / Auth etc. so they don't try to
    // click into a no-op.
    let can_enqueue = connected
        && matches!(state.view, crate::app::state::View::Browse | crate::app::state::View::Search);

    match menu {
        TopMenu::File => vec![
            // Mac-style "app menu" entries sit at the top of File since
            // we don't have a dedicated app menu on non-macOS builds.
            Item::About,
            Item::Sep,
            entry("Settings\u{2026}",       "F2", Action::Navigation(NavigationAction::SetView(View::Settings))),
            entry_with("Switch Library\u{2026}", "F3", Action::Search(SearchAction::OpenLibraryPicker), has_multiple_libs),
            entry_with("Sign Out",               "",   Action::Settings(SettingsAction::Logout), connected),
            Item::Sep,
            // Quit shortcut label — show the platform-conventional one.
            // Cmd+Q on macOS, Alt+F4 on Windows, Ctrl+Q on Linux. The
            // shared key handler accepts all of them on every platform.
            entry(
                "Quit",
                if cfg!(target_os = "macos") { "Cmd+Q" }
                else if cfg!(target_os = "windows") { "Alt+F4" }
                else { "Ctrl+Q" },
                Action::System(SystemAction::Quit),
            ),
        ],
        TopMenu::View => {
            use crate::app::state::ColumnSortMode;

            // Sort options are context-sensitive: only the modes
            // applicable to the focused Miller column are enabled.
            // Trying to sort an Artist column by duration is
            // meaningless, so we grey those rows out instead of
            // letting the user click into a no-op.
            let focused_type = focused_sort_column_type(state);
            let sort_available = |mode: ColumnSortMode| -> bool {
                focused_type
                    .map(|t| t.available_modes().contains(&mode))
                    .unwrap_or(false)
            };
            let mk_sort = |label: &'static str, mode: ColumnSortMode| -> Item {
                entry_with(label, "", Action::Search(SearchAction::ApplyFocusedSortMode(mode)), sort_available(mode))
            };

            vec![
            entry("Browse",      "",       Action::Navigation(NavigationAction::SetView(View::Browse))),
            entry("Queue",       "Ctrl+U", Action::Navigation(NavigationAction::SetView(View::Queue))),
            entry("Now Playing", "Ctrl+N", Action::Navigation(NavigationAction::SetView(View::NowPlaying))),
            Item::Sep,
            entry_with("Library",     "Ctrl+L", Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Library)),   connected),
            entry_with("Playlists",   "Ctrl+P", Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Playlists)), connected),
            entry_with("Genres",      "Ctrl+G", Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Genres)),    connected),
            entry_with("Folders",     "Ctrl+O", Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Folders)),   connected),
            Item::Sep,
            // Similar / Related / Open-in-Library / Artist Bio resolve
            // their target from the current selection, so they only
            // make sense in Browse / Now Playing — and only once a
            // library has loaded.
            key_entry_with("Similar Albums\u{2026}",  "Ctrl+M", KeyModifiers::CONTROL, KeyCode::Char('m'), connected),
            key_entry_with("Related Artists\u{2026}", "Ctrl+R", KeyModifiers::CONTROL, KeyCode::Char('r'), connected),
            key_entry_with("Open in Library",         "Ctrl+J", KeyModifiers::CONTROL, KeyCode::Char('j'), connected),
            key_entry_with("Artist Bio",              "F4",     KeyModifiers::NONE,    KeyCode::F(4),     connected),
            Item::Sep,
            // Sort actions — flat list, operate on the focused Miller
            // column. Each entry is enabled only when the focused
            // column type actually supports that mode (see
            // `SortColumnType::available_modes`). The TUI still uses
            // Ctrl+S → popup; the GUI expresses these as direct menu
            // items instead so there's no modal blocking the view.
            mk_sort("Sort: Default",     ColumnSortMode::Default),
            mk_sort("Sort: By Artist",   ColumnSortMode::ByArtist),
            mk_sort("Sort: By Album",    ColumnSortMode::ByAlbum),
            mk_sort("Sort: By Title",    ColumnSortMode::ByTitle),
            mk_sort("Sort: By Duration", ColumnSortMode::ByDuration),
            mk_sort("Sort: Shuffle",     ColumnSortMode::Shuffled),
            Item::Sep,
            entry_with(
                "Reverse Sort Direction",
                "",
                Action::Search(SearchAction::ReverseFocusedSortDirection),
                focused_type.is_some(),
            ),
            // Group by Album / Group by Track:
            //   - Only meaningful in Playlists view (the Library and
            //     Genres views always show albums).
            //   - Label flips to "Group by Track" when the focused
            //     playlist column is currently grouped.
            {
                let in_playlists = state.browse_category == BrowseCategory::Playlists;
                let grouped = in_playlists && state.playlist_nav.focused()
                    .map_or(false, |c| c.grouped_by_album);
                let label = if grouped { "Group by Track" } else { "Group by Album" };
                entry_with(label, "", Action::Search(SearchAction::ToggleFocusedColumnGrouping), in_playlists)
            },
            Item::Sep,
            Item::CoverArtToggle,
        ]
        }
        TopMenu::Playback => vec![
            // Transport rows are gated on whether there's something to
            // act on. Without a current track Play/Pause / Stop /
            // Prev / Next / Seek are no-ops; keeping them clickable
            // would just confuse the user.
            entry_with("Play / Pause",     "Space", Action::Playback(PlaybackAction::TogglePlayPause), has_track),
            entry_with("Stop",             "",      Action::Playback(PlaybackAction::Stop),            is_playing),
            Item::Sep,
            entry_with("Previous Track",   "",      Action::Playback(PlaybackAction::Previous),        any_playable),
            entry_with("Next Track",       "",      Action::Playback(PlaybackAction::Next),            any_playable),
            Item::Sep,
            entry_with("Seek Back 10s",    "Shift+\u{2190}", Action::Playback(PlaybackAction::SeekRelative(-10_000)), has_track),
            entry_with("Seek Forward 10s", "Shift+\u{2192}", Action::Playback(PlaybackAction::SeekRelative(10_000)),  has_track),
            Item::Sep,
            entry("Volume Up",        "Ctrl+Shift+\u{2191}", Action::Playback(PlaybackAction::VolumeUp)),
            entry("Volume Down",      "Ctrl+Shift+\u{2193}", Action::Playback(PlaybackAction::VolumeDown)),
            entry("Mute / Unmute",    "",      Action::Playback(PlaybackAction::ToggleMute)),
        ],
        TopMenu::Queue => {
            // DJ modes operate on the live playback queue. Greyed
            // out when the queue is empty (or radio mode is active
            // — the modes only manipulate the user-built queue).
            let dj_enabled = queue_has_tracks;
            // Remix tools rewrite or shuffle the existing queue, so
            // they too need a non-empty queue.
            let remix_enabled = queue_has_tracks;
            // "Undo Shuffle" is the only Remix entry that needs the
            // shuffle-undo stash to actually be populated.
            let undo_enabled = remix_enabled && state.queue.shuffle_undo_queue.is_some();
            vec![
            entry_with("Add to end of queue",    "Cmd+E",       Action::Queue(QueueAction::EnqueueSelection),     can_enqueue),
            entry_with("Play next in queue",     "Cmd+Shift+E", Action::Queue(QueueAction::EnqueueSelectionNext), can_enqueue),
            Item::Sep,
            entry_with("Save queue as playlist\u{2026}", "Cmd+S", Action::Queue(QueueAction::PromptSavePlaylist), any_playable),
            entry_with("Clear Queue",            "Cmd+X",       Action::Queue(QueueAction::ClearQueue),           any_playable),
            entry_with("Shuffle",                "",             Action::Queue(QueueAction::ToggleQueueShuffle),   any_playable),
            Item::Sep,
            // ── DJ Modes ──
            // Modes are listed flat (no submenu primitive yet) but
            // visually grouped between separators. Each toggles
            // continuous insertion of one extra track per
            // transition. Greyed when no queue.
            entry_with(DjMode::Stretch.name(),  "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Stretch)),  dj_enabled),
            entry_with(DjMode::Gemini.name(),   "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Gemini)),   dj_enabled),
            entry_with(DjMode::Freeze.name(),   "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Freeze)),   dj_enabled),
            entry_with(DjMode::Twofer.name(),   "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Twofer)),   dj_enabled),
            entry_with(DjMode::Contempo.name(), "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Contempo)), dj_enabled),
            entry_with(DjMode::Groupie.name(),  "", Action::Radio(RadioAction::ToggleDjMode(DjMode::Groupie)),  dj_enabled),
            Item::Sep,
            // ── Remix tools ──
            entry_with("Remix: Gemini",        "", Action::Queue(QueueAction::RemixGemini),       remix_enabled),
            entry_with("Remix: Twofer",        "", Action::Queue(QueueAction::RemixTwofer),       remix_enabled),
            entry_with("Remix: Stretch",       "", Action::Queue(QueueAction::RemixStretch),      remix_enabled),
            entry_with("Remix: Doppelganger",  "", Action::Queue(QueueAction::RemixDoppelganger), remix_enabled),
            entry_with("Remix: Shuffle",       "", Action::Queue(QueueAction::RemixShuffle),      remix_enabled),
            entry_with("Remix: Undo Shuffle",  "", Action::Queue(QueueAction::RemixUndoShuffle),  undo_enabled),
        ]
        }
        TopMenu::Radio => {
            // Adventure + Artist Radio live here (both spin up a
            // streaming queue, so they're radio sources, not Tools).
            // Random Album moved to Tools — it just plays a random
            // album from the library, no streaming/radio behaviour.
            // "Stations…" opens the full stations popup which lists
            // every Plex station with category drill-down (the
            // dropdown can't host the full list cleanly without
            // submenu chrome we don't have, and the popup already
            // exists and handles it well).
            vec![
                entry_with("Artist Radio\u{2026}", "", Action::Search(SearchAction::OpenArtistRadioPicker), has_active_library),
                entry_with("Adventure\u{2026}",    "", Action::Search(SearchAction::OpenAdventureLauncher), has_active_library),
                Item::Sep,
                Item::Custom {
                    label: "Stations\u{2026}".to_string(),
                    shortcut: "",
                    message: GuiMessage::OpenStationsPopup,
                    enabled: has_active_library,
                },
            ]
        }
        TopMenu::Tools => {
            use crate::services::external_search::SearchTarget;
            vec![
            entry_with("Search\u{2026}",       "Cmd+F", Action::Search(SearchAction::OpenSearchPopup),         connected),
            key_entry_with("Random Album",     "Alt+R",  KeyModifiers::ALT, KeyCode::Char('r'), has_active_library),
            Item::Sep,
            // Web-search shortcuts — menu-driven only, no keyboard
            // accelerators on any of the three.
            entry_with("Search Apple Music\u{2026}", "", Action::System(SystemAction::OpenExternalSearch { target: SearchTarget::AppleMusic, query: None }), connected),
            entry_with("Search Spotify\u{2026}",     "", Action::System(SystemAction::OpenExternalSearch { target: SearchTarget::Spotify,    query: None }), connected),
            entry_with("Search YouTube\u{2026}",     "", Action::System(SystemAction::OpenExternalSearch { target: SearchTarget::YouTube,    query: None }), connected),
            Item::Sep,
            key_entry_with("Refresh",          "F5",     KeyModifiers::NONE, KeyCode::F(5),     connected),
        ]
        }
        TopMenu::Help => vec![
            Item::UserGuide,
            Item::KeyboardShortcuts,
        ],
    }
}

/// Map the focused Miller column to a `SortColumnType`, mirroring the
/// detection in `dispatch_search::SearchAction::OpenSortPopup`. Used
/// by the View menu to enable only the sort modes that apply to the
/// current column (`SortColumnType::available_modes`). Returns `None`
/// when no browse column is focused or the column has no sortable
/// content (e.g. the Genre category strip).
fn focused_sort_column_type(state: &AppState) -> Option<crate::app::state::SortColumnType> {
    use crate::app::state::{BrowseItem, SortColumnType, View};
    if state.view != View::Browse {
        return None;
    }
    let nav = state.browse_nav()?;
    let col_idx = nav.focused_column;
    let col = nav.columns.get(col_idx)?;
    let first_item = col.items.first();
    if first_item.map_or(false, |i| matches!(i, BrowseItem::Artist { .. }))
        || col.items.iter().take(3).any(|i| matches!(i, BrowseItem::Artist { .. }))
    {
        Some(SortColumnType::Artist)
    } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Album { .. }))
        || col.items.iter().take(4).any(|i| matches!(i, BrowseItem::Album { .. }))
    {
        Some(SortColumnType::Album)
    } else if first_item.map_or(false, |i| matches!(i, BrowseItem::Track { .. })) {
        if state.is_special_track_column(nav, col_idx) {
            Some(SortColumnType::AllTracks)
        } else {
            Some(SortColumnType::Track)
        }
    } else {
        None
    }
}

// ── Layout constants ────────────────────────────────────────────────────────

// Tightened to match Win32 menu bar proportions.
const BAR_HEIGHT: u16 = 28;
const BAR_HPAD: u16 = 8;
// Each top-level button has a per-menu width (see `TopMenu::button_width`)
// wide enough to fit its label ("Playback" is the longest). BTN_GAP adds
// breathing room between buttons and is shared by all.
const BTN_GAP: f32 = 8.0;
const DROPDOWN_WIDTH: f32 = 300.0;
const ITEM_HEIGHT: u16 = 24;

// ── View functions ──────────────────────────────────────────────────────────

/// The always-visible menu strip (top of the window).
pub fn bar(open: Option<TopMenu>, _state: &AppState) -> Element<'static, GuiMessage> {
    let mut buttons = row![]
        .spacing(BTN_GAP as u16)
        .padding(Padding::from([0, BAR_HPAD]));
    let any_open = open.is_some();
    for m in TopMenu::ALL {
        let active = Some(m) == open;
        buttons = buttons.push(top_button(m, active, any_open));
    }

    container(
        row![buttons, Space::with_width(Length::Fill)]
            .align_y(Alignment::Center)
            .height(Length::Fixed(BAR_HEIGHT as f32)),
    )
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(palette.background.weak.color)),
            border: Border {
                color: palette.background.strong.color,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        }
    })
    .width(Length::Fill)
    .height(Length::Fixed(BAR_HEIGHT as f32))
    .into()
}

/// Render a top-level menu button. When a dropdown is already open elsewhere,
/// hovering over a sibling top-button switches the open menu to this one —
/// standard Windows/macOS menu-bar behaviour.
fn top_button(menu: TopMenu, active: bool, any_open: bool) -> Element<'static, GuiMessage> {
    let click_msg = if active { GuiMessage::MenuClose } else { GuiMessage::MenuOpen(menu) };
    let btn = button(
        container(text(menu.label()).size(14))
            .center_y(Length::Fixed(BAR_HEIGHT as f32))
            .center_x(Length::Fill)
            .padding(Padding::from([0, 6])),
    )
    .on_press(click_msg)
    .padding(0)
    .style(move |theme: &Theme, _status| {
        let palette = theme.extended_palette();
        // Active = the strong/recessed bg + its readable text. Pulls
        // both halves from the same Pair so the active "this menu is
        // open" indicator stays visible even in strict-monochrome
        // themes (otherwise primary.weak.color collapses to the same
        // white as the menu strip itself, hiding the highlight).
        let (bg, fg) = if active {
            (palette.background.strong.color, palette.background.strong.text)
        } else {
            (Color::TRANSPARENT, palette.background.base.text)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border::default(),
            ..button::Style::default()
        }
    })
    .width(Length::Fixed(menu.button_width()));

    if any_open && !active {
        // Hover-to-switch: pointer enters this button while another menu
        // dropdown is open → switch to this menu. Emits once per enter.
        mouse_area(btn)
            .on_enter(GuiMessage::MenuOpen(menu))
            .into()
    } else {
        btn.into()
    }
}

/// Dropdown + dismiss backdrop. Returns `None` when no menu is open.
/// Call site should stack this on top of the rest of the UI so the dropdown
/// floats over content without pushing it down.
pub fn dropdown_overlay(open: Option<TopMenu>, state: &AppState) -> Option<Element<'static, GuiMessage>> {
    let menu = open?;
    let items = items_for(menu, state);
    let entry_count = items.iter()
        .filter(|i| matches!(i,
            Item::Entry { .. } | Item::KeyEntry { .. } | Item::Custom { .. }
            | Item::About | Item::CoverArtToggle
            | Item::UserGuide | Item::KeyboardShortcuts))
        .count() as u16;
    let sep_count = items.iter().filter(|i| matches!(i, Item::Sep)).count() as u16;
    let natural_height = entry_count * ITEM_HEIGHT + sep_count * 7 + 8;
    // Cap the dropdown so it can't grow taller than the viewport. The
    // Radio menu in particular grows with the number of Plex
    // stations a library exposes — without this cap, anything past
    // the bottom of the window would be invisible (no scroll). Using
    // a fixed pixel cap keeps the math simple without threading a
    // viewport-height dependency through the menu API; iced's
    // scrollable handles the overflow.
    const DROPDOWN_MAX_HEIGHT: f32 = 560.0;
    let scrollable_panel = natural_height as f32 > DROPDOWN_MAX_HEIGHT;
    let panel_height = if scrollable_panel { DROPDOWN_MAX_HEIGHT } else { natural_height as f32 };

    let mut col = column![].spacing(0).padding(4);
    for it in items {
        col = col.push(render_item(it));
    }

    let inner: Element<'static, GuiMessage> = if scrollable_panel {
        iced::widget::scrollable(col)
            .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
            .style(crate::ui_gui::widgets::chunky_scrollable_style)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    } else {
        col.into()
    };

    let panel = container(inner)
        .width(Length::Fixed(DROPDOWN_WIDTH))
        .height(Length::Fixed(panel_height))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                border: Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            }
        });

    // Offset the panel horizontally beneath the clicked top-level button,
    // summing the widths + gaps of every button to its left.
    let left_pad = BAR_HPAD as f32 + menu.x_offset();
    let top_pad = BAR_HEIGHT as f32;

    let positioned = container(panel)
        .padding(Padding { top: top_pad, right: 0.0, bottom: 0.0, left: left_pad })
        .width(Length::Fill)
        .height(Length::Fill);

    // Transparent backdrop beneath the dropdown — click-anywhere-to-close.
    let backdrop = mouse_area(
        container(Space::with_width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(GuiMessage::MenuClose);

    Some(iced::widget::stack![backdrop, positioned].into())
}

fn render_item(item: Item) -> Element<'static, GuiMessage> {
    match item {
        Item::Sep => container(
            Space::with_height(Length::Fixed(1.0)),
        )
        .width(Length::Fill)
        .height(Length::Fixed(7.0))
        .padding(Padding::from([3, 8]))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                border: Border {
                    color: palette.background.strong.color,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                background: None,
                ..container::Style::default()
            }
        })
        .into(),
        Item::Entry { label, shortcut, action, enabled } => {
            render_row(label, shortcut, GuiMessage::MenuItemClick(action), enabled)
        }
        Item::KeyEntry { label, shortcut, code, mods, enabled } => {
            let key_event = KeyEvent::new_with_kind_and_state(
                code, mods, KeyEventKind::Press, KeyEventState::NONE,
            );
            render_row(label.to_string(), shortcut, GuiMessage::MenuKeyClick(key_event), enabled)
        }
        Item::About => render_row("About Textamp\u{2026}".to_string(), "", GuiMessage::ShowAbout, true),
        Item::CoverArtToggle => render_row("Toggle Cover Art".to_string(), "", GuiMessage::ToggleCoverArt, true),
        Item::UserGuide => render_row("User Guide\u{2026}".to_string(), "F1", GuiMessage::OpenUserGuide, true),
        Item::KeyboardShortcuts => render_row("Keyboard Shortcuts\u{2026}".to_string(), "", GuiMessage::OpenKeyboardShortcuts, true),
        Item::Custom { label, shortcut, message, enabled } => render_row(label, shortcut, message, enabled),
    }
}

fn render_row(label: String, shortcut: &'static str, msg: GuiMessage, enabled: bool) -> Element<'static, GuiMessage> {
    let row_content = row![
        text(label).size(15).width(Length::Fill),
        text(shortcut).size(14),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding(Padding::from([0, 8]));

    let mut btn = button(row_content)
        .padding(0)
        .width(Length::Fill)
        .style(move |theme: &Theme, status| {
            let palette = theme.extended_palette();
            // Disabled rows render dimmed and don't react to hover.
            // Using `palette.background.weak.color` as a dim-text shade
            // works in default iced palettes (where weak is a faintly
            // tinted base) but collapses to invisible white-on-white
            // in strict-monochrome themes. Take the body's own text
            // colour and drop its alpha instead — that gives a
            // theme-neutral "dim" without depending on a derived
            // grey shade existing.
            if !enabled {
                let mut fg = palette.background.base.text;
                fg.a *= 0.45;
                return button::Style {
                    background: Some(Background::Color(Color::TRANSPARENT)),
                    text_color: fg,
                    border: Border::default(),
                    ..button::Style::default()
                };
            }
            // Hover inverts to the strong-bg pair so the hovered row
            // is unmistakable in every theme — including strict B/W
            // where primary.weak collapses to the same white as the
            // dropdown panel itself (no visible hover otherwise).
            let (bg, fg) = match status {
                button::Status::Hovered | button::Status::Pressed => {
                    (palette.background.strong.color, palette.background.strong.text)
                }
                _ => (Color::TRANSPARENT, palette.background.base.text),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border::default(),
                ..button::Style::default()
            }
        })
        .width(Length::Fill)
        .height(Length::Fixed(ITEM_HEIGHT as f32));
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}
