use crate::{
    app::{
        Action, AppState, Event,
        action::PlaybackAction,
        handlers::dispatch_playback,
        state::{PlayStatus, PlaybackMode},
    },
    audio::AudioPlayer,
    library::models::Track,
};
use tokio::sync::mpsc;

fn state() -> AppState {
    let mut state = AppState::new();
    state.playback_mode = PlaybackMode::Queue;
    state.queue.tracks = vec![Track {
        rating_key: "seed".into(),
        title: "Starting song".into(),
        original_title: Some("Starting artist".into()),
        duration: Some(60_000),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    state.playback.request_id = 12;
    state
}
fn dispatch(state: &mut AppState, action: PlaybackAction) -> Vec<Action> {
    let (tx, _) = mpsc::channel::<Event>(8);
    dispatch_playback::dispatch(&tx, action, state, &mut AudioPlayer::new_without_audio()).unwrap()
}

#[test]
fn continuous_modes_do_not_edit_the_queue_and_mute_retains_restore_volume() {
    let mut state = state();
    dispatch(&mut state, PlaybackAction::ToggleAutoMix);
    assert!(state.playback.auto_mix);
    assert_eq!(state.queue.tracks.len(), 1);
    state.playback.volume = 0.63;
    dispatch(&mut state, PlaybackAction::ToggleMute);
    assert!(state.playback.muted);
    assert_eq!(state.playback.volume, 0.63);
    dispatch(&mut state, PlaybackAction::VolumeUp);
    assert!(!state.playback.muted);
    assert!((state.playback.volume - 0.68).abs() < 0.001);
}

#[test]
fn lyrics_fallback_remembers_the_requested_song_and_embedded_lyrics_are_displayed() {
    let mut state = state();
    dispatch(&mut state, PlaybackAction::ShowLyrics);
    let view = crate::app::lyrics::view(&state).unwrap();
    assert_eq!(view.message.as_deref(), Some("No lyrics found"));
    let actions = crate::app::lyrics::Control::Search.actions(&state);
    let [Action::System(crate::app::action::SystemAction::OpenBrowser(url))] = actions.as_slice()
    else {
        panic!("Missing explicit browser search")
    };
    assert_eq!(
        reqwest::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "q")
            .unwrap()
            .1,
        "Starting artist Starting song lyrics"
    );
    state.queue.tracks[0].lyrics = Some("First line\nSecond line".into());
    dispatch(&mut state, PlaybackAction::ShowLyrics);
    let view = crate::app::lyrics::view(&state).unwrap();
    assert_eq!(
        view.lines
            .iter()
            .map(|line| line.value.as_str())
            .collect::<Vec<_>>(),
        ["First line", "Second line"]
    );
    assert!(view.message.is_none());
    assert!(!view.synced);
}

#[test]
fn automix_is_requested_once_near_the_end_and_never_for_repeat_track() {
    let mut state = state();
    state.queue.tracks.push(Track {
        rating_key: "next".into(),
        ..Default::default()
    });
    state.playback.auto_mix = true;
    state.playback.status = PlayStatus::Playing;
    state.playback.duration_ms = 60_000;
    state.playback.position_ms = 56_500;
    let mut audio = AudioPlayer::new_without_audio();
    let mut clock = crate::app::runtime::PlaybackClock::default();
    assert!(
        clock
            .poll(&mut state, &mut audio)
            .iter()
            .any(|event| matches!(
                event,
                Event::Effect(Action::Playback(PlaybackAction::MixNext {
                    playback_id: 12
                }))
            ))
    );
    assert!(
        !clock
            .poll(&mut state, &mut audio)
            .iter()
            .any(|event| matches!(
                event,
                Event::Effect(Action::Playback(PlaybackAction::MixNext { .. }))
            ))
    );
    state.queue.repeat = crate::app::state::RepeatMode::Track;
    assert!(dispatch(&mut state, PlaybackAction::MixNext { playback_id: 12 }).is_empty());
}

#[test]
fn stopping_station_preserves_audio_and_queue_and_invalidates_refills() {
    let mut state = state();
    let tracks = state.queue.tracks.clone();
    state.playback_mode = PlaybackMode::Radio;
    state.radio.tracks = tracks.clone();
    state.radio.track_index = Some(0);
    state.radio.refill = crate::app::state::RadioRefill::Prefetching;
    state.radio.active_station = Some(crate::app::state::ActiveStation {
        source: crate::library::models::RadioSource::Station("test".into()),
        title: "Station".into(),
    });
    state.playback.status = PlayStatus::Playing;
    let generation = state.radio_generation;
    let playback_id = state.playback.request_id;
    let (tx, _) = mpsc::channel(8);
    let mut audio = AudioPlayer::new_without_audio();
    crate::app::handlers::dispatch_radio::dispatch(
        &tx,
        crate::app::action::RadioAction::StopStation,
        &mut state,
        &mut audio,
    )
    .unwrap();
    assert_eq!(state.playback_mode, PlaybackMode::Queue);
    assert_eq!(
        state
            .queue
            .tracks
            .iter()
            .map(|t| &t.rating_key)
            .collect::<Vec<_>>(),
        tracks.iter().map(|t| &t.rating_key).collect::<Vec<_>>()
    );
    assert_eq!(state.queue.index, Some(0));
    assert_eq!(state.playback.request_id, playback_id);
    assert_eq!(state.playback.status, PlayStatus::Playing);
    assert_ne!(state.radio_generation, generation);
    assert!(state.radio.active_station.is_none());
    assert!(state.radio_task.is_none());
}
