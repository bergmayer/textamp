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
    Transport,
    Footer,
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
    if workspace == View::Browse {
        if describing(state) {
            panes.extend([Pane::Description, Pane::DescriptionSubmit]);
        }
        if state.tui_sidebar_visible {
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
    } else if workspace == View::NowPlaying {
        panes.extend([Pane::Artwork, Pane::Visualizer]);
    }
    if !state.hit_regions.artist_links.is_empty() {
        panes.push(Pane::Artists);
    }
    if state.tui_queue_visible {
        panes.push(Pane::Queue);
    }
    panes.extend([Pane::Transport, Pane::Footer]);
    let current = if state.tui_footer_focus.is_some() {
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
        Pane::Sidebar => state.category_column_focused = true,
        Pane::Column(index) => {
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
            state.now_playing_focus = NowPlayingFocus::Artwork;
            state.visualizer_tab_focused = false;
        }
        Pane::Visualizer => {
            state.now_playing_focus = NowPlayingFocus::Tracks;
            state.visualizer_tab_focused = true;
        }
        Pane::Queue => {
            state.set_view(View::Queue);
            state.tui_queue_type_focused = !backwards;
            state.now_playing_focus = NowPlayingFocus::Tracks;
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

/// Footer controls in visual order; their commands use the same keyboard reducer.
pub const FOOTER: &[(KeyCode, &str, &str)] = &[
    (KeyCode::Char('['), " [ ", "Show / Hide Sidebar"),
    (KeyCode::Left, " ← ", "Back"),
    (KeyCode::Char(','), " ⚙ ", "Settings"),
    (KeyCode::Char('f'), " / ", "Search"),
    (KeyCode::Char(':'), " : ", "Commands"),
    (KeyCode::Char(']'), " ] ", "Show / Hide Queue"),
];

/// Activate a footer control through the normal input path.
pub fn footer(state: &mut AppState, index: usize) -> Vec<Action> {
    state.tui_footer_focus = None;
    match index {
        0 => vec![super::action::NavigationAction::ToggleSidebar.into()],
        1 => vec![super::action::NavigationAction::Back.into()],
        2 => vec![if state.view == View::Settings {
            super::action::NavigationAction::SetView(View::Browse).into()
        } else {
            super::action::SettingsAction::OpenSettings.into()
        }],
        3 => vec![super::action::SearchAction::OpenSearchPopup.into()],
        4 => {
            state.palette.player_context = false;
            super::command_palette::open(state)
        }
        5 => vec![super::action::NavigationAction::ToggleQueuePanel.into()],
        _ => vec![],
    }
}

/// Context for the selected terminal control, without tooltip overlays.
pub fn status(state: &AppState) -> Option<String> {
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
                Details => state
                    .browse_nav()
                    .and_then(|nav| nav.columns.get(focus.column))
                    .map_or("Details", |column| {
                        if column.has_album_details()
                            || matches!(column.selected_item(), Some(BrowseItem::Album { .. }))
                        {
                            "Album Details"
                        } else if matches!(column.selected_item(), Some(BrowseItem::Artist { .. }))
                        {
                            "Artist Details"
                        } else {
                            "Track Details"
                        }
                    }),
                PlaylistEdit => "Edit Playlist / Done",
                Filter => "Filter Column",
                Play => "Play",
                Next => "Play Next",
                Add => "Add to Queue",
                Options => "View Options",
                Close => "Close Column",
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
            .map(|control| control.label().to_owned());
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
