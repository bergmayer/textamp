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
    state.sources.audiomuse.connections.insert(
        textamp::app::sources::audiomuse::key(state.sources.active.navidrome().unwrap()),
        textamp::audiomuse::Connection {
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            server: "Music".into(),
        },
    );
    state.view = View::Queue;
    state.queue.tracks = vec![track("playing"), track("selected")];
    state.queue.index = Some(0);
    state.list_state.queue_index = 1;
    let entries =
        textamp::app::track_context::track_context_entries(&state, &track("selected"), false);
    assert!(
        entries
            .iter()
            .any(|e| matches!(e.kind, textamp::app::track_context::ContextKind::SonicRadio))
    );
    let actions = run_label(&mut state, "Start Sonic Mix based on track");
    assert!(
        matches!(actions.as_slice(), [Action::Radio(RadioAction::StartSonicRadio(track))] if track.rating_key == "selected")
    );
    state.sources.active = textamp::app::sources::ActiveSource::Folder("local".into());
    assert!(
        !command_palette::materialize_entries(&state)
            .iter()
            .any(|entry| entry.label == "Start Sonic Mix based on track")
    );
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
                assert!(
                    key_input::handle_key(
                        KeyEvent::new(code, KeyModifiers::NONE),
                        &mut state,
                        &config,
                    )
                    .is_empty()
                );
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
                [
                    Action::Settings(textamp::app::action::SettingsAction::PersistCommandHistory),
                    Action::System(textamp::app::action::SystemAction::Quit)
                ]
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
        assert_eq!(state.palette.entries[first].label, "Play Queue");
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
            for track in state.queue.tracks.iter_mut().chain(&mut state.radio.tracks) {
                track.parent_rating_key = Some("album".into());
                track.grandparent_rating_key = Some("artist".into());
            }
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
            let entries = command_palette::materialize_entries(&state);
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry.label == "Open in Library")
                    .count(),
                1
            );
            let open = run_label(&mut state, "Open in Library");
            assert!(
                matches!(open.as_slice(), [Action::Browse(textamp::app::action::BrowseAction::OpenDetailsInLibrary {
                track, destination: textamp::app::library_details::Destination::Track
            })] if track.rating_key == expected[0])
            );
            if view == View::NowPlaying {
                assert!(!entries.iter().any(|entry| matches!(
                    entry.label.as_str(),
                    "Play track" | "Play track and following" | "Show Similar Tracks"
                )));
                continue;
            }
            assert!(!entries.iter().any(|entry| matches!(
                entry.label.as_str(),
                "Play track and following" | "Play next in queue" | "Add to end of queue"
            )));
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
        assert!(
            !command_palette::materialize_entries(&state)
                .iter()
                .any(|e| e.label == "Play track")
        );
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
        let mut state = common::navidrome_state();
        state.active_library = Some("navidrome:fixture:all".into());
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
        state.active_library = Some("navidrome:fixture:all".into());
        state.view = view;
        state.station_nav.columns = vec![textamp::app::state::StationColumn::new(
            None,
            "stations".into(),
            vec![station(
                "nav-radio/randomAlbum",
                "Play Random Albums",
                "station",
            )],
        )];
        helpers::append_station_action_items(&mut state.station_nav.columns[0].stations, false);
        let entries = command_palette::materialize_entries(&state);
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.label.starts_with("DJ "))
                .count(),
            0
        );
        for entry in entries {
            if let PaletteCommandKind::PlayStation(key) = entry.command {
                assert_eq!(key, "nav-radio/randomAlbum");
            }
        }
        assert!(matches!(
            run_label(&mut state, "Artist Mix").as_slice(),
            [Action::Search(SearchAction::OpenArtistRadioPicker)]
        ));
        assert!(
            command_palette::materialize_entries(&state)
                .iter()
                .any(|e| matches!(e.command, PaletteCommandKind::RemixGemini))
        );
        assert!(
            !command_palette::materialize_entries(&state)
                .iter()
                .any(|e| e.label.contains("Friendg"))
        );
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
    assert_eq!(state.playback_mode, PlaybackMode::Radio);
    assert_eq!(keys(&state.radio.tracks), ["keep"]);
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
            station_title: "Mood Mix".into(),
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
        state.station_nav.columns = vec![textamp::app::state::StationColumn::new(
            None,
            "stations".into(),
            vec![choice],
        )];
        assert!(matches!(
            run_label(&mut state, "Radio: Choice").as_slice(),
            [Action::Radio(RadioAction::PlayStation(_))]
        ));
    }
}

