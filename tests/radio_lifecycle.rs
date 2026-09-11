use std::time::Duration;

use textamp::app::action::{PlaybackAction, RadioAction};
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::event::{Event, RadioEvent};
use textamp::app::handlers::helpers;
use textamp::app::state::{ActiveStation, PlayStatus, PlaybackMode, RadioRefill};
use textamp::app::{Action, AppState};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::models::{RadioSource, Track};

use tokio::sync::mpsc;
mod common;

fn track(key: &str) -> Track {
    serde_json::from_value(serde_json::json!({"ratingKey":key,"title":key})).unwrap()
}

fn station() -> ActiveStation {
    ActiveStation {
        source: RadioSource::Station("nav-radio/randomAlbum".into()),
        title: "Random Album".into(),
    }
}

fn refill(keys: &[&str]) -> Event {
    RadioEvent::RadioTracksLoaded {
        result: Ok(keys.iter().map(|key| track(key)).collect()),
        time_travel_index: None,
    }
    .into()
}

fn loaded() -> Event {
    RadioEvent::StationTracksLoaded {
        station: station(),
        tracks: vec![track("replacement")],
        time_travel_decades: vec![],
        time_travel_index: None,
    }
    .into()
}

fn sonic_loaded(state: &mut AppState, playback_id: u64) -> Event {
    state.station_starting = Some(textamp::app::state::StationStart {
        title: "Sonic Radio".into(),
        continue_playback: Some(playback_id),
    });
    RadioEvent::StationTracksLoaded {
        station: ActiveStation {
            source: RadioSource::Sonic("first".into()),
            title: "Sonic Radio".into(),
        },
        tracks: vec![track("first"), track("similar")],
        time_travel_decades: vec![],
        time_travel_index: None,
    }
    .into()
}

#[tokio::test]
async fn sonic_late_results_never_interrupt_a_new_playback_instance() {
    let mut p = Player::new();
    p.state.playback.request_id = 8;
    p.state.playback.position_ms = 2_000;
    let event = sonic_loaded(&mut p.state, 7);
    assert!(p.reduce(event).is_empty());
    assert_eq!(p.state.radio.tracks[1].rating_key, "last");
    assert_eq!(p.state.playback.position_ms, 2_000);
    assert!(p.state.station_starting.is_none());
}

#[tokio::test]
async fn sonic_result_after_song_ends_starts_matches_without_replaying_seed() {
    let mut p = Player::new();
    p.state.playback.status = PlayStatus::Stopped;
    let id = p.state.playback.request_id;
    let event = sonic_loaded(&mut p.state, id);
    let actions = p.reduce(event);
    assert!(matches!(
        actions.as_slice(),
        [Action::Radio(RadioAction::PlayCurrentRadioTrack)]
    ));
    assert_eq!(p.state.current_track().unwrap().rating_key, "similar");
}

#[tokio::test]
async fn natural_song_end_waits_for_sonic_discovery_instead_of_advancing_old_queue() {
    let mut p = Player::new();
    p.state.playback.playback_started_at = Some(std::time::Instant::now() - Duration::from_secs(5));
    let id = p.state.playback.request_id;
    let result = sonic_loaded(&mut p.state, id);
    assert!(p
        .reduce(textamp::app::event::PlaybackEvent::TrackEnded.into())
        .is_empty());
    assert_eq!(p.state.playback.status, PlayStatus::Stopped);
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
    assert!(p.state.station_starting.is_some());
    let actions = p.reduce(result);
    assert!(matches!(
        actions.as_slice(),
        [Action::Radio(RadioAction::PlayCurrentRadioTrack)]
    ));
    assert_eq!(p.state.current_track().unwrap().rating_key, "similar");
}

#[tokio::test]
async fn pausing_while_sonic_discovery_is_pending_preserves_the_continuation() {
    let mut p = Player::new();
    let id = p.state.playback.request_id;
    let result = sonic_loaded(&mut p.state, id);
    p.dispatch(PlaybackAction::TogglePlayPause).await;
    assert!(p.state.station_starting.is_some());
    assert_eq!(p.state.playback.status, PlayStatus::Paused);
    assert!(p.reduce(result).is_empty());
    assert_eq!(p.state.playback.status, PlayStatus::Paused);
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
    assert_eq!(p.state.radio.tracks[1].rating_key, "similar");
}

