use textamp::{
    app::{
        AppState,
        action::{BrowseAction, NavigationAction, SettingsAction},
        command_palette,
        dispatch::dispatch_action,
        state::{LibraryRoot, PaletteCommandKind},
    },
    audio::AudioPlayer,
    config::Config,
    library::models::Album,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn singles_threshold_and_selection_are_shared_and_do_not_start_playback() {
    for desktop in [false, true] {
        let mut state = AppState::new();
        state.desktop_frontend = desktop;
        state.library.albums = std::sync::Arc::new(
            [Some(1), Some(2), Some(3), Some(0), None]
                .into_iter()
                .enumerate()
                .map(|(index, count)| Album {
                    rating_key: index.to_string(),
                    title: format!("Release {index}"),
                    leaf_count: count,
                    ..Default::default()
                })
                .collect(),
        );
        let mut config = Config::default();
        let mut audio = AudioPlayer::new_without_audio();
        let (tx, _rx) = mpsc::channel(16);
        command_palette::open_with_query(&mut state, "Singles");
        let selected = state.palette.matches[0];
        assert!(matches!(
            state.palette.entries[selected].command,
            PaletteCommandKind::GotoSingles
        ));
        let old_request = state.artist_nav_request_id;
        dispatch_action(
            BrowseAction::OpenSingles.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(state.desktop_library_root, LibraryRoot::Singles);
        assert!(state.artist_nav_request_id > old_request);
        assert!(state.queue.tracks.is_empty());
        assert_eq!(state.artist_nav.columns.len(), 1);
        let keys = |state: &AppState| {
            state.artist_nav.columns[0]
                .items
                .iter()
                .map(|item| item.key().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&state), ["0", "1"]);
        state.artist_nav.columns[0].selected_index = 1;
        dispatch_action(
            SettingsAction::SetSinglesTrackLimit(3).into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(keys(&state), ["0", "1", "2"]);
        assert_eq!(
            state.artist_nav.columns[0].selected_item().unwrap().key(),
            "1"
        );
        assert_eq!(config.ui.singles_track_limit, 3);
        dispatch_action(
            SettingsAction::SetSinglesTrackLimit(0).into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(state.singles_track_limit, 3);
        assert_eq!(config.ui.singles_track_limit, 3);
        let request = state.artist_nav_request_id;
        state.artist_nav.loading = true;
        dispatch_action(
            SettingsAction::SetSinglesTrackLimit(1).into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(keys(&state), ["0"]);
        assert!(state.artist_nav_request_id > request);
        assert!(!state.artist_nav.loading);
        dispatch_action(
            NavigationAction::set_category(textamp::app::BrowseCategory::Library).into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(state.desktop_library_root, LibraryRoot::Artists);
        assert_eq!(state.artist_nav.columns[0].title, "artists");
    }
}

#[test]
fn singles_settings_keep_the_two_track_default_for_existing_configurations() {
    let config = Config::default();
    assert_eq!(config.ui.singles_track_limit, 2);
    let mut value = serde_json::to_value(&config.ui).unwrap();
    value.as_object_mut().unwrap().remove("singles_track_limit");
    let restored: textamp::config::UiConfig = serde_json::from_value(value).unwrap();
    assert_eq!(restored.singles_track_limit, 2);
}
