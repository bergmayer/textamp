use textamp::app::AppState;
use textamp::app::action::SystemAction;
use textamp::app::dispatch::dispatch_action;
use textamp::audio::AudioPlayer;
use textamp::config::Config;

use textamp::services::external_search::{SearchTarget, generate_search_url};

/// Opt-in because this brings Music to the foreground.
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Opens the Music app"]
async fn native_music_search_handoff() {
    let notice = textamp::services::external_search::open_search(SearchTarget::AppleMusic, "Björk")
        .await
        .unwrap();
    assert_eq!(notice, None);
}

#[tokio::test]
#[ignore = "Queries the live US Apple catalog"]
async fn apple_music_song_opens_its_catalog_album_with_the_track_selected() {
    use textamp::services::external_search::{MusicQuery, resolve};
    let result = resolve(
        SearchTarget::AppleMusic,
        MusicQuery::Song {
            artist: "Guided by Voices".into(),
            title: "Doughnut for a Snowman".into(),
            album: Some("Let's Go Eat the Factory".into()),
        },
    )
    .await;
    let url =
        reqwest::Url::parse(result.native_url.as_deref().expect("song should resolve")).unwrap();
    assert_eq!(url.host_str(), Some("music.apple.com"));
    assert_eq!(url.path_segments().unwrap().next_back(), Some("480002450"));
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "i" && value == "480002454")
    );
}

#[test]
fn external_search_preserves_special_characters_as_one_query() {
    let query = "Björk & Ryuichi Sakamoto - A/B + #1?";
    for (target, parameter) in [
        (SearchTarget::AppleMusic, "term"),
        (SearchTarget::YouTube, "search_query"),
    ] {
        let url = reqwest::Url::parse(&generate_search_url(target, query)).unwrap();
        assert_eq!(url.fragment(), None);
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![(parameter.into(), query.into())]
        );
    }
    let url = reqwest::Url::parse(&generate_search_url(SearchTarget::Spotify, query)).unwrap();
    assert_eq!(url.query(), None);
    assert_eq!(url.fragment(), None);
    assert_eq!(
        urlencoding::decode(url.path().strip_prefix("/search/").unwrap()).unwrap(),
        query
    );
}

#[test]
fn apple_music_universal_link_includes_the_storefront() {
    assert_eq!(
        generate_search_url(SearchTarget::AppleMusic, "Miles Davis"),
        "https://music.apple.com/us/search?term=Miles%20Davis"
    );
}

#[test]
fn apple_music_shortcut_dispatches_the_palette_search_in_the_terminal() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use textamp::app::{Action, handlers::key_input, state::PlayStatus};
    use textamp::library::models::Track;
    let mut state = AppState::new();
    state.queue.tracks = vec![Track {
        title: "Test song".into(),
        ..Default::default()
    }];
    state.queue.index = Some(0);
    state.playback.status = PlayStatus::Playing;
    for modifier in [KeyModifiers::SUPER, KeyModifiers::CONTROL] {
        for letter in ['a', 'A'] {
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Char(letter), modifier | KeyModifiers::SHIFT),
                &mut state,
                &Config::default(),
            );
            assert!(matches!(
                actions.as_slice(),
                [Action::System(SystemAction::OpenExternalSearch {
                    target: SearchTarget::AppleMusic,
                    query: Some(_),
                    ..
                })]
            ));
        }
    }
}

#[tokio::test]
async fn disabled_and_blank_searches_do_not_open_external_apps() {
    let mut state = AppState::new();
    let mut config = Config::default();

    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for (enabled, query, expected) in [
        (
            false,
            "Miles Davis",
            "Apple Music search is disabled in Settings",
        ),
        (true, " \t\n", "No artist, album or track to search"),
    ] {
        state.external_search.apple_music = enabled;
        dispatch_action(
            SystemAction::OpenExternalSearch {
                target: SearchTarget::AppleMusic,
                query: Some(query.into()),
            }
            .into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(
            state.notifications.status_message.as_deref(),
            Some(expected)
        );
        assert!(state.notifications.last_error.is_none());
    }
}
