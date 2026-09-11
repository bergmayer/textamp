//! One per-library policy for sonic UI entry points and dispatched requests.
//! Availability is separate from server readiness: enabling permits an attempt.
use super::{ActiveSource, LibraryChoice};
use crate::app::action::*;
use crate::app::state::{DjMode, PaletteCommandKind, PlaybackMode, View};
use crate::app::{Action, AppState};
use crate::library::station::{Station, StationKind};

pub(super) fn nav_key(source: &crate::navidrome::Source, folder: &Option<String>) -> String {
    serde_json::to_string(&(
        "navidrome",
        &source.id,
        &source.url,
        &source.username,
        folder,
    ))
    .expect("string tuple")
}
pub fn choice_key(choice: &LibraryChoice, _state: &AppState) -> Option<String> {
    match choice {
        LibraryChoice::Navidrome { source, folder, .. } => Some(nav_key(source, folder)),
        _ => None,
    }
}
pub fn enabled(state: &AppState) -> bool {
    if !state
        .sources
        .active
        .capabilities()
        .supports(crate::library::capabilities::Feature::SonicSimilarity)
    {
        return false;
    }
    let key = match &state.sources.active {
        ActiveSource::Folder(_) | ActiveSource::None => return false,
        ActiveSource::Navidrome(s) => nav_key(&s.source, &s.client.folder),
    };
    !state.sources.sonic_disabled_libraries.contains(&key)
}
pub fn dj_requires_sonic(mode: DjMode) -> bool {
    matches!(mode, DjMode::Gemini | DjMode::Freeze | DjMode::Stretch)
}
pub fn dj_feature(mode: DjMode) -> crate::library::capabilities::Feature {
    use crate::library::capabilities::Feature;
    match mode {
        DjMode::Stretch => Feature::SonicPaths,
        DjMode::Gemini | DjMode::Freeze => Feature::SonicSimilarity,
        DjMode::Contempo => Feature::ReleaseYears,
        DjMode::Twofer | DjMode::Groupie => Feature::ArtistRadio,
    }
}
pub fn station_requires_sonic(station: &Station) -> bool {
    key_requires_sonic(&station.key) || matches!(station.kind(), StationKind::AlbumMix)
}
fn key_requires_sonic(key: &str) -> bool {
    matches!(
        key,
        "dj:gemini"
            | "dj:freeze"
            | "dj:stretch"
            | "remix:gemini"
            | "remix:stretch"
            | "remix:doppelganger"
    ) || matches!(
        key.split('?').next(),
        Some("nav-radio/sonic" | "nav-radio/albumMix" | "nav-radio/audioMood")
    ) || key.ends_with("/stations/sonic")
}
fn station_blocked(state: &AppState, key: &str) -> bool {
    key_requires_sonic(key)
        || state
            .stations
            .iter()
            .chain(state.station_nav.columns.iter().flat_map(|c| &c.stations))
            .any(|s| s.key == key && station_requires_sonic(s))
}
pub fn action_blocked(state: &AppState, action: &Action) -> bool {
    if enabled(state) {
        return false;
    }
    match action {
        Action::Source(super::SourceAction::AudioMuse(command)) => command.requires_analysis(),
        Action::Source(super::SourceAction::Navidrome(action))
            if matches!(
                action.as_ref(),
                super::navidrome::NavAction::Command(
                    super::navidrome::commands::Command::Collection(
                        super::navidrome::commands::CollectionKind::AudioMuse(_)
                    )
                ) | super::navidrome::NavAction::Command(
                    super::navidrome::commands::Command::Analysis
                )
            ) =>
        {
            true
        }
        Action::Data(
            DataAction::LoadSimilarAlbums { .. }
            | DataAction::LoadSimilarTracks { .. }
            | DataAction::LoadTrackPaneSimilar { .. }
            | DataAction::LoadSimilarArtists { .. },
        ) => true,
        Action::Search(
            SearchAction::OpenAdventureLauncher
            | SearchAction::OpenAdventureLauncherWithStart { .. }
            | SearchAction::AdventureLauncherGenerate,
        ) => true,
        Action::Settings(
            SettingsAction::SetAdventureLength(_)
            | SettingsAction::AdventureComplete(_)
            | SettingsAction::AdventureGenerated { .. },
        ) => true,
        Action::Queue(
            QueueAction::RemixGemini | QueueAction::RemixStretch | QueueAction::RemixDoppelganger,
        ) => true,
        Action::Radio(RadioAction::ToggleDjMode(mode)) => dj_requires_sonic(*mode),
        Action::Radio(RadioAction::DjModeProcess) => {
            state.dj.active_mode.is_some_and(dj_requires_sonic)
        }
        Action::Radio(RadioAction::PlayStation(key) | RadioAction::DrillIntoStation(key, _)) => {
            station_blocked(state, key)
        }
        Action::Radio(RadioAction::StartSonicRadio(_)) => true,
        Action::Radio(RadioAction::StartStation(station)) => {
            matches!(
                station.source,
                crate::library::station::RadioSource::Sonic(_)
            ) || station
                .source
                .station_key()
                .is_some_and(|key| station_blocked(state, key))
        }
        _ => false,
    }
}
pub fn context_visible(
    state: &AppState,
    kind: &crate::services::track_context::ContextKind,
) -> bool {
    use crate::services::track_context::ContextKind;
    enabled(state)
        || !matches!(
            kind,
            ContextKind::SonicAdventure
                | ContextKind::SonicRadio
                | ContextKind::ShowSimilarTracks { .. }
                | ContextKind::ShowSimilarAlbums { .. }
        )
}
pub fn command_visible(state: &AppState, command: &PaletteCommandKind) -> bool {
    if enabled(state) {
        return true;
    }
    match command {
        PaletteCommandKind::OpenSimilar
        | PaletteCommandKind::SonicAdventure
        | PaletteCommandKind::SonicAdventureFromFocusedTrack
        | PaletteCommandKind::RemixGemini
        | PaletteCommandKind::RemixStretch
        | PaletteCommandKind::RemixDoppelganger => false,
        PaletteCommandKind::ToggleDj(mode) => !dj_requires_sonic(*mode),
        PaletteCommandKind::FromTrackContext { kind, .. } => context_visible(state, kind),
        PaletteCommandKind::PlayStation(key) | PaletteCommandKind::BrowseStations { key, .. } => {
            !station_blocked(state, key)
        }
        _ => true,
    }
}

