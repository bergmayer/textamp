//! Queue generation policy and controls shared by every presentation.
use super::{AppState, Event, state::*};
use crate::library::station::RadioSource;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Manual,
    Related,
    Station,
}
pub fn current(state: &AppState) -> Mode {
    if state.playback_mode != PlaybackMode::Radio {
        return Mode::Manual;
    }
    match state.radio.active_station.as_ref().map(|s| &s.source) {
        Some(RadioSource::Related(_)) => Mode::Related,
        _ => Mode::Station,
    }
}
pub fn label(state: &AppState) -> String {
    match current(state) {
        Mode::Manual => match state.queue.repeat {
            RepeatMode::Off => "Manual Queue",
            RepeatMode::Track => "Repeat Track",
            RepeatMode::Queue => "Repeat Queue",
        }
        .into(),
        Mode::Related => "Continue with Related Tracks".into(),
        Mode::Station => state.radio.title(),
    }
}
#[derive(Clone)]
pub struct Choice {
    pub label: String,
    pub group: &'static str,
    pub command: PaletteCommandKind,
    pub selected: bool,
    pub disabled: Option<&'static str>,
}
pub fn choices(state: &AppState) -> Vec<Choice> {
    let mode = current(state);
    let mut choices = vec![Choice {
        label: "Manual Queue".into(),
        group: "Queue type",
        command: PaletteCommandKind::StopRadio,
        selected: mode == Mode::Manual && state.station_starting.is_none(),
        disabled: None,
    }];
    let mut seen = std::collections::HashSet::new();
    for station in state.station_nav.columns.iter().flat_map(|c| c.stations()) {
        if !seen.insert(&station.key) {
            continue;
        }
        let command = if station.requires_input() {
            PaletteCommandKind::ConfigureStation(station.key.clone())
        } else {
            PaletteCommandKind::PlayStation(station.key.clone())
        };
        choices.push(Choice {
            label: station.title.clone(),
            group: "Queue type",
            selected: mode == Mode::Station
                && state
                    .radio
                    .active_station
                    .as_ref()
                    .is_some_and(|s| s.source.station_key() == Some(station.key.as_str())),
            disabled: super::command_catalog::disabled_reason(state, &command),
            command,
        });
    }
    if state
        .sources
        .active
        .capabilities()
        .supports(crate::library::capabilities::Feature::ArtistRadio)
    {
        choices.push(Choice {
            label: "Artist Mix…".into(),
            group: "Queue type",
            command: PaletteCommandKind::ArtistRadio,
            selected: false,
            disabled: super::command_catalog::disabled_reason(
                state,
                &PaletteCommandKind::ArtistRadio,
            ),
        });
    }
    if mode == Mode::Station && !choices.iter().any(|choice| choice.selected) {
        choices.insert(
            1,
            Choice {
                label: label(state),
                group: "Queue type",
                command: PaletteCommandKind::StopRadio,
                selected: true,
                disabled: None,
            },
        );
    }
    choices.extend(
        [
            (RepeatMode::Off, "Repeat Off"),
            (RepeatMode::Track, "Repeat Track"),
            (RepeatMode::Queue, "Repeat Queue"),
        ]
        .into_iter()
        .map(|(repeat, label)| Choice {
            label: label.into(),
            group: "Playback",
            command: PaletteCommandKind::SetQueueRepeat(repeat),
            selected: state.queue.repeat == repeat,
            disabled: (repeat != RepeatMode::Off && state.playback_tracks().is_empty())
                .then_some("The queue is empty"),
        }),
    );
    choices.push(Choice {
        label: "Continue with Related Tracks".into(),
        group: "Playback",
        command: PaletteCommandKind::ToggleAutoPlay,
        selected: mode == Mode::Related,
        disabled: if state.playback_tracks().is_empty() {
            Some("Add tracks to the queue first")
        } else if state.downloads.is_offline() {
            Some("Unavailable offline")
        } else {
            None
        },
    });
    choices
}
/// Preserve the audible occurrence and all existing queue entries; discovery appends only and stays close to the final track.
pub fn continue_related(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if state.downloads.is_offline() {
        state.set_status("Unavailable offline".into());
        return;
    }
    let Some(seed) = state.playback_tracks().last().cloned() else {
        state.set_status("Add tracks to the queue first".into());
        return;
    };
    let index = if state.playback_mode == PlaybackMode::Radio {
        state.radio.track_index
    } else {
        state.queue.index
    };
    let tracks = state.playback_tracks().to_vec();
    state.set_playback_mode(PlaybackMode::Radio);
    state.radio.clear();
    state
        .radio
        .introduced_albums
        .extend(tracks.iter().filter_map(crate::library::radio::album_key));
    state.radio.tracks = tracks;
    state.radio.track_index = index;
    state.radio.active_station = Some(ActiveStation {
        source: RadioSource::Related(seed.rating_key),
        title: "Continue with Related Tracks".into(),
    });
    prefetch(state, tx);
}
/// Refill at the second-to-last track, including short queues and direct track selection.
pub fn prefetch(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    if state.playback_mode == PlaybackMode::Radio
        && state
            .radio
            .track_index
            .is_some_and(|i| i.saturating_add(2) >= state.radio.tracks.len())
    {
        super::sources::radio::refill(tx, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        action::*,
        dispatch::{dispatch_action, handle_core_event},
    };
    fn queue() -> AppState {
        let mut state = AppState::new();
        state.playback_mode = PlaybackMode::Queue;
        state.queue.tracks = (0..5)
            .map(|i| crate::library::models::Track {
                rating_key: format!("track-{i}"),
                ..Default::default()
            })
            .collect();
        state.queue.index = Some(0);
        state.playback.status = PlayStatus::Paused;
        state.playback.request_id = 17;
        state.playback.position_ms = 1234;
        state
    }
    #[tokio::test]
    async fn continuation_preserves_audio_and_edits_and_refills_only_at_the_penultimate_track() {
        let mut state = queue();
        let (tx, _rx) = mpsc::channel(16);
        continue_related(&mut state, &tx);
        assert_eq!(current(&state), Mode::Related);
        assert!(
            matches!(&state.radio.active_station.as_ref().unwrap().source,RadioSource::Related(key) if key=="track-4")
        );
        assert_eq!(state.playback.request_id, 17);
        assert_eq!(state.playback.position_ms, 1234);
        assert_eq!(state.playback.status, PlayStatus::Paused);
        assert_eq!(state.radio.tracks.len(), 5);
        assert_eq!(state.radio.refill, RadioRefill::Idle);
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let mut config = crate::config::Config::default();
        dispatch_action(
            QueueAction::ReorderSelection { from: 4, to: 2 }.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(current(&state), Mode::Related);
        assert_eq!(state.radio.tracks[2].rating_key, "track-4");
        assert_eq!(state.current_track().unwrap().rating_key, "track-0");
        assert_eq!(state.playback.request_id, 17);
        state.radio.track_index = Some(2);
        prefetch(&mut state, &tx);
        assert_eq!(state.radio.refill, RadioRefill::Idle);
        state.radio.track_index = Some(3);
        prefetch(&mut state, &tx);
        assert_eq!(state.radio.refill, RadioRefill::Prefetching);
        let generation = state.radio_generation;
        dispatch_action(
            RadioAction::StopStation.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(current(&state), Mode::Manual);
        assert_eq!(state.queue.tracks.len(), 5);
        assert_eq!(state.playback.status, PlayStatus::Paused);
        assert_eq!(state.playback.request_id, 17);
        let late = crate::app::event::RadioEvent::RadioTracksLoaded {
            result: Ok(vec![crate::library::models::Track {
                rating_key: "late".into(),
                ..Default::default()
            }]),
            time_travel_index: None,
        };
        assert!(
            handle_core_event(
                Event::RadioResult {
                    generation,
                    navigation_generation: None,
                    event: Box::new(late.into())
                },
                &mut state,
                &tx
            )
            .is_empty()
        );
        assert_eq!(state.queue.tracks.len(), 5);
    }
    #[tokio::test]
    async fn editing_a_mix_keeps_the_inactive_queue_and_pending_station_switch() {
        let mut state = queue();
        let (tx, _rx) = mpsc::channel(16);
        continue_related(&mut state, &tx);
        state.station_starting = Some(StationStart {
            title: "Next station".into(),
            continue_playback: None,
        });
        let generation = state.radio_generation;
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let mut config = crate::config::Config::default();
        for (action, remaining) in [
            (QueueAction::RemoveFromQueue(1), 4),
            (QueueAction::UndoLastRemix, 5),
        ] {
            dispatch_action(action.into(), &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
            assert_eq!(state.radio.tracks.len(), remaining);
            assert_eq!(state.queue.tracks.len(), 5);
            assert_eq!(state.playback.request_id, 17);
            assert_eq!(current(&state), Mode::Related);
            assert_eq!(state.radio_generation, generation);
            assert_eq!(
                state.station_starting.as_ref().unwrap().title,
                "Next station"
            );
        }
    }
    #[tokio::test]
    async fn shuffle_after_switching_to_radio_keeps_the_station_tracks() {
        let mut state = queue();
        let (tx, _rx) = mpsc::channel(16);
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let mut config = crate::config::Config::default();
        dispatch_action(
            QueueAction::ToggleQueueShuffle.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        continue_related(&mut state, &tx);
        for track in &mut state.radio.tracks {
            track.rating_key.insert_str(0, "station-");
        }
        let playing = state.current_track().unwrap().rating_key.clone();
        dispatch_action(
            QueueAction::ToggleQueueShuffle.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(current(&state), Mode::Related);
        assert_eq!(state.current_track().unwrap().rating_key, playing);
        assert_eq!(state.radio.tracks.len(), 5);
        assert!(
            state
                .radio
                .tracks
                .iter()
                .all(|t| t.rating_key.starts_with("station-"))
        );
    }

    #[test]
    fn continuation_follows_repeat_and_retains_an_unlisted_active_station() {
        let mut state = queue();
        state.set_playback_mode(PlaybackMode::Radio);
        state.station_nav.columns.clear();
        state.radio.active_station = Some(ActiveStation {
            source: RadioSource::Station("unlisted".into()),
            title: "Active station".into(),
        });
        let options = choices(&state);
        assert_eq!(options[0].label, "Manual Queue");
        assert_eq!(options[1].label, "Active station");
        assert!(options[1].selected);
        assert_eq!(
            options
                .iter()
                .filter(|choice| choice.group == "Playback")
                .map(|choice| choice.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Repeat Off",
                "Repeat Track",
                "Repeat Queue",
                "Continue with Related Tracks"
            ]
        );
        assert!(matches!(
            options.last().unwrap().command,
            PaletteCommandKind::ToggleAutoPlay
        ));
    }
    #[test]
    fn random_album_radio_is_distinct_from_random_album_navigation() {
        let play = PaletteCommandKind::PlayStation("nav-radio/randomAlbum".into());
        assert_eq!(
            super::super::command_catalog::label(&play, "Play Random Albums"),
            "Play Random Albums"
        );
        assert!(!super::super::command_catalog::visible(
            &play,
            "Play Random Albums",
            &[],
            false
        ));
        assert!(super::super::command_catalog::visible(
            &play,
            "Play Random Albums",
            &[],
            true
        ));
        assert!(super::super::command_catalog::visible(
            &play,
            "Play Random Albums",
            &["Queue".into(), "Queue Type".into()],
            false
        ));
        assert_eq!(
            super::super::command_catalog::label(&PaletteCommandKind::GoToRandomAlbum, ""),
            "Go to Random Album"
        );
        assert!(
            !super::super::command_palette::global_entries()
                .iter()
                .any(|entry| entry.label == "Play Random Album")
        );
    }
}
