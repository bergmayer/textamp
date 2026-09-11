use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textamp::app::action::{QueueAction, RadioAction, SearchAction};
use textamp::app::command_palette::{self};
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::handlers::{helpers, key_input};
use textamp::app::state::{
    BrowseCategory, BrowseColumn, BrowseItem, PaletteCommandKind, PlaybackMode, View,
};
use textamp::app::{Action, AppState};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use textamp::library::models::{Station, Track};

use tokio::sync::mpsc;
mod common;

fn track(key: &str) -> Track {
    serde_json::from_value(serde_json::json!({"ratingKey":key,"title":key})).unwrap()
}

fn station(key: &str, title: &str, kind: &str) -> Station {
    serde_json::from_value(serde_json::json!({"key":key,"title":title,"type":kind})).unwrap()
}

fn column(keys: &[&str]) -> BrowseColumn {
    let tracks: Vec<_> = keys.iter().map(|key| track(key)).collect();
    BrowseColumn::new_with_tracks("tracks", BrowseItem::from_tracks(&tracks), tracks)
}

fn run_label(state: &mut AppState, label: &str) -> Vec<Action> {
    let entry = command_palette::materialize_entries(state)
        .into_iter()
        .find(|entry| entry.label == label)
        .unwrap_or_else(|| panic!("Missing command {label}"));
    command_palette::run(entry.command, state)
}

#[test]
fn track_context_and_palette_start_sonic_radio_from_the_focused_track() {
    let mut state = common::navidrome_state();
    state.view = View::Queue;
    state.queue.tracks = vec![track("playing"), track("selected")];
    state.queue.index = Some(0);
    state.list_state.queue_index = 1;
    let entries =
        textamp::services::track_context::track_context_entries(&state, &track("selected"), false);
    assert!(entries.iter().any(|e| matches!(
        e.kind,
        textamp::services::track_context::ContextKind::SonicRadio
    )));
    let actions = run_label(&mut state, "Start Sonic Radio based on track");
    assert!(
        matches!(actions.as_slice(), [Action::Radio(RadioAction::StartSonicRadio(track))] if track.rating_key == "selected")
    );
    state.sources.active = textamp::app::sources::ActiveSource::Folder("local".into());
    assert!(!command_palette::materialize_entries(&state)
        .iter()
        .any(|e| e.label == "Start Sonic Radio based on track"));
}

fn keys(tracks: &[Track]) -> Vec<&str> {
    tracks
        .iter()
        .map(|track| track.rating_key.as_str())
        .collect()
}

#[test]
fn colon_q_enter_quits_from_browse_and_playback_views() {
    for view in [View::Browse, View::Queue, View::NowPlaying] {
        for query in ['q', 'Q'] {
            let mut state = AppState::new();
            state.view = view;
            let config = Config::default();
            for code in [KeyCode::Char(':'), KeyCode::Char(query)] {
                assert!(key_input::handle_key(
                    KeyEvent::new(code, KeyModifiers::NONE),
                    &mut state,
                    &config,
                )
                .is_empty());
            }
            assert!(state.palette.open);
            let first = state.palette.matches[0];
            assert!(matches!(
                state.palette.entries[first].command,
                PaletteCommandKind::Quit
            ));
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut state,
                &config,
            );
            assert!(matches!(
                actions.as_slice(),
                [Action::System(textamp::app::action::SystemAction::Quit)]
            ));
            assert!(!state.palette.open);
        }
    }
}

#[test]
fn queue_query_still_ranks_queue_first() {
    let mut state = AppState::new();
    for query in ["queue", "Queue"] {
        command_palette::open_with_query(&mut state, query);
        let first = state.palette.matches[0];
        assert_eq!(state.palette.entries[first].label, "Queue");
    }
}

#[test]
fn every_browse_track_list_plays_its_visible_tail_not_a_hidden_library_column() {
    for &category in BrowseCategory::all() {
        if category == BrowseCategory::Folders {
            continue; // Folder items do not carry full Track objects.
        }
        let mut state = AppState::new();
        state.view = View::Browse;
        state.category_column_focused = false;
        state.browse_category = category;
        state.artist_nav.columns.push(column(&["hidden-library"]));
        let mut visible = column(&["before", "selected", "after"]);
        visible.selected_index = 1;
        state.browse_nav_mut().unwrap().columns = vec![visible];
        let actions = run_label(&mut state, "Play track and following");
        assert!(
            matches!(actions.as_slice(), [Action::Queue(QueueAction::PlayTracksNow(tracks))]
            if keys(tracks) == ["selected", "after"]),
            "{category:?}: {actions:?}"
        );
    }
}

#[test]
fn queue_and_radio_contexts_ignore_retained_browse_and_similar_selections() {
    for view in [View::Queue, View::NowPlaying] {
        for mode in [PlaybackMode::Queue, PlaybackMode::Radio] {
            let mut state = AppState::new();
            state.view = view;
            state.playback_mode = mode;
            state.artist_nav.columns.push(column(&["hidden-library"]));
            state.track_pane_focused = true;
            state.track_pane_index = 1;
            state
                .track_pane_similar
                .insert("hidden-library".into(), Ok(vec![track("hidden-similar")]));
            state.queue.tracks = vec![track("queue-0"), track("queue-1"), track("queue-2")];
            state.radio.tracks = vec![track("radio-0"), track("radio-1"), track("radio-2")];
            state.list_state.queue_index = 1;
            let expected = if mode == PlaybackMode::Radio {
                ["radio-1", "radio-2"]
            } else {
                ["queue-1", "queue-2"]
            };
            assert!(!state.palette_target_is_similar());
            assert_eq!(
                state.palette_target_track().unwrap().rating_key,
                expected[0]
            );
            let actions = run_label(&mut state, "Play track and following");
            assert!(
                matches!(actions.as_slice(), [Action::Queue(QueueAction::PlayTracksNow(tracks))]
                if keys(tracks) == expected)
            );
            let actions = run_label(&mut state, "Play track");
            assert!(
                matches!(actions.as_slice(), [Action::Queue(QueueAction::PlayTrack(track))]
                if track.rating_key == expected[0])
            );
        }
    }
}

