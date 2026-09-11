use textamp::app::action::SystemAction;
use textamp::app::dispatch::dispatch_action;
use textamp::app::AppState;
use textamp::audio::AudioPlayer;
use textamp::config::Config;

use textamp::services::external_search::{generate_search_url, SearchTarget};

/// Opt-in because this activates Music and can request macOS permissions.
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Opens Music; requires Automation and Accessibility permission"]
async fn native_music_search_handoff() {
    let notice = textamp::services::external_search::open_search(SearchTarget::AppleMusic, "Björk")
        .await
        .unwrap();
    assert_eq!(notice, None, "native search fell back to the browser");
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
fn apple_music_web_fallback_skips_the_country_redirect() {
    assert_eq!(
        generate_search_url(SearchTarget::AppleMusic, "Miles Davis"),
        "https://music.apple.com/us/search?term=Miles%20Davis"
    );
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
        (true, " \t\n", "Nothing selected to search"),
    ] {
        config.ui.enable_apple_music_search = enabled;
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