#[test]
fn similar_and_related_remain_detail_pane_destinations() {
    let mut state = common::navidrome_state();
    state.queue.tracks = vec![track("playing"), track("selected")];
    state.queue.index = Some(0);
    for view in [View::Queue, View::Settings, View::Browse] {
        state.view = view;
        assert!(
            !command_palette::materialize_entries(&state)
                .iter()
                .any(|entry| entry.label.starts_with("Show Similar")
                    || matches!(
                        entry.label.as_str(),
                        "Similar" | "Related" | "Related Artists"
                    ))
        );
    }
}

#[test]
fn palette_submenus_preserve_context_and_global_fuzzy_search() {
    let mut state = AppState::new();
    command_palette::open(&mut state);
    state.palette.player_context = true;
    let labels = |state: &AppState| {
        state
            .palette
            .matches
            .iter()
            .map(|&index| state.palette.entries[index].label.clone())
            .collect::<Vec<_>>()
    };
    assert!(labels(&state).contains(&"View".into()));
    assert!(!labels(&state).contains(&"By Title".into()));
    command_palette::run(
        PaletteCommandKind::OpenCommandGroup(vec!["View".into(), "Sort".into()]),
        &mut state,
    );
    assert!(state.palette.open && state.palette.player_context);
    assert!(labels(&state).contains(&"By Title".into()));
    assert!(labels(&state).contains(&"Back".into()));
    command_palette::handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(state.palette.path, ["View"]);
    command_palette::open_with_query(&mut state, "view sort title");
    let entry = &state.palette.entries[state.palette.matches[0]];
    assert!(matches!(
        entry.command,
        PaletteCommandKind::ApplySort(textamp::app::state::ColumnSortMode::ByTitle)
    ));
    assert!(!state.palette.matches.iter().any(|&index| matches!(
        state.palette.entries[index].command,
        PaletteCommandKind::OpenCommandGroup(_)
    )));
}

#[test]
fn palette_puts_selection_actions_before_menu_groups_and_browses_queue_destination() {
    let mut state = common::navidrome_state();
    state.view = View::Queue;
    state.queue.tracks = vec![track("selected")];
    command_palette::open(&mut state);
    let commands = state
        .palette
        .matches
        .iter()
        .map(|&i| &state.palette.entries[i].command)
        .collect::<Vec<_>>();
    let first_group = commands
        .iter()
        .position(|command| matches!(command, PaletteCommandKind::OpenCommandGroup(_)))
        .unwrap();
    assert!(first_group > 1);
    assert!(
        commands[..first_group]
            .iter()
            .all(|command| textamp::app::command_catalog::contextual(command))
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, PaletteCommandKind::GotoQueue))
    );

    let shortcuts: Vec<_> = state.palette.matches[..first_group]
        .iter()
        .map(|&index| {
            let entry = &state.palette.entries[index];
            (
                entry.label.clone(),
                textamp::app::command_catalog::path(&entry.command),
            )
        })
        .collect();
    for (label, home) in shortcuts {
        assert!(!home.is_empty(), "{label} needs a menu home");
        command_palette::run(PaletteCommandKind::OpenCommandGroup(home), &mut state);
        assert!(
            state
                .palette
                .matches
                .iter()
                .any(|&i| state.palette.entries[i].label == label)
        );
    }
    command_palette::open(&mut state);
    for path in [
        vec![String::from("View")],
        vec!["View".into(), "Browse".into()],
    ] {
        let index = state.palette.matches.iter().position(|&index| matches!(&state.palette.entries[index].command, PaletteCommandKind::OpenCommandGroup(target) if *target == path)).unwrap();
        state.palette.selected = index;
        assert!(matches!(
            command_palette::handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
            ),
            command_palette::PaletteOutcome::Continue
        ));
        assert_eq!(state.palette.path, path);
    }
    assert!(state.palette.matches.iter().any(|&index| matches!(
        state.palette.entries[index].command,
        PaletteCommandKind::GotoQueue
    )));
    command_palette::handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(state.palette.path, ["View"]);
    state.palette.query = "play queue".into();
    command_palette::refresh_matches(&mut state);
    assert!(matches!(
        state.palette.entries[state.palette.matches[0]].command,
        PaletteCommandKind::GotoQueue
    ));
}

