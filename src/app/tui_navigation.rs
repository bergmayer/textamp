//! Terminal pane traversal over the visible workspace and shared navigation actions.
use super::{Action, AppState, state::*};
use crossterm::event::KeyCode;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Description,
    DescriptionSubmit,
    Artists,
    Sidebar,
    Column(usize),
    Alphabet,
    Track,
    Related,
    Artwork,
    Visualizer,
    Queue,
    Rail,
    Transport,
    Footer,
}

/// Step to the parent in the single-column Library or dismiss the player at the root.
pub fn sheet_back(state: &mut AppState) -> Vec<Action> {
    if !state.tui_sidebar_visible || state.category_column_focused {
        return vec![super::action::NavigationAction::SetView(View::Browse).into()];
    }
    state.set_view(View::Browse);
    state.tui_browsing_player = true;
    if state.desktop_library_root == LibraryRoot::Radio && state.station_nav.can_go_left() {
        return vec![super::action::RadioAction::NavigateStationsBack.into()];
    }
    if state.track_pane_focused
        || state
            .browse_nav()
            .is_some_and(|n| n.focused_column > state.browse_column_offset())
        || (state.browse_category == BrowseCategory::Folders
            && state
                .folder_state
                .as_ref()
                .is_some_and(|n| n.focused_column > 0))
    {
        super::handlers::key_input::close_focused_browse_column(state);
    } else {
        state.tui_sidebar_group = None;
        state.focus_category_column();
    }
    vec![]
}

pub fn focus_root_after_close(state: &mut AppState) {
    if state.tui_player_raised() {
        state.tui_sidebar_group = None;
        state.focus_category_column();
    }
}

/// Whether Describe Music owns the library workspace.
pub fn describing(state: &AppState) -> bool {
    state.view == View::Browse
        && state.sources.nav_collection
            == Some(
                super::sources::navidrome::commands::CollectionKind::AudioMuse(
                    crate::audiomuse::Feature::Describe,
                ),
            )
}