struct Player {
    state: AppState,

    audio: AudioPlayer,
    config: Config,
    tx: mpsc::Sender<Event>,
    _rx: mpsc::Receiver<Event>,
}

impl Player {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut state = common::navidrome_state();
        state.set_playback_mode(PlaybackMode::Radio);
        state.radio.tracks = vec![track("first"), track("last")];
        state.radio.track_index = Some(0);
        state.playback.status = PlayStatus::Playing;
        Self {
            state,

            audio: AudioPlayer::new_without_audio(),
            config: Config::default(),
            tx,
            _rx: rx,
        }
    }

    fn reduce(&mut self, event: Event) -> Vec<Action> {
        handle_core_event(event, &mut self.state, &self.tx)
    }

    async fn dispatch(&mut self, action: impl Into<Action>) {
        dispatch_action(
            action.into(),
            &mut self.state,
            &mut self.audio,
            &mut self.config,
            &self.tx,
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn prefetch_finishing_on_last_track_does_not_skip_it() {
    let mut p = Player::new();
    p.state.radio.refill = RadioRefill::Prefetching;
    assert!(helpers::advance_radio(&p.tx, &mut p.state, &mut p.audio));
    assert_eq!(p.state.current_track().unwrap().rating_key, "last");
    // Simulate the new track becoming audible before the outstanding refill finishes.
    p.state.playback.status = PlayStatus::Playing;
    assert!(p.reduce(refill(&["new", "new"])).is_empty());
    assert_eq!(p.state.current_track().unwrap().rating_key, "last");
    assert_eq!(
        p.state.radio.tracks.len(),
        3,
        "deduplicate within the response too"
    );
    assert_eq!(p.state.playback.status, PlayStatus::Playing);
}

#[tokio::test]
async fn next_at_buffer_end_waits_and_advances_once_when_tracks_arrive() {
    let mut p = Player::new();
    p.state.radio.track_index = Some(1);
    p.state.radio.refill = RadioRefill::Prefetching;
    p.state.playback.status = PlayStatus::Stopped;
    p.dispatch(PlaybackAction::Next).await;
    assert_eq!(p.state.radio.refill, RadioRefill::Waiting);
    let actions = p.reduce(refill(&["new"]));
    assert!(matches!(
        actions.as_slice(),
        [Action::Playback(PlaybackAction::Next)]
    ));
    for action in actions {
        p.dispatch(action).await;
    }
    assert_eq!(p.state.current_track().unwrap().rating_key, "new");
    assert!(p.reduce(refill(&["later"])).is_empty());
}

#[tokio::test]
async fn explicit_transport_input_cancels_a_pending_automatic_advance() {
    for action in [
        PlaybackAction::TogglePlayPause,
        PlaybackAction::Previous,
        PlaybackAction::Seek(0),
    ] {
        let mut p = Player::new();
        p.state.radio.refill = RadioRefill::Waiting;
        // Previous at index zero must cancel too, even though there is no previous track.
        p.dispatch(action).await;
        assert_eq!(p.state.radio.refill, RadioRefill::Prefetching);
        assert!(p.reduce(refill(&["new"])).is_empty());
        assert_eq!(p.state.current_track().unwrap().rating_key, "first");
    }
}

#[test]
fn empty_and_duplicate_only_refills_finish_without_an_automatic_retry_loop() {
    for keys in [vec![], vec!["first", "last", "last"]] {
        let mut p = Player::new();
        p.state.radio.refill = RadioRefill::Waiting;
        assert!(p.reduce(refill(&keys)).is_empty());
        assert_eq!(p.state.radio.refill, RadioRefill::Idle);
        assert_eq!(p.state.radio.tracks.len(), 2);
        assert!(p
            .state
            .notifications
            .last_error
            .as_ref()
            .unwrap()
            .contains("no new tracks"));
    }
}

#[tokio::test]
async fn stop_rejects_late_station_and_refill_completions() {
    let mut p = Player::new();
    p.state.station_starting = Some(textamp::app::state::StationStart {
        title: "Candidate".into(),
        continue_playback: None,
    });
    p.state.radio.refill = RadioRefill::Waiting;
    let generation = p.state.radio_generation;
    p.dispatch(PlaybackAction::Stop).await;
    for event in [loaded(), refill(&["new"])] {
        assert!(p
            .reduce(Event::RadioResult {
                generation,
                navigation_generation: None,
                event: Box::new(event),
            })
            .is_empty());
    }
    assert!(p.state.station_starting.is_none());
    assert_eq!(p.state.playback.status, PlayStatus::Stopped);
    assert_eq!(p.state.radio.tracks.len(), 2);
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
}

#[tokio::test]
async fn only_latest_station_can_replace_playback_and_empty_results_preserve_it() {
    let mut p = Player::new();
    // Invalid addresses finish asynchronously without issuing HTTP requests.
    p.dispatch(RadioAction::PlayStation("invalid-a".into()))
        .await;
    let old = p.state.radio_generation;
    p.dispatch(RadioAction::StartArtistRadio {
        key: "artist".into(),
        title: "Artist".into(),
    })
    .await;
    assert!(p
        .reduce(Event::RadioResult {
            generation: old,
            navigation_generation: None,
            event: Box::new(loaded()),
        })
        .is_empty());
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
    let mut event = loaded();
    if let Event::Radio(RadioEvent::StationTracksLoaded { tracks, .. }) = &mut event {
        tracks.clear();
    }
    assert!(p.reduce(event).is_empty());
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
    assert_eq!(p.state.playback.status, PlayStatus::Playing);
    assert!(matches!(
        p.reduce(loaded()).as_slice(),
        [Action::Radio(RadioAction::PlayCurrentRadioTrack)]
    ));
    assert_eq!(p.state.current_track().unwrap().rating_key, "replacement");
}

#[tokio::test]
async fn pause_cancels_station_start_without_stopping_current_music_context() {
    let mut p = Player::new();
    p.state.station_starting = Some(textamp::app::state::StationStart {
        title: "Candidate".into(),
        continue_playback: None,
    });
    let generation = p.state.radio_generation;
    p.dispatch(PlaybackAction::TogglePlayPause).await;
    assert_eq!(p.state.playback.status, PlayStatus::Paused);
    assert!(p.state.station_starting.is_none());
    assert!(p
        .reduce(Event::RadioResult {
            generation,
            navigation_generation: None,
            event: Box::new(loaded()),
        })
        .is_empty());
    assert_eq!(p.state.current_track().unwrap().rating_key, "first");
}

#[tokio::test]
async fn stop_is_available_in_the_palette_and_cancels_station_loading() {
    let mut p = Player::new();
    p.state.station_starting = Some(textamp::app::state::StationStart {
        title: "Candidate".into(),
        continue_playback: None,
    });
    let entry = textamp::app::command_palette::materialize_entries(&p.state)
        .into_iter()
        .find(|entry| entry.label == "Stop Playback")
        .unwrap();
    let actions = textamp::app::command_palette::run(entry.command, &mut p.state);
    for action in actions {
        p.dispatch(action).await;
    }
    assert!(p.state.station_starting.is_none());
    assert_eq!(p.state.playback.status, PlayStatus::Stopped);
}

#[test]
fn library_changes_clear_loading_for_requests_whose_results_will_be_discarded() {
    let mut p = Player::new();
    p.state.station_starting = Some(textamp::app::state::StationStart {
        title: "Candidate".into(),
        continue_playback: None,
    });
    p.state.station_nav.loading = true;
    p.state.radio.refill = RadioRefill::Waiting;
    p.state.advance_library_generation();
    assert!(p.state.station_starting.is_none());
    assert!(!p.state.station_nav.loading);
    assert_eq!(p.state.radio.refill, RadioRefill::Idle);
}