#[test]
fn palette_root_promotes_available_music_searches_with_canonical_routes() {
    let mut state = common::navidrome_state();
    command_palette::open(&mut state);
    let commands: Vec<_> = state
        .palette
        .matches
        .iter()
        .map(|&i| &state.palette.entries[i].command)
        .collect();
    assert!(!commands.iter().any(|c| matches!(
        c,
        PaletteCommandKind::SearchAppleMusic | PaletteCommandKind::OpenSettings
    )));
    command_palette::run(
        PaletteCommandKind::OpenCommandGroup(vec!["Textamp".into()]),
        &mut state,
    );
    assert!(state.palette.matches.iter().any(|&i| matches!(
        state.palette.entries[i].command,
        PaletteCommandKind::OpenSettings
    )));

    state.view = View::Queue;
    state.queue.tracks = vec![track("selected")];
    command_palette::open(&mut state);
    let commands: Vec<_> = state
        .palette
        .matches
        .iter()
        .map(|&i| &state.palette.entries[i].command)
        .collect();
    let group = commands
        .iter()
        .position(|c| matches!(c, PaletteCommandKind::OpenCommandGroup(_)))
        .unwrap();
    let search = commands
        .iter()
        .position(|c| matches!(c, PaletteCommandKind::SearchAppleMusic))
        .unwrap();
    assert!(search < group);
    let context =
        textamp::app::track_context::track_context_entries(&state, &track("selected"), false);
    for entry in context {
        if matches!(
            entry.kind,
            textamp::app::track_context::ContextKind::Separator
                | textamp::app::track_context::ContextKind::SearchExternal(_)
        ) {
            continue;
        }
        let command = PaletteCommandKind::FromTrackContext {
            kind: entry.kind,
            track: Box::new(track("selected")),
        };
        if textamp::app::command_catalog::disabled_reason(&state, &command).is_some() {
            continue;
        }
        let index = state
            .palette
            .matches
            .iter()
            .position(|&i| state.palette.entries[i].label == entry.label)
            .unwrap_or_else(|| panic!("Missing selected-track action: {}", entry.label));
        assert!(
            index < group,
            "{} belongs with the selected track",
            entry.label
        );
    }
}

#[test]
fn palette_music_search_uses_selected_queue_track_before_the_playing_track() {
    let mut state = common::navidrome_state();
    state.view = View::Queue;
    state.queue.tracks = ["playing", "selected"]
        .map(|key| {
            let mut track = track(key);
            track.grandparent_title = Some("Artist".into());
            track
        })
        .to_vec();
    state.queue.index = Some(0);
    state.list_state.queue_index = 1;
    assert_eq!(
        textamp::app::external_search::query(&state).as_deref(),
        Some("Artist - selected")
    );
    state.view = View::Browse;
    state.category_column_focused = true;
    assert_eq!(
        textamp::app::external_search::query(&state).as_deref(),
        Some("Artist - playing")
    );
}