/// Traverse panes without changing the workspace, selection or playback.
pub fn traverse(state: &mut AppState, backwards: bool) {
    let workspace = if state.view == View::Queue {
        state.tui_queue_workspace.view()
    } else {
        state.view
    };
    let mut panes = Vec::new();
    let sheet = state.tui_player_raised();
    if sheet {
        if state.tui_sidebar_visible || cfg!(target_os = "macos") {
            let pane = if state.tui_sidebar_visible
                && !state.category_column_focused
                && state
                    .hit_regions
                    .miller_columns
                    .as_ref()
                    .is_some_and(|r| !r.columns.is_empty())
            {
                Pane::Column(state.browse_nav().map_or(0, |nav| nav.focused_column))
            } else {
                Pane::Sidebar
            };
            panes.push(pane);
        }
        panes.extend([Pane::Artwork, Pane::Visualizer]);
    } else if workspace == View::Browse {
        if describing(state) {
            panes.extend([Pane::Description, Pane::DescriptionSubmit]);
        }
        if state.tui_sidebar_visible || cfg!(target_os = "macos") {
            panes.push(Pane::Sidebar);
        }
        if state.tui_sidebar_group.is_some() {
            panes.push(Pane::Column(0));
        } else if super::radio_navigation::visible(state) {
            panes.extend((0..state.station_nav.columns.len()).map(Pane::Column));
        } else if let Some(nav) = state.browse_nav() {
            panes.extend((state.browse_column_offset()..nav.columns.len()).map(Pane::Column));
        } else if let Some(folders) = &state.folder_state {
            panes.extend((0..folders.columns.len()).map(Pane::Column));
        }
        if state.alphabet_strip_visible() && !super::radio_navigation::visible(state) {
            panes.push(Pane::Alphabet);
        }
        if state.pane_track().is_some()
            || (super::radio_navigation::visible(state) && state.station_detail.is_some())
        {
            panes.push(Pane::Track);
        }
        if super::sonic_details::visible(state).is_some() {
            panes.push(Pane::Related);
        }
    }
    if !state.hit_regions.artist_links.is_empty() {
        panes.push(Pane::Artists);
    }
    if state.tui_queue_visible {
        panes.push(Pane::Queue);
    }
    if !RailControl::visible(state).is_empty() {
        panes.push(Pane::Rail);
    }
    panes.extend([Pane::Transport, Pane::Footer]);
    let current = if state.tui_rail_focus.is_some() {
        Pane::Rail
    } else if state.tui_footer_focus.is_some() {
        Pane::Footer
    } else if state.tui_transport_focus.is_some() {
        Pane::Transport
    } else if state.tui_artist_focus.is_some() {
        Pane::Artists
    } else if state.tui_description_submit_focused {
        Pane::DescriptionSubmit
    } else if state.tui_description_focused {
        Pane::Description
    } else if state.view == View::Queue {
        Pane::Queue
    } else if workspace == View::NowPlaying {
        if state.now_playing_focus == NowPlayingFocus::Artwork {
            Pane::Artwork
        } else {
            Pane::Visualizer
        }
    } else if state.alphabet_strip_focused {
        Pane::Alphabet
    } else if super::sonic_details::focused(state) {
        Pane::Related
    } else if state.track_pane_focused {
        Pane::Track
    } else if state.category_column_focused {
        Pane::Sidebar
    } else if super::radio_navigation::visible(state) {
        Pane::Column(state.station_nav.focused_column)
    } else {
        Pane::Column(
            state
                .browse_nav()
                .map(|nav| nav.focused_column)
                .or_else(|| state.folder_state.as_ref().map(|nav| nav.focused_column))
                .unwrap_or(0),
        )
    };
    let index = panes.iter().position(|pane| *pane == current);
    let next = match index {
        Some(index) if backwards => (index + panes.len() - 1) % panes.len(),
        Some(index) => (index + 1) % panes.len(),
        None if backwards => panes.len() - 1,
        None => 0,
    };
    state.tui_queue_type_focused = false;
    state.tui_transport_focus = None;
    state.tui_footer_focus = None;
    state.tui_rail_focus = None;
    state.tui_artist_focus = None;
    state.tui_column_header = None;
    state.tui_description_focused = false;
    state.tui_description_submit_focused = false;
    state.category_column_focused = false;
    state.alphabet_strip_focused = false;
    state.track_pane_focused = false;
    state.miller_scroll_manual = false;
    if let Some(child) = &mut state.sonic_details {
        child.details.focused = false;
    }
    if !matches!(panes[next], Pane::Queue) {
        state.set_view(workspace);
    }
    match panes[next] {
        Pane::Description => state.tui_description_focused = true,
        Pane::DescriptionSubmit => state.tui_description_submit_focused = true,
        Pane::Artists => {
            let mut links: Vec<_> = state
                .hit_regions
                .artist_links
                .iter()
                .filter(|link| link.generation == state.library_generation)
                .collect();
            links.sort_by_key(|link| (link.area.y, link.area.x));
            state.tui_artist_focus = if backwards {
                links.last()
            } else {
                links.first()
            }
            .map(|link| (*link).clone());
        }
        Pane::Sidebar => {
            if state.tui_player_raised() {
                state.set_view(View::Browse);
                state.tui_browsing_player = true;
            }
            state.category_column_focused = true;
        }
        Pane::Column(index) => {
            if sheet {
                state.set_view(View::Browse);
                state.tui_browsing_player = true;
            }
            if super::radio_navigation::visible(state) {
                state.station_nav.focused_column = index;
            } else if let Some(nav) = state.browse_nav_mut() {
                nav.focused_column = index;
                if let Some(column) = nav.focused_mut() {
                    column.header_selection = ColumnHeaderSelection::Items;
                }
            } else if let Some(nav) = &mut state.folder_state {
                nav.focused_column = index;
            }
            if !backwards
                && !super::radio_navigation::visible(state)
                && state.tui_sidebar_group.is_none()
                && let Some(control) = super::column_controls::controls(state, index)
                    .first()
                    .copied()
            {
                super::column_controls::focus(state, index, control);
            }
        }
        Pane::Alphabet => state.alphabet_strip_focused = true,
        Pane::Track => {
            state.track_pane_focused = true;
            state.track_pane_index = 0;
        }
        Pane::Related => {
            state.track_pane_focused = true;
            if let Some(child) = &mut state.sonic_details {
                child.details.focused = true;
            }
        }
        Pane::Artwork => {
            state.set_view(View::NowPlaying);
            state.now_playing_focus = NowPlayingFocus::Artwork;
            state.visualizer_tab_focused = false;
        }
        Pane::Visualizer => {
            state.set_view(View::NowPlaying);
            state.now_playing_focus = NowPlayingFocus::Tracks;
            state.visualizer_tab_focused = true;
        }
        Pane::Queue => {
            state.set_view(View::Queue);
            state.tui_queue_type_focused = !backwards;
            state.now_playing_focus = NowPlayingFocus::Tracks;
        }
        Pane::Rail => {
            let controls = RailControl::visible(state);
            state.tui_rail_focus = if backwards {
                controls.last()
            } else {
                controls.first()
            }
            .copied();
        }
        Pane::Transport => {
            let controls = super::transport_controls::visible(state);
            state.tui_transport_focus = if backwards {
                controls.last()
            } else {
                controls.first()
            }
            .copied();
        }
        Pane::Footer => state.tui_footer_focus = Some(if backwards { FOOTER.len() - 1 } else { 0 }),
    }
}