#[test]
fn unrelated_views_and_invalid_rows_do_not_offer_hidden_track_or_album_commands() {
    let mut state = AppState::new();
    state.artist_nav.columns.push(column(&["hidden"]));
    for view in [
        View::Search,
        View::Help,
        View::Settings,
        View::Similar,
        View::Related,
    ] {
        state.view = view;
        assert!(state.palette_target_track().is_none());
        assert!(!command_palette::materialize_entries(&state)
            .iter()
            .any(|e| e.label == "Play track"));
    }
    state.view = View::Queue;
    state.queue.tracks = vec![track("only")];
    state.list_state.queue_index = 99;
    assert!(state.palette_target_track().is_none());
    assert!(
        command_palette::run(PaletteCommandKind::PlayFocusedTrackAndFollowing, &mut state)
            .is_empty()
    );
}

#[test]
fn refresh_palette_matches_f5_in_every_category() {
    for &category in BrowseCategory::all() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.browse_category = category;
        let palette = command_palette::run(PaletteCommandKind::Refresh, &mut state);
        let key = key_input::handle_key(
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            &mut state,
            &Config::default(),
        );
        assert_eq!(format!("{palette:?}"), format!("{key:?}"), "{category:?}");
    }
}

#[test]
fn utility_rows_are_native_commands_not_station_urls_in_both_playback_views() {
    for view in [View::Queue, View::NowPlaying] {
        let mut state = common::navidrome_state();
        state.view = view;
        state.stations.push(station(
            "nav-radio/randomAlbum",
            "Random Album Radio",
            "station",
        ));
        helpers::append_station_action_items(&mut state.stations, false);
        let entries = command_palette::materialize_entries(&state);
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.command, PaletteCommandKind::ToggleDj(_)))
                .count(),
            6
        );
        for entry in entries {
            if let PaletteCommandKind::PlayStation(key) = entry.command {
                assert_eq!(key, "nav-radio/randomAlbum");
            }
        }
        assert!(matches!(
            run_label(&mut state, "Artist Radio").as_slice(),
            [Action::Search(SearchAction::OpenArtistRadioPicker)]
        ));
        assert!(command_palette::materialize_entries(&state)
            .iter()
            .any(|e| matches!(e.command, PaletteCommandKind::RemixGemini)));
        assert!(!command_palette::materialize_entries(&state)
            .iter()
            .any(|e| e.label.contains("Friendg")));
    }
}

#[tokio::test]
async fn radio_removal_uses_the_active_list_even_when_the_old_queue_is_empty() {
    let mut state = AppState::new();
    state.view = View::Queue;
    state.playback_mode = PlaybackMode::Radio;
    state.radio.tracks = vec![track("keep"), track("remove")];
    state.radio.track_index = Some(0);
    state.list_state.queue_index = 1;
    let actions = command_palette::run(PaletteCommandKind::RemoveFocusedFromQueue, &mut state);
    assert!(matches!(
        actions.as_slice(),
        [Action::Queue(QueueAction::RemoveFromQueue(1))]
    ));

    let mut audio = AudioPlayer::new_without_audio();
    let mut config = Config::default();
    let (tx, _) = mpsc::channel(8);
    for action in actions {
        dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
            .await
            .unwrap();
    }
    assert_eq!(keys(&state.queue.tracks), ["keep"]);
    assert_eq!(state.current_track().unwrap().rating_key, "keep");
    assert!(state.queue.undo_snapshot.is_some());
}

#[test]
fn closing_the_palette_before_station_choices_arrive_does_not_reopen_it() {
    let mut state = AppState::new();
    command_palette::open_with_query(&mut state, "Radio");
    state.palette.close();

    let (tx, _) = mpsc::channel(8);
    handle_core_event(
        textamp::app::event::RadioEvent::StationChildrenLoaded {
            station_key: "/library/sections/5/mood".into(),
            station_title: "Mood Radio".into(),
            children: vec![station(
                "/library/sections/5/stations/mood?id=42",
                "Happy",
                "mood",
            )],
        }
        .into(),
        &mut state,
        &tx,
    );
    assert!(!state.palette.open);
}

#[test]
fn filtered_station_choices_are_leaves_regardless_of_their_identifiers() {
    for kind in ["mood", "style", "decade"] {
        let mut choice = station(
            &format!("/library/sections/5/stations/{kind}?id=42&title=Choice"),
            "Choice",
            kind,
        );
        choice.identifier = Some(kind.into());
        assert!(!choice.is_category());
        let mut state = AppState::new();
        state.stations = vec![choice];
        assert!(matches!(
            run_label(&mut state, "Radio: Choice").as_slice(),
            [Action::Radio(RadioAction::PlayStation(_))]
        ));
    }
}
