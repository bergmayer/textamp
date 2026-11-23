use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textamp::{
    app::{
        Action, AppState, Event,
        action::QueueAction,
        handlers::{dispatch_queue, key_input},
    },
    audio::AudioPlayer,
    config::Config,
    library::models::Track,
    services::playlist_export::Format,
};

#[test]
fn playlist_selection_recovers_a_missing_root_and_retains_loaded_tracks() {
    let mut state = AppState::new();
    state.library.playlists = std::sync::Arc::new(
        ["A", "B"]
            .into_iter()
            .map(|key| {
                serde_json::from_value(
                    serde_json::json!({"ratingKey":key,"key":key,"title":key,"type":"audio"}),
                )
                .unwrap()
            })
            .collect(),
    );
    assert!(state.select_playlist("B"));
    assert_eq!(
        state.playlist_nav.columns[0].selected_item().unwrap().key(),
        "B"
    );
    state
        .playlist_nav
        .columns
        .push(textamp::app::state::BrowseColumn::new("B tracks", vec![]));
    assert!(state.select_playlist("B"));
    assert_eq!(state.playlist_nav.columns.len(), 2);
    assert!(state.select_playlist("A"));
    assert_eq!(state.playlist_nav.columns.len(), 1);
    assert!(!state.select_playlist("missing"));
    assert_eq!(
        state.playlist_nav.columns[0].selected_item().unwrap().key(),
        "A"
    );
    state.playlist_nav.columns[0].items.clear();
    assert!(state.select_playlist("B"));
    assert_eq!(
        state.playlist_nav.columns[0].selected_item().unwrap().key(),
        "B"
    );
}

#[tokio::test]
async fn terminal_export_uses_the_captured_queue_and_rejects_an_old_library() {
    for format in [Format::Csv, Format::M3u8] {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary
            .path()
            .join(format!("export.{}", format.extension()));
        let mut state = AppState::new();
        state.queue.tracks = vec![Track::from_folder("fixture", "/Music/Original.flac")];
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut audio = AudioPlayer::new_without_audio();
        dispatch_queue::dispatch(
            &tx,
            QueueAction::PromptExportPlaylist {
                format,
                queue: true,
            },
            &mut state,
            &mut audio,
        )
        .unwrap();
        state.popups.input_dialog_mut().unwrap().input =
            destination.to_string_lossy().as_ref().into();
        state.queue.tracks = vec![Track::from_folder("fixture", "/Music/Changed.flac")];
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        let [Action::Queue(export)] = actions.as_slice() else {
            panic!("export path must submit a shared action")
        };
        dispatch_queue::dispatch(&tx, export.clone(), &mut state, &mut audio).unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::Effect(Action::Queue(result)) = event else {
            panic!("expected export completion")
        };
        dispatch_queue::dispatch(&tx, result, &mut state, &mut audio).unwrap();
        let contents = std::fs::read_to_string(destination).unwrap();
        assert!(contents.contains("Original.flac"));
        assert!(!contents.contains("Changed.flac"));
        assert_eq!(state.queue.tracks[0].title, "Changed.flac");
        let stale = temporary.path().join("stale.csv");
        dispatch_queue::dispatch(
            &tx,
            QueueAction::ExportPlaylist {
                generation: state.library_generation.wrapping_add(1),
                tracks: state.queue.tracks.clone(),
                format: Format::Csv,
                path: stale.clone(),
            },
            &mut state,
            &mut audio,
        )
        .unwrap();
        assert!(!stale.exists());
    }
}

#[test]
fn playlist_export_rejects_a_retained_column_from_another_playlist() {
    use textamp::app::state::{BrowseColumn, BrowseItem, PlayAllRow};
    let mut state = AppState::new();
    state.playlist_nav.columns = vec![
        BrowseColumn::new(
            "playlists",
            vec![BrowseItem::Playlist {
                key: "new".into(),
                title: "New Playlist".into(),
                track_count: None,
            }],
        ),
        BrowseColumn::new_with_tracks(
            "Old Playlist",
            vec![],
            vec![Track::from_folder("fixture", "/Music/old.flac")],
        ),
    ];
    state.playlist_nav.columns[1].play_all_row = Some(PlayAllRow::Playlist {
        rating_key: "old".into(),
        title: "Old Playlist".into(),
    });
    let (tx, _) = tokio::sync::mpsc::channel(8);
    dispatch_queue::dispatch(
        &tx,
        QueueAction::PromptExportPlaylist {
            format: Format::Csv,
            queue: false,
        },
        &mut state,
        &mut AudioPlayer::new_without_audio(),
    )
    .unwrap();
    assert!(state.popups.input_dialog().is_none());
    assert_eq!(
        state.notifications.last_error.as_deref(),
        Some("Open a complete playlist before exporting")
    );
}
