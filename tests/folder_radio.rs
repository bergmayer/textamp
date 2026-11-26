use textamp::library::{radio::FolderRadio, track::Track};

#[test]
fn folder_station_counts_album_credits_and_excludes_previous_albums() {
    let mut radio = FolderRadio::default();
    for i in 0..20 {
        let mut track = Track::from_folder("fixture", &format!("dir-{i}/song.flac"));
        track.grandparent_title = Some(format!("artist-{}", i.min(18)));
        radio.observe(track);
    }
    assert!(!radio.available());
    let mut last = Track::from_folder("fixture", "last/song.flac");
    last.grandparent_title = Some("twentieth".into());
    radio.observe(last);
    let selected = radio.select(&Default::default(), None).unwrap();
    assert_eq!(selected.len(), 21);
    let excluded = selected
        .iter()
        .filter_map(textamp::library::radio::album_key)
        .collect();
    assert!(radio.select(&excluded, None).unwrap().is_empty());
}

#[tokio::test]
async fn prepared_metadata_updates_radio_even_when_audio_output_is_unavailable() {
    use textamp::app::sources::{self, FileAction};
    use textamp::app::state::PlaybackMode;
    use textamp::library::{MediaFile, track::Track};
    let mut state = textamp::app::AppState::new();
    state.queue.tracks = vec![Track::from_folder("fixture", "inactive.flac")];
    state.queue.index = Some(0);
    state.radio.tracks = vec![Track::from_folder("fixture", "Album/song.flac")];
    state.radio.track_index = Some(0);
    state.set_playback_mode(PlaybackMode::Radio);
    let mut track = state.current_track().unwrap().clone();
    track.title = "Embedded title".into();
    track.duration = Some(123_000);
    let action = FileAction::Prepared {
        tagged_album: false,
        generation: state.library_generation,
        id: state.playback.preparation_id,
        track: Box::new(track),
        artwork: Some(vec![1]),
        result: Ok(MediaFile::local("/unused-without-audio".into())),
    };
    let (tx, _) = tokio::sync::mpsc::channel(8);
    sources::dispatch(
        action.into(),
        &mut state,
        &mut textamp::audio::AudioPlayer::new_without_audio(),
        &mut textamp::config::Config::default(),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(state.current_track().unwrap().title, "Embedded title");
    assert_eq!(state.queue.tracks[0].title, "inactive.flac");
    assert_eq!(state.playback.duration_ms, 123_000);
    assert_eq!(state.artwork.current_data, Some(vec![1]));
}

#[tokio::test]
async fn shared_station_lifecycle_commits_folder_tracks_and_cancels_obsolete_work() {
    use std::time::Duration;
    use textamp::app::action::{PlaybackAction, RadioAction};
    use textamp::app::dispatch::{dispatch_action, handle_core_event};
    use textamp::app::state::{PlaybackMode, RadioRefill};
    use textamp::app::{Action, AppState, Event};
    let mut state = AppState::new();
    state.sources.active = textamp::app::sources::ActiveSource::Folder("fixture".into());
    for i in 0..140 {
        let mut track = Track::from_folder("fixture", &format!("album-{i}/song.flac"));
        track.grandparent_title = Some(format!("artist-{i}"));
        state.library.folder_radio.observe(track);
    }
    textamp::app::sources::radio::load_folder_stations(&mut state);

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let mut config = textamp::config::Config::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(16);
    let action = RadioAction::PlayStation("folder-radio/randomAlbum".into());
    dispatch_action(
        action.clone().into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.radio_task.is_some());
    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let follow = handle_core_event(result, &mut state, &tx);
    assert!(matches!(
        follow.as_slice(),
        [Action::Radio(RadioAction::PlayCurrentRadioTrack)]
    ));
    assert_eq!(state.playback_mode, PlaybackMode::Radio);
    assert_eq!(state.radio.tracks.len(), 100);
    assert!(state.radio_task.is_none());
    textamp::app::sources::radio::refill(&tx, &mut state);
    assert_eq!(state.radio.refill, RadioRefill::Prefetching);
    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(handle_core_event(result, &mut state, &tx).is_empty());
    assert_eq!(state.radio.tracks.len(), 140);
    assert_eq!(state.radio.track_index, Some(0));
    dispatch_action(
        action.clone().into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    let stale = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch_action(
        PlaybackAction::Stop.into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await
    .unwrap();
    assert!(state.radio_task.is_none());
    assert!(handle_core_event(stale, &mut state, &tx).is_empty());
    assert!(state.station_starting.is_none());
    dispatch_action(action.into(), &mut state, &mut audio, &mut config, &tx)
        .await
        .unwrap();
    state.advance_library_generation();
    assert!(state.radio_task.is_none());
}
