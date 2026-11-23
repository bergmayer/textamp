//! Projection of application browse rows into UI-independent filter records.
use crate::services::BrowseFilterRecord;
pub fn browse_filter_records(items: &[crate::app::state::BrowseItem]) -> Vec<BrowseFilterRecord> {
    use crate::app::state::BrowseItem;

    items
        .iter()
        .map(|item| BrowseFilterRecord {
            title: item.title().to_string(),
            search_text: match item {
                BrowseItem::Album { title, artist, .. } => Some(format!("{title} {artist}")),
                BrowseItem::Track {
                    title,
                    artist_name,
                    album_name,
                    ..
                } => Some(format!(
                    "{} {} {}",
                    title,
                    artist_name.as_deref().unwrap_or_default(),
                    album_name.as_deref().unwrap_or_default()
                )),
                _ => None,
            },
            album_year: match item {
                BrowseItem::Album { year, .. } => *year,
                _ => None,
            },
            artist_key: match item {
                BrowseItem::Artist { key, .. } => Some(key.clone()),
                _ => None,
            },
            is_compilations: matches!(item, BrowseItem::Compilations),
        })
        .collect()
}
#[cfg(test)]
mod artist_credit_tests {
    use super::*;
    #[tokio::test]
    async fn album_filter_completes_through_the_live_session_dispatch() {
        use crate::app::{
            action::SearchAction,
            runtime::{Session, reduce},
            state::{BrowseColumn, BrowseItem},
        };
        let mut session = Session::with_audio(
            Default::default(),
            crate::audio::AudioPlayer::new_without_audio(),
        );
        session.state.artist_nav.columns = vec![BrowseColumn::new(
            "all albums",
            ["A", "B"]
                .into_iter()
                .map(|letter| BrowseItem::Album {
                    key: letter.into(),
                    title: format!("{letter} Album"),
                    artist: format!("{letter} Artist"),
                    year: None,
                    thumb: None,
                    is_placeholder: false,
                })
                .collect(),
        )];
        session
            .dispatch(SearchAction::SetListFilterQuery("\"B Artist\"".into()).into())
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while session.state.list_filter.results.is_none() {
                let event = session.completions.recv().await.unwrap();
                for action in reduce(event, &mut session.state, &session.config, &session.events) {
                    session.dispatch(action).await.unwrap();
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            session
                .state
                .list_filter
                .results
                .as_ref()
                .unwrap()
                .matched_indices,
            vec![1]
        );
        assert_eq!(session.state.artist_nav.columns[0].selected_index, 1);
    }

    #[test]
    fn album_filter_matches_artist_and_title_without_changing_result_identity() {
        use crate::app::state::BrowseItem;
        let items = vec![
            BrowseItem::Album {
                key: "voyager".into(),
                title: "Voyager".into(),
                artist: "Momus".into(),
                year: Some(1992),
                thumb: None,
                is_placeholder: false,
            },
            BrowseItem::Album {
                key: "other".into(),
                title: "Other Album".into(),
                artist: "Other Artist".into(),
                year: None,
                thumb: None,
                is_placeholder: false,
            },
        ];
        for query in ["momus", "Voyager", "Momus AND Voyager"] {
            let result = crate::services::filter_browse_records(
                &browse_filter_records(&items),
                query,
                100,
                &Default::default(),
                &Default::default(),
            );
            assert_eq!(result.matched_indices, vec![0]);
        }
    }
    #[test]
    fn column_filter_keeps_sort_order_and_ignores_typo_only_matches() {
        use crate::{app::state::BrowseItem, library::models::Track};
        let tracks = ["My Heartbeat", "Heartbaet", "Heartbeat"].map(|title| Track {
            title: title.into(),
            ..Default::default()
        });
        let records = browse_filter_records(&BrowseItem::from_tracks(&tracks));
        let result = crate::services::filter_browse_records(
            &records,
            "heartbeat",
            1,
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(result.matched_indices, [0]);
        assert_eq!(result.total_matches, 2);
        assert!(result.has_more);
    }

    #[test]
    fn album_track_filter_matches_individual_performers_and_combined_queries() {
        use crate::{app::state::BrowseItem, library::models::Track};
        let tracks = vec![
            Track {
                rating_key: "a".into(),
                title: "First song".into(),
                original_title: Some("Guest Performer".into()),
                ..Default::default()
            },
            Track {
                rating_key: "b".into(),
                title: "Second song".into(),
                original_title: Some("Other Artist".into()),
                ..Default::default()
            },
        ];
        let items = BrowseItem::from_tracks(&tracks);
        for query in ["Guest", "First AND Performer"] {
            let result = crate::services::filter_browse_records(
                &browse_filter_records(&items),
                query,
                100,
                &Default::default(),
                &Default::default(),
            );
            assert_eq!(result.matched_indices, vec![0]);
        }
    }
}

/// Re-run an active column filter after its underlying items change.
pub(crate) fn refresh_column(
    state: &mut crate::app::AppState,
    column: usize,
) -> Option<crate::app::Action> {
    if !state.list_filter.active
        || state.list_filter.category != state.browse_category
        || state.list_filter.column != column
    {
        return None;
    }
    let anchor = state.browse_nav()?.columns.get(column)?.selected_index;
    state.list_filter.advance_version();
    state.list_filter.refresh_anchor = Some(anchor);
    state.list_filter.results = None;
    state.list_filter.loading = true;
    Some(
        crate::app::action::SearchAction::RunListFilter {
            version: state.list_filter.version,
        }
        .into(),
    )
}