/// Controls in the always-visible queue shortcut rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailControl {
    Next,
    History,
    Settings,
    Search,
    Commands,
    Radio,
    QueueType,
    QueueTools,
    Clear,
}

impl RailControl {
    pub fn visible(state: &AppState) -> Vec<Self> {
        use super::queue_controls::Menu;
        let regions = &state.hit_regions;
        let mut controls: Vec<_> = regions
            .queue_tabs
            .iter()
            .map(
                |(_, history)| {
                    if *history { Self::History } else { Self::Next }
                },
            )
            .collect();
        for (key, control) in [
            (KeyCode::Char(','), Self::Settings),
            (KeyCode::Char('f'), Self::Search),
            (KeyCode::Char(':'), Self::Commands),
        ] {
            if regions
                .footer_controls
                .iter()
                .any(|(_, index)| FOOTER[*index].0 == key)
            {
                controls.push(control);
            }
        }
        if regions.rail_radio.is_some()
            && state
                .station_nav
                .columns
                .first()
                .is_some_and(|column| !column.stations().is_empty())
        {
            controls.push(Self::Radio);
        }
        controls.extend(regions.queue_menus.iter().map(|(_, menu)| match menu {
            Menu::QueueType => Self::QueueType,
            Menu::Tools => Self::QueueTools,
        }));
        if regions.queue_clear.is_some() && super::queue_controls::can_clear_tab(state) {
            controls.push(Self::Clear);
        }
        controls
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Next => "Playing Next",
            Self::History => "Listening History",
            Self::Settings => "Settings",
            Self::Search => "Search",
            Self::Commands => "Commands",
            Self::Radio => "Radio Shortcut",
            Self::QueueType => "Queue Type",
            Self::QueueTools => "Queue Tools",
            Self::Clear => "Clear current queue tab",
        }
    }

    pub fn activate(self, state: &mut AppState) -> Vec<Action> {
        use super::{action::*, queue_controls::Menu};
        match self {
            Self::Next | Self::History => {
                let history = self == Self::History;
                if state.tui_queue_visible && state.showing_queue_history == history {
                    return vec![NavigationAction::ToggleQueuePanel.into()];
                }
                let mut actions = vec![QueueAction::ShowHistory(history).into()];
                if !state.tui_queue_visible {
                    actions.push(NavigationAction::ToggleQueuePanel.into());
                }
                actions
            }
            Self::Settings | Self::Search | Self::Commands => {
                state.tui_rail_focus = None;
                let index = FOOTER
                    .iter()
                    .position(|(_, _, label)| *label == self.label())
                    .unwrap();
                footer(state, index)
            }
            Self::Radio => {
                let preferred = crate::config::load_config()
                    .ok()
                    .and_then(|c| c.ui.library_radio_station);
                let stations = state
                    .station_nav
                    .columns
                    .first()
                    .map(|col| col.stations())
                    .unwrap_or_default();
                let command =
                    super::radio_navigation::shortcut_station(stations, preferred.as_deref())
                        .map(super::radio_navigation::shortcut_command);
                command
                    .map(|command| super::command_palette::run(command, state))
                    .unwrap_or_default()
            }
            Self::QueueType => Menu::QueueType.open(state),
            Self::QueueTools => Menu::Tools.open(state),
            Self::Clear => super::transport_controls::activate(
                state,
                super::transport_controls::TransportControl::Clear,
            ),
        }
    }
}

/// Footer controls in visual order; their commands use the same keyboard reducer.
pub const FOOTER: &[(KeyCode, &str, &str)] = &[
    #[cfg(not(target_os = "macos"))]
    (KeyCode::Char('['), " [ ", "Show / Hide Sidebar"),
    (KeyCode::Left, " ← ", "Back"),
    (KeyCode::Char(','), " ⚙ ", "Settings"),
    (KeyCode::Char('f'), " / ", "Search"),
    (KeyCode::Char(':'), " : ", "Commands"),
    #[cfg(not(target_os = "macos"))]
    (KeyCode::Char(']'), " ] ", "Show / Hide Queue"),
];

