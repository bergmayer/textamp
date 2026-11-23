//! Desktop destinations compose shared workflows inside the library workspace.
use super::surface;
use crate::app::{Action, AppState, action::*, state::*};

pub const DESCRIPTION_EXAMPLES: [&str; 12] = [
    "Jazz with trumpet",
    "Upbeat summer songs",
    "Soft acoustic guitar",
    "Dark ambient textures",
    "Slow piano with a spacious sound",
    "Fast punk with distorted guitars",
    "Warm soul with expressive vocals",
    "Driving techno with a steady beat",
    "Dreamy electronic music with synthesizers",
    "Orchestral strings with a dramatic mood",
    "Quiet folk with fingerpicked guitar",
    "Heavy metal with powerful drums",
];

pub fn title(state: &AppState) -> Option<&'static str> {
    if state.view != View::Browse || state.browse_category != BrowseCategory::Library {
        return None;
    }
    match state.desktop_library_root {
        LibraryRoot::Mp3Players if crate::app::player_sync::visible(state) => {
            Some(LibraryRoot::Mp3Players.label())
        }
        LibraryRoot::Radio if super::radio::visible(state) => Some("Radio"),
        LibraryRoot::Journey if state.sources.nav_collection.is_none() => Some("Journey"),
        LibraryRoot::DescribeMusic
            if state.sources.nav_collection.is_none()
                || state.sources.nav_collection
                    == Some(
                        crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(
                            crate::audiomuse::Feature::Describe,
                        ),
                    ) =>
        {
            Some("Describe Music")
        }
        _ => None,
    }
}
pub fn owns(state: &AppState) -> bool {
    match state.popups.modal() {
        Modal::ArtistRadioPicker(_) => super::radio::visible(state),
        Modal::AdventureLauncher(launcher) => {
            title(state) == Some("Journey")
                || (super::radio::visible(state) && launcher.purpose != TrackPickerPurpose::Journey)
        }
        Modal::Input(dialog) => {
            state.desktop_library_root == LibraryRoot::DescribeMusic
                && matches!(
                    dialog.action_type,
                    InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe)
                )
        }
        _ => false,
    }
}
pub fn project(state: &AppState) -> Option<surface::Surface> {
    if owns(state) {
        surface::content(state)
    } else if title(state) == Some("Describe Music")
        && state.sources.nav_collection.is_none()
        && matches!(state.popups.modal(), Modal::Closed)
    {
        Some(surface::unavailable_description(state))
    } else {
        None
    }
}
pub fn open(
    root: LibraryRoot,
    state: &mut AppState,
    config: &crate::config::Config,
) -> Vec<Action> {
    let mut actions = surface::navigate(PaletteCommandKind::GotoLibrary, state, config);
    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.sources.nav_collection = None;
    state.desktop_library_root = root;
    state.desktop_player_visible = false;
    match root {
        LibraryRoot::Journey => actions.push(SearchAction::OpenAdventureLauncher.into()),
        LibraryRoot::DescribeMusic => actions.push(
            crate::app::sources::audiomuse::Command::BrowseDescriptions { take_focus: true }.into(),
        ),
        _ => {}
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn station_chooser_reopens_inline_after_selecting_another_station() {
        for multi in [false, true] {
            let mut state = AppState::new();
            state.desktop_frontend = true;
            state.view = View::Browse;
            state.desktop_library_root = LibraryRoot::Radio;
            state.station_nav.columns.push(StationColumn::new(
                None,
                "Stations".into(),
                crate::library::station::standard_stations(
                    "fixture",
                    crate::library::capabilities::NAVIDROME,
                ),
            ));
            let mut config = crate::config::Config::default();
            config.ui.desktop_separate_player = multi;
            let mut sessions = super::super::workflows::Sessions::default();
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            let (tx, _rx) = tokio::sync::mpsc::channel(32);
            for _ in 0..2 {
                sessions
                    .dispatch(
                        SearchAction::OpenArtistRadioPicker.into(),
                        &mut state,
                        &mut audio,
                        &mut config,
                        &tx,
                    )
                    .await
                    .unwrap();
                assert!(owns(&state));
                assert_eq!(project(&state).unwrap().title, "Artist Mix");
                assert!(sessions.models().is_empty());
                for action in super::super::radio::select(&mut state, 0, "fixture/library") {
                    sessions
                        .dispatch(action, &mut state, &mut audio, &mut config, &tx)
                        .await
                        .unwrap();
                }
                assert!(state.popups.artist_radio_picker().is_none());
                assert!(project(&state).is_none());
                assert!(sessions.models().is_empty());
                assert!(state.queue.tracks.is_empty());
            }
        }
    }
    #[tokio::test]
    async fn selecting_seed_stations_opens_inline_inputs_without_playback() {
        let mut state = AppState::new();
        state.desktop_frontend = true;
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::Radio;
        state.station_nav.columns.push(StationColumn::new(
            None,
            "Stations".into(),
            crate::library::station::standard_stations(
                "fixture",
                crate::library::capabilities::NAVIDROME,
            ),
        ));
        let mut config = crate::config::Config::default();
        let mut sessions = super::super::workflows::Sessions::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        for _ in 0..2 {
            for (key, expected) in [("sonic", "Sonic Mix"), ("stayClose", "Stay Close Mix")] {
                for action in super::super::radio::select(&mut state, 0, &format!("fixture/{key}"))
                {
                    sessions
                        .dispatch(action, &mut state, &mut audio, &mut config, &tx)
                        .await
                        .unwrap();
                }
                let pane = project(&state).unwrap();
                assert_eq!(pane.title, expected);
                assert_eq!(pane.fields.len(), 1);
                assert_eq!(pane.fields[0].label, "Start track");
                assert_eq!(pane.endpoints.len(), 1);
                assert!(sessions.models().is_empty());
                assert!(state.queue.tracks.is_empty());
            }
        }
    }
    #[tokio::test]
    async fn journey_is_inline_in_both_window_modes_and_cancels_on_navigation() {
        for multi in [false, true] {
            let mut state = AppState::new();
            state.desktop_frontend = true;
            let mut config = crate::config::Config::default();
            config.ui.desktop_separate_player = multi;
            let mut sessions = super::super::workflows::Sessions::default();
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            let (tx, _rx) = tokio::sync::mpsc::channel(32);
            for action in open(LibraryRoot::Journey, &mut state, &config) {
                sessions
                    .dispatch(action, &mut state, &mut audio, &mut config, &tx)
                    .await
                    .unwrap();
            }
            assert_eq!(title(&state), Some("Journey"));
            assert_eq!(project(&state).unwrap().title, "Journey");
            assert!(sessions.models().is_empty());
            for action in surface::navigate(PaletteCommandKind::GotoLibrary, &mut state, &config) {
                sessions
                    .dispatch(action, &mut state, &mut audio, &mut config, &tx)
                    .await
                    .unwrap();
            }
            assert!(!owns(&state));
            assert!(state.popups.adventure_launcher().is_none());
            assert!(state.queue.tracks.is_empty());
        }
    }
    #[test]
    fn description_prompt_projects_as_workspace_content() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::DescribeMusic;
        state.popups.set_input_dialog(Some(InputDialog {
            title: "Describe music…".into(),
            input: "quiet piano".into(),
            action_type: InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe),
        }));
        assert!(owns(&state));
        let model = project(&state).unwrap();
        assert_eq!(model.fields[0].value, "quiet piano");
        assert!(super::super::surface::project(&state).is_some());
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[tokio::test]
    async fn leaving_description_does_not_publish_its_fallback_page() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::DescribeMusic;
        state.popups.set_input_dialog(Some(InputDialog {
            title: "Describe music…".into(),
            input: "".into(),
            action_type: InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe),
        }));
        let mut config = crate::config::Config::default();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        for action in surface::cancel(&mut state, &config) {
            crate::app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert!(
            project(&state).is_none(),
            "the closed form must not become a Describe Music fallback while the next destination starts"
        );
        assert_eq!(title(&state), None);
        assert!(state.queue.tracks.is_empty());
    }
}
