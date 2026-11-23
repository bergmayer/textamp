use textamp::app::sources::{ActiveSource, navidrome::Session};
use textamp::app::{
    AppState,
    action::{MillerAction, NavigationAction},
    state::{BrowseCategory, CategoryRow},
};
use textamp::library::models::Album;

#[tokio::test]
async fn release_decades_browse_the_cached_catalog_in_both_frontend_modes() {
    for desktop in [false, true] {
        let mut state = AppState::new();
        state.desktop_frontend = desktop;
        state.active_library = Some("fixture".into());
        let source = textamp::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active = ActiveSource::Navidrome(Box::new(Session {
            client: textamp::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        }));
        state.library.albums = std::sync::Arc::new(
            [1969, 1970, 1979, 1980]
                .map(|year| Album {
                    rating_key: year.to_string(),
                    title: year.to_string(),
                    year: Some(year),
                    ..Default::default()
                })
                .to_vec(),
        );
        assert!(
            state
                .category_rows()
                .iter()
                .any(|row| matches!(row, CategoryRow::Category(BrowseCategory::Decades)))
        );
        let mut audio = textamp::audio::AudioPlayer::new_without_audio();
        let mut config = textamp::config::Config::default();
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        textamp::app::dispatch::dispatch_action(
            NavigationAction::set_category(BrowseCategory::Decades).into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(
            state.tag_nav.columns[0]
                .items
                .iter()
                .map(|item| item.title())
                .collect::<Vec<_>>(),
            ["1960s", "1970s", "1980s"]
        );
        state.tag_nav.columns[0].selected_index = 1;
        textamp::app::dispatch::dispatch_action(
            MillerAction::LoadGenreAlbumsForMiller {
                genre_key: "1970".into(),
                replace_child: false,
            }
            .into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(state.tag_nav.columns.len(), 2);
        assert_eq!(
            state.tag_nav.columns[1]
                .items
                .iter()
                .map(|item| item.key())
                .collect::<Vec<_>>(),
            ["1970", "1979"]
        );
        assert!(state.queue.tracks.is_empty());
    }
    assert_eq!(BrowseCategory::AlbumGenres.display_label(), "Genres");
    assert_eq!(
        serde_json::from_str::<BrowseCategory>("\"AlbumGenres\"").unwrap(),
        BrowseCategory::AlbumGenres
    );
}