/// Activate a footer control through the normal input path.
pub fn footer(state: &mut AppState, index: usize) -> Vec<Action> {
    state.tui_footer_focus = None;
    match FOOTER.get(index).map(|entry| entry.0) {
        Some(KeyCode::Char('[')) => vec![super::action::NavigationAction::ToggleSidebar.into()],
        Some(KeyCode::Left) => vec![super::action::NavigationAction::Back.into()],
        Some(KeyCode::Char(',')) => vec![if state.view == View::Settings {
            super::action::NavigationAction::SetView(View::Browse).into()
        } else {
            super::action::SettingsAction::OpenSettings.into()
        }],
        Some(KeyCode::Char('f')) => vec![super::action::SearchAction::OpenSearchPopup.into()],
        Some(KeyCode::Char(':')) => {
            state.palette.player_context = false;
            super::command_palette::open(state)
        }
        Some(KeyCode::Char(']')) => vec![super::action::NavigationAction::ToggleQueuePanel.into()],
        _ => vec![],
    }
}

/// Context for the selected terminal control, without tooltip overlays.
pub fn status(state: &AppState) -> Option<String> {
    if let Some(control) = state.tui_rail_focus {
        return Some(control.label().into());
    }
    if state.tui_queue_type_focused {
        return Some(format!("Queue Type: {}", super::queue_modes::label(state)));
    }
    if state.tui_description_submit_focused && describing(state) {
        return Some("Search".into());
    }
    if let Some(index) = state.tui_footer_focus {
        return FOOTER.get(index).map(|(_, _, label)| (*label).to_owned());
    }
    if let Some(control) = state.tui_transport_focus {
        return Some(control.label().to_owned());
    }
    if let Some(link) = &state.tui_artist_focus {
        return Some(format!("{} · Artist Details", link.name));
    }
    if let Some(focus) = state.tui_column_header {
        use super::column_controls::ColumnControl::*;
        return Some(
            match focus.control {
                ArtistBio => "Artist Biography",
                Details => "Album Info",
                DownloadAlbum => "Download Album",
                PlaylistEdit => "Edit Playlist / Done",
                Filter => "Filter Column",
                Play => "Play",
                Next => "Play Next",
                Add => "Add to Queue",
                Options => "View Options",
                Close => "Close Column",
                ArtistMix => "Artist Mix",
            }
            .into(),
        );
    }
    if state.view == View::NowPlaying && state.visualizer_tab_focused {
        return Some(state.visualizer_tab.name().into());
    }
    if state.view == View::Queue && state.now_playing_focus == NowPlayingFocus::Tools {
        return Some(super::queue_controls::Menu::Tools.label().into());
    }
    if state.track_pane_focused {
        let index = super::sonic_details::visible(state)
            .filter(|child| child.details.focused)
            .map_or(state.track_pane_index, |child| child.details.action_index);
        return super::column_controls::DetailControl::ALL
            .get(index)
            .map(|control| {
                use crate::app::playback_controls::TrackRange;
                control
                    .label_for(if state.tui_shift_held {
                        TrackRange::Following
                    } else {
                        TrackRange::Single
                    })
                    .to_owned()
            });
    }
    if state.view == View::Browse
        && let Some(id) = state
            .browse_nav()
            .and_then(|nav| nav.focused())
            .and_then(|col| col.selected_item())
            .and_then(|item| item.pending_download())
    {
        return super::downloads::pending_status(state, id)
            .map(|status| format!("{status} · x / Delete: Cancel download"));
    }
    None
}

/// A retained column filter captures typing only while its own column has focus.
pub fn filter_focused(state: &AppState) -> bool {
    matches!(state.popups.modal(), super::state::Modal::Closed)
        && !state.palette.open
        && state.list_filter.active
        && state.list_filter.category == state.browse_category
        && state.view == View::Browse
        && !state.category_column_focused
        && !state.track_pane_focused
        && !state.alphabet_strip_focused
        && state.tui_transport_focus.is_none()
        && state.tui_footer_focus.is_none()
        && state.tui_rail_focus.is_none()
        && state.tui_artist_focus.is_none()
        && !state.tui_description_focused
        && !state.tui_description_submit_focused
        && state
            .tui_column_header
            .is_none_or(|focus| focus.control == super::column_controls::ColumnControl::Filter)
        && state
            .browse_nav()
            .map(|nav| nav.focused_column)
            .or_else(|| state.folder_state.as_ref().map(|nav| nav.focused_column))
            == Some(state.list_filter.column)
}
