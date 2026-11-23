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
    pub columns: Vec<Column>,
    pub selected: Option<Station>,
    pub loading: bool,
    pub notice: String,
    pub error: bool,
}
pub const ARTIST_PICKER: &str = "desktop:artist-radio";
pub fn visible(state: &AppState) -> bool {
    state.view == View::Browse
        && state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Radio
        && state.sources.nav_collection.is_none()
}
fn artist_station() -> Station {
    Station {
        key: ARTIST_PICKER.into(),
        title: "Artist Mix".into(),
        station_type: "station".into(),
        identifier: Some("artist-picker".into()),
        thumb: None,
        art: None,
        description: Some("Choose one or more artists for continuous artist radio.".into()),
    }
}
pub fn play(key: &str) -> Vec<Action> {
    if key == ARTIST_PICKER {
        vec![SearchAction::OpenArtistRadioPicker.into()]
    } else {
        vec![RadioAction::PlayStation(key.into()).into()]
    }
}
fn playable_kind(station: &Station) -> bool {
    !station.is_separator() && !station.is_action() && !station.is_dj_mode() && !station.is_remix()
}
pub fn project(state: &AppState) -> Model {
    let mut model = Model {
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
                stations: column
                    .stations
                    .iter()
                    .filter(|station| playable_kind(station))
                    .cloned()
                    .collect::<Vec<_>>()
                    .into(),
                selected: column.selected_station().map(|station| station.key.clone()),
            })
            .collect(),
        selected: state
            .station_nav
            .columns
            .get(state.station_nav.focused_column)
            .and_then(StationColumn::selected_station)
            .filter(|station| playable_kind(station))
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
    if state
        .sources
        .active
        .capabilities()
        .supports(crate::library::capabilities::Feature::ArtistRadio)
        && let Some(root) = model.columns.first_mut()
    {
        let mut stations = root.stations.to_vec();
        stations.insert(2.min(stations.len()), artist_station());
        root.stations = stations.into();
    }
    if state.popups.artist_radio_picker().is_some() {
        if let Some(root) = model.columns.first_mut() {
            root.selected = Some(ARTIST_PICKER.into());
        }
        model.selected = Some(artist_station());
    }
    model
}
impl Model {
    pub fn revision(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
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

pub fn select(state: &mut AppState, column: usize, key: &str) -> Vec<Action> {
    if column == 0
        && key == ARTIST_PICKER
        && project(state)
            .columns
            .first()
            .is_some_and(|column| column.stations.iter().any(|station| station.key == key))
    {
        return vec![SearchAction::OpenArtistRadioPicker.into()];
    }
    crate::app::radio_navigation::select(state, column, key)
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
        KeyCode::Right | KeyCode::Enter => {
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
            } else if key == KeyCode::Enter {
                play(&station.key)
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radio_is_a_repeatable_library_destination_and_artist_radio_is_a_peer() {
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
        assert!(matches!(
            play(ARTIST_PICKER).as_slice(),
            [Action::Search(SearchAction::OpenArtistRadioPicker)]
        ));
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
        let actions = select(&mut state, 0, "nav-radio/style");
        assert!(
            matches!(actions.as_slice(), [Action::Radio(RadioAction::DrillIntoStation(key, _))] if key == "nav-radio/style")
        );
        let mut leaf = state.station_nav.columns[0]
            .stations
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
