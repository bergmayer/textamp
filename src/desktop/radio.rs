//! Radio navigation projects the shared station tree; providers own child loads.
use crate::app::{Action, AppState, action::*, state::*};
use crate::library::station::Station;
use crossterm::event::KeyCode;
use std::hash::{Hash, Hasher};

#[derive(Clone)]
pub struct Column {
    pub key: String,
    pub title: String,
    pub stations: std::sync::Arc<[Station]>,
    pub selected: Option<String>,
}
#[derive(Clone)]
pub struct Model {
    pub focused_column: usize,
    pub columns: Vec<Column>,
    pub selected: Option<Station>,
    pub loading: bool,
    pub notice: String,
    pub error: bool,
}
pub use crate::app::radio_navigation::select;
pub fn visible(state: &AppState) -> bool {
    state.view == View::Browse
        && state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Radio
        && state.sources.nav_collection.is_none()
}
pub fn project(state: &AppState) -> Model {
    let mut model = Model {
        focused_column: state.station_nav.focused_column,
        columns: state
            .station_nav
            .columns
            .iter()
            .map(|column| Column {
                key: column.key.clone().unwrap_or_else(|| "radio-root".into()),
                title: if column.key.is_none() {
                    "Stations".into()
                } else {
                    column.title.clone()
                },
                stations: column.stations().into(),
                selected: column.selected_station().map(|station| station.key.clone()),
            })
            .collect(),
        selected: state
            .station_nav
            .columns
            .get(state.station_nav.focused_column)
            .and_then(StationColumn::selected_station)
            .cloned(),
        loading: state.station_nav.loading,
        error: state.notifications.last_error.is_some(),
        notice: state
            .notifications
            .last_error
            .clone()
            .or_else(|| state.notifications.status_message.clone())
            .unwrap_or_default(),
    };
    if state.popups.artist_radio_picker().is_some()
        && let Some(root) = model.columns.first_mut()
        && let Some(station) = root
            .stations
            .iter()
            .find(|s| s.identifier.as_deref() == Some("artist-picker"))
    {
        root.selected = Some(station.key.clone());
        model.selected = Some(station.clone());
    }
    model
}
impl Model {
    pub fn revision(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.focused_column.hash(&mut hash);
        self.loading.hash(&mut hash);
        self.notice.hash(&mut hash);
        self.error.hash(&mut hash);
        self.selected
            .as_ref()
            .map(|station| &station.key)
            .hash(&mut hash);
        for column in &self.columns {
            column.key.hash(&mut hash);
            column.selected.hash(&mut hash);
            for station in column.stations.iter() {
                station.key.hash(&mut hash);
                station.title.hash(&mut hash);
            }
        }
        hash.finish()
    }
}

pub fn key(state: &mut AppState, key: KeyCode) -> Vec<Action> {
    let focused = state.station_nav.focused_column;
    let model = project(state);
    let Some(column) = model.columns.get(focused) else {
        return vec![];
    };
    let index = column
        .stations
        .iter()
        .position(|station| Some(&station.key) == column.selected.as_ref())
        .unwrap_or(0);
    match key {
        KeyCode::Up | KeyCode::Down => {
            let next = if key == KeyCode::Up {
                index.saturating_sub(1)
            } else {
                (index + 1).min(column.stations.len().saturating_sub(1))
            };
            column
                .stations
                .get(next)
                .map(|station| station.key.clone())
                .map_or_else(Vec::new, |key| select(state, focused, &key))
        }
        KeyCode::Left => {
            state.station_nav.focused_column = focused.saturating_sub(1);
            vec![]
        }
        KeyCode::Right if focused + 1 < model.columns.len() => {
            state.station_nav.focused_column += 1;
            vec![]
        }
        KeyCode::Enter => {
            let Some(station) = column.stations.get(index) else {
                return vec![];
            };
            if station.is_category() {
                if focused + 1 < model.columns.len() {
                    state.station_nav.focused_column += 1;
                    vec![]
                } else {
                    select(state, focused, &station.key)
                }
            } else {
                vec![RadioAction::PlayStation(station.key.clone()).into()]
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radio_is_a_repeatable_library_destination() {
        let mut state = AppState::new();
        let config = crate::config::Config::default();
        super::super::surface::navigate(PaletteCommandKind::OpenRadioBrowser, &mut state, &config);
        assert!(visible(&state));
        assert!(matches!(state.popups.modal(), Modal::Closed));
        assert!(!state.desktop_player_visible);
        super::super::surface::navigate(PaletteCommandKind::GotoLibrary, &mut state, &config);
        assert!(!visible(&state));
        super::super::surface::navigate(PaletteCommandKind::OpenRadioBrowser, &mut state, &config);
        assert!(visible(&state));
    }
    #[test]
    fn browsing_categories_and_leaves_does_not_start_playback() {
        let mut state = AppState::new();
        state.station_nav.columns.push(StationColumn::new(
            None,
            "Stations".into(),
            crate::library::station::standard_stations(
                "nav-radio",
                crate::library::capabilities::NAVIDROME,
            ),
        ));
        let actions = select(&mut state, 0, "nav-radio/artist-picker");
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::OpenArtistRadioPicker)]
        ));
        let actions = select(&mut state, 0, "nav-radio/style");
        assert!(
            matches!(actions.as_slice(), [Action::Radio(RadioAction::DrillIntoStation(key, _))] if key == "nav-radio/style")
        );
        let mut leaf = state.station_nav.columns[0]
            .stations()
            .iter()
            .find(|station| station.identifier.as_deref() == Some("style"))
            .unwrap()
            .clone();
        leaf.key = "nav-radio/style?id=Hip%20Hop".into();
        leaf.title = "Hip Hop".into();
        state.station_nav.columns.push(StationColumn::new(
            Some("nav-radio/style".into()),
            "Genres".into(),
            vec![leaf],
        ));
        assert!(select(&mut state, 1, "nav-radio/style?id=Hip%20Hop").is_empty());
        assert!(state.queue.tracks.is_empty());
        assert!(
            matches!(key(&mut state, KeyCode::Enter).as_slice(), [Action::Radio(RadioAction::PlayStation(key))] if key == "nav-radio/style?id=Hip%20Hop")
        );
        assert!(select(&mut state, 1, "old-library-station").is_empty());
        assert!(select(&mut state, 0, "nav-radio/style").is_empty());
        assert_eq!(state.station_nav.columns.len(), 2);
        select(&mut state, 0, "nav-radio/randomAlbum");
        assert_eq!(state.station_nav.columns.len(), 1);
    }
}
