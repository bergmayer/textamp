//! Library navigation and inline configuration for radio stations.
use super::{Action, AppState, action::*, state::*};

pub(crate) fn shortcut_station<'a>(
    stations: &'a [crate::library::station::Station],
    preferred: Option<&str>,
) -> Option<&'a crate::library::station::Station> {
    preferred
        .and_then(|id| {
            stations
                .iter()
                .find(|s| s.identifier.as_deref().unwrap_or(&s.key) == id)
        })
        .or_else(|| {
            stations
                .iter()
                .find(|s| s.identifier.as_deref() == Some("randomAlbum"))
        })
        .or(stations.first())
}

pub(crate) fn shortcut_command(station: &crate::library::station::Station) -> PaletteCommandKind {
    if station.requires_input() {
        PaletteCommandKind::ConfigureStation(station.key.clone())
    } else {
        PaletteCommandKind::PlayStation(station.key.clone())
    }
}

pub fn visible(state: &AppState) -> bool {
    (state.view == View::Browse
        || (state.view == View::Queue && state.tui_queue_workspace == TuiWorkspace::Library))
        && state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Radio
}
pub fn inline_input(state: &AppState) -> bool {
    visible(state)
        && (state.popups.artist_radio_picker().is_some()
            || state
                .popups
                .adventure_launcher()
                .is_some_and(|form| form.purpose != TrackPickerPurpose::Journey))
}
pub(crate) fn seed_picker(station: &crate::library::station::Station) -> Option<SearchAction> {
    match station.identifier.as_deref() {
        Some("sonic") => Some(SearchAction::OpenTrackRadioPicker),
        Some("stayClose") => Some(SearchAction::OpenStayCloseRadioPicker),
        Some("artist-picker") => Some(SearchAction::OpenArtistRadioPicker),
        _ => None,
    }
}
pub fn select(state: &mut AppState, column: usize, key: &str) -> Vec<Action> {
    let Some((index, station)) = state.station_nav.columns.get(column).and_then(|column| {
        column
            .stations()
            .iter()
            .enumerate()
            .find(|(_, station)| station.key == key)
            .map(|(index, station)| (index, station.clone()))
    }) else {
        return vec![];
    };
    let selected = state.station_nav.columns[column].selected_index == index;
    let children = state
        .station_nav
        .columns
        .get(column + 1)
        .is_some_and(|child| child.key.as_deref() == Some(key));
    state.station_nav.focused_column = column;
    if station.is_category() && selected && (children || state.station_nav.loading) {
        return vec![];
    }
    let mut actions = vec![];
    if state.popups.artist_radio_picker().is_some() {
        actions.push(SearchAction::CloseArtistRadioPicker.into());
    }
    if state.popups.adventure_launcher().is_some() {
        actions.push(SearchAction::CloseAdventureLauncher.into());
    }
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.sources.nav_tasks.remove("station-children");
    state.station_nav.loading = false;
    state.station_nav.columns[column].selected_index = index;
    state.station_nav.columns.truncate(column + 1);
    state.station_detail = (!station.is_category()).then(|| station.key.clone());
    if station.is_category() {
        actions.push(RadioAction::DrillIntoStation(station.key, station.title).into());
    } else if let Some(input) = seed_picker(&station) {
        state.track_pane_focused = true;
        actions.push(input.into());
    }
    actions
}
/// A configuration choice opens the same selected station chain as manual browsing.
pub fn configure(state: &mut AppState, key: &str) -> Vec<Action> {
    let Some(column) = state
        .station_nav
        .columns
        .iter()
        .position(|column| column.stations().iter().any(|station| station.key == key))
    else {
        return vec![];
    };
    super::navigation_history::record(state);
    for slot in [
        "artist-browse",
        "album-browse",
        "genre-browse",
        "collection",
    ] {
        state.sources.nav_tasks.remove(slot);
    }
    state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
    state.tag_nav_request_id = state.tag_nav_request_id.wrapping_add(1);
    state.track_pane_open = false;
    state.track_pane_focused = false;
    state.sonic_details = None;
    state.pending_library_details = None;
    state.popups.close_all();
    state.palette.close();
    state.set_view(View::Browse);
    state.browse_category = BrowseCategory::Library;
    state.desktop_library_root = LibraryRoot::Radio;
    state.desktop_player_visible = false;
    state.library_navigation = super::navigation_history::LibraryNavigation::Columns(None);
    state.sources.nav_collection = None;
    state.category_column_focused = false;
    state.tui_sidebar_group = None;
    state.station_nav.columns.truncate(column + 1);
    for parent in 0..column {
        let next = state.station_nav.columns[parent + 1].key.clone();
        if let Some(index) = state.station_nav.columns[parent]
            .stations()
            .iter()
            .position(|station| Some(&station.key) == next.as_ref())
        {
            state.station_nav.columns[parent].selected_index = index;
        }
    }
    select(state, column, key)
}

pub fn key(state: &mut AppState, key: crossterm::event::KeyCode) -> Option<Vec<Action>> {
    use crossterm::event::KeyCode;
    let column = state.station_nav.focused_column;
    let current = state.station_nav.columns.get(column)?;
    let index = current.selected_index;
    match key {
        KeyCode::Up | KeyCode::Down if !state.track_pane_focused => {
            let next = if key == KeyCode::Up {
                index.saturating_sub(1)
            } else {
                (index + 1).min(current.stations().len().saturating_sub(1))
            };
            state.station_nav.columns[column].selected_index = next;
            Some(vec![])
        }
        KeyCode::Left => {
            state.track_pane_focused = false;
            state.station_nav.focused_column = column.saturating_sub(1);
            Some(vec![])
        }
        KeyCode::Right => {
            if state.station_nav.columns.len() > column + 1 {
                state.station_nav.focused_column += 1;
            } else if state.station_detail.is_some() {
                state.track_pane_focused = true;
            }
            Some(vec![])
        }
        KeyCode::Enter => {
            let station = current.stations().get(index)?.clone();
            state.track_pane_focused = !station.is_category();
            let actions = select(state, column, &station.key);
            if station.is_category() && state.station_nav.columns.len() > column + 1 {
                state.station_nav.focused_column = column + 1;
            }
            Some(actions)
        }
        KeyCode::Char('p') if state.track_pane_focused => Some(vec![
            RadioAction::PlayStation(current.stations().get(index)?.key.clone()).into(),
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_choice_opens_station_column_and_restores_previous_library() {
        let mut state = AppState::new();
        state.desktop_library_root = LibraryRoot::Artists;
        state.station_nav.columns = vec![StationColumn::new(
            None,
            "Radio".into(),
            crate::library::station::standard_stations(
                "nav-radio",
                crate::library::capabilities::NAVIDROME,
            ),
        )];
        state.playback.request_id = 42;
        let actions = configure(&mut state, "nav-radio/stayClose");
        assert!(visible(&state));
        assert_eq!(state.station_detail.as_deref(), Some("nav-radio/stayClose"));
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::OpenStayCloseRadioPicker)]
        ));
        assert_eq!(state.playback.request_id, 42);
        assert!(state.jump_history.head().is_some());
        super::super::navigation_history::back(&mut state);
        assert_eq!(state.desktop_library_root, LibraryRoot::Artists);
        assert!(state.station_detail.is_none());
    }
}
