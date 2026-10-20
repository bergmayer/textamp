use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textamp::app::{
    dispatch::dispatch_action,
    handlers::key_input,
    state::{SettingsFocus, SettingsPage, SettingsSection, TextampSetting, View},
    AppState,
};
use textamp::{audio::AudioPlayer, config::Config};

#[tokio::test]
async fn audio_setting_persists_and_controls_the_next_stream_url() {
    let source = textamp::navidrome::Source {
        id: "stream-fixture".into(),
        name: "Fixture".into(),
        url: "http://127.0.0.1:9".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    let client = textamp::navidrome::Client::new(&source, "fixture".into(), None).unwrap();
    let mut state = AppState::new();
    state.sources.active = textamp::app::sources::ActiveSource::Navidrome(Box::new(
        textamp::app::sources::navidrome::Session {
            source,
            client: client.clone(),
            extensions: Default::default(),
        },
    ));
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.page = SettingsPage::Audio;
    state.settings_state.focus = SettingsFocus::Content;
    state.settings_state.item_index = state
        .textamp_settings()
        .iter()
        .position(|s| *s == TextampSetting::Transcode)
        .unwrap();
    let mut config = Config::default();
    let mut audio = AudioPlayer::new_without_audio();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    state.playback.position_ms = 4321;
    for expected in [128, 192, 256, 320, 0] {
        let actions = key_input::handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
            &config,
        );
        assert_eq!(actions.len(), 1);
        for action in actions {
            dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert_eq!(state.transcode_kbps, expected);
        assert_eq!(config.playback.transcode_kbps, expected);
        assert_eq!(state.playback.position_ms, 4321);
        let url = client
            .stream_url("track/with spaces", state.transcode_kbps)
            .unwrap();
        assert_eq!(url.path(), "/rest/stream");
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query["id"], "track/with spaces");
        if expected == 0 {
            assert_eq!(query["format"], "raw");
            assert!(!query.contains_key("maxBitRate"));
        } else {
            assert_eq!(query["maxBitRate"], expected.to_string());
            assert!(!query.contains_key("format"));
        }
        // Wait for the real asynchronous save, then reload through startup's path.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if textamp::config::load_config()
                    .unwrap()
                    .playback
                    .transcode_kbps
                    == expected
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