pub fn radio_blocked(state: &AppState) -> bool {
    !enabled(state) && (state.radio.seed.is_some() || state.radio.active_station.as_ref().is_some_and(|s| {
        let title = s.title.to_lowercase();
        matches!(s.source, crate::library::station::RadioSource::Sonic(_))
            || matches!(title.as_str(), "album mix" | "album mix builder" | "sonic radio")
            || matches!(&s.source, crate::library::models::RadioSource::Station(key) if station_blocked(state, key))
    }))
}
/// Filter model lists, not just drawn rows, keeping keyboard/mouse indices aligned.
pub fn reconcile(state: &mut AppState) {
    if enabled(state) {
        return;
    }
    state.sources.nav_tasks.remove("audiomuse");
    state.sources.nav_tasks.remove("audiomuse-info");
    state.sources.nav_tasks.remove(super::audiomuse::SYNC_SLOT);
    state.sources.audiomuse.snapshot = None;
    state.sources.audiomuse.refresh_failed = false;
    if matches!(
        state.sources.nav_collection,
        Some(super::navidrome::commands::CollectionKind::AudioMuse(_))
    ) {
        state.set_browse_category(crate::app::state::BrowseCategory::Library, true);
    }

    if state.dj.active_mode.is_some_and(dj_requires_sonic) {
        state.dj = Default::default();
    }
    // Undo can restore a previous radio snapshot; preserve its songs, not its AI refills.
    if state.playback_mode == PlaybackMode::Radio && radio_blocked(state) {
        state.convert_radio_to_queue("Sonic features disabled");
    }
    state.stations.retain(|s| !station_requires_sonic(s));
    for column in &mut state.station_nav.columns {
        column.stations.retain(|s| !station_requires_sonic(s));
        column.selected_index = column
            .selected_index
            .min(column.stations.len().saturating_sub(1));
    }
    state.track_pane_similar.clear();
    state.track_pane_similar_loading.clear();
    state.track_pane_index = 0;
}
pub fn cancel_pending(state: &mut AppState) {
    state.sources.sonic_tasks.clear();
    state.dj.inserting = false;
    state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
    state.adventure_launcher_request_id = state.adventure_launcher_request_id.wrapping_add(1);
    state.adventure = Default::default();
    state.popups.adventure_launcher = None;
    state.similar = Default::default();
    if state.view == View::Similar {
        state.set_view(View::Browse);
    }
    // Keep current playback and its queued music; stop only future AI additions.
    if state.dj.active_mode.is_some_and(dj_requires_sonic) {
        state.dj = Default::default();
    }
    state.radio_generation = state.radio_generation.wrapping_add(1);
    state.radio_task = None;
    state.station_starting = None;
    state.radio.refill = crate::app::state::RadioRefill::Idle;
    for slot in [
        "dj",
        "remix",
        "similar",
        "similar-pane",
        "adventure",
        "station-children",
    ] {
        state.sources.nav_tasks.remove(slot);
    }
    // Return to the retained root before filtering, so a drilled sonic category
    // cannot leave unlabeled child stations visible. Disabling requires no network.
    state.station_navigation_generation = state.station_navigation_generation.wrapping_add(1);
    state.station_nav.columns.truncate(1);
    state.station_nav.focused_column = 0;
    state.station_nav.loading = false;
    state.stations_loading = false;
    if let Some(root) = state.station_nav.columns.first() {
        state.stations = root.stations.clone();
    }
    reconcile(state);
}
