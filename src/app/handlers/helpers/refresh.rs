//! View refresh, stale data detection, and background category refresh.

use crate::app::action::*;
use crate::app::state::{BrowseCategory, View};
use crate::app::{Action, AppState, Event};

use tokio::sync::mpsc;

/// Refresh the current view's category and return actions.
pub fn refresh_current_view(state: &mut AppState) -> Vec<Action> {
    state.artist_bios.clear();
    state.sources.album_artists = None;
    state.sources.nav_tasks.remove("album-artists");
    use crate::app::state::RefreshCategory;

    if state.view == View::Browse
        && let Some((seed, _)) = crate::app::details::similar_seed(state)
        && state
            .track_pane_similar
            .get(&seed.rating_key)
            .is_some_and(Result::is_err)
    {
        state.track_pane_similar.remove(&seed.rating_key);
        state.clear_error();
        return vec![
            DataAction::LoadTrackPaneSimilar {
                rating_key: seed.rating_key,
            }
            .into(),
        ];
    }

    if state.view == View::Similar {
        let Some(key) = state.similar.source_key.clone() else {
            return vec![];
        };
        let title = state.similar.source_title.clone();
        return vec![
            match state.similar.mode {
                crate::app::state::SimilarMode::Tracks => DataAction::LoadSimilarTracks {
                    rating_key: key,
                    title,
                },
                crate::app::state::SimilarMode::Albums => DataAction::LoadSimilarAlbums {
                    rating_key: key,
                    title,
                },
                crate::app::state::SimilarMode::Artists => DataAction::LoadSimilarArtists {
                    artist_key: key,
                    title,
                },
            }
            .into(),
        ];
    }
    if state.view == View::Settings
        && state.settings_state.section == crate::app::state::SettingsSection::Libraries
    {
        return vec![SettingsAction::RefreshCacheStats.into()];
    }
    if let Some(choice) = crate::app::sources::library_choices(state)
        .into_iter()
        .find(|c| c.active(state))
    {
        crate::app::sources::cache::resume(&choice, state);
    }
    if state.sources.active.navidrome().is_some() {
        if let Some(kind) = state.sources.nav_collection.filter(|_| {
            state.view == View::Browse && state.browse_category == BrowseCategory::Library
        }) {
            if let crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(feature) =
                kind
            {
                return vec![
                    crate::app::sources::audiomuse::Command::Open {
                        feature,
                        refresh: true,
                    }
                    .into(),
                ];
            }
            if kind == crate::app::sources::navidrome::commands::CollectionKind::RandomAlbums {
                state.sources.random_albums =
                    crate::app::sources::navidrome::random_albums::State::RefreshPending;
                state.sources.nav_tasks.remove("random-albums");
            }
            return vec![kind.action()];
        }
        return vec![SystemAction::RefreshCategory(RefreshCategory::Artists).into()];
    }
    if state.sources.active.folder().is_some() {
        let path = state
            .folder_state
            .as_ref()
            .and_then(|nav| nav.focused())
            .and_then(|column| column.key.clone())
            .unwrap_or_default();
        return vec![FolderAction::RefreshSubfolder(path).into()];
    }

    // Special handling for Folders
    if state.view == View::Browse && state.browse_category == BrowseCategory::Folders {
        let subfolder_key = state.folder_state.as_ref().and_then(|folder_state| {
            if folder_state.focused_column > 0 {
                folder_state
                    .columns
                    .get(folder_state.focused_column)
                    .and_then(|col| col.key.clone())
            } else {
                None
            }
        });

        if let Some(folder_key) = subfolder_key {
            state.set_status("Refreshing folder...".to_string());
            return vec![FolderAction::RefreshSubfolder(folder_key).into()];
        }
    }

    // Check if we're viewing the All Library Tracks column (artist_nav, All Artists → All Tracks)
    if state.view == View::Browse
        && state.browse_category == BrowseCategory::Library
        && state.artist_nav.focused_column >= 2
    {
        // Check if parent column's selected item is the "__all_library__" AllTracks entry
        if let Some(parent_col) = state
            .artist_nav
            .columns
            .get(state.artist_nav.focused_column - 1)
            && let Some(item) = parent_col.selected_item()
            && matches!(
                item,
                crate::app::state::BrowseItem::AllTracks {
                    scope: crate::app::state::AllTracksScope::Library,
                    ..
                }
            )
        {
            state.set_status("Refreshing all tracks...".to_string());
            return vec![SystemAction::RefreshCategory(RefreshCategory::AllTracks).into()];
        }
    }

    // Check if we're viewing album tracks in a Miller column — refresh just that album
    if state.view == View::Browse {
        let album_key = match state.browse_category {
            BrowseCategory::Library => {
                if state.artist_nav.focused_column >= 2 {
                    state.artist_nav.focused().and_then(|col| {
                        if col
                            .items
                            .iter()
                            .any(|i| matches!(i, crate::app::state::BrowseItem::Track { .. }))
                            && !col.tracks.is_empty()
                        {
                            state
                                .artist_nav
                                .columns
                                .get(state.artist_nav.focused_column - 1)
                                .and_then(|parent| parent.selected_item())
                                .map(|item| item.key().to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }
            cat if cat.is_tag_section() => {
                // Tag nav: root tags → albums → tracks (depth >= 2)
                if state.tag_nav.focused_column >= 2 {
                    state.tag_nav.focused().and_then(|col| {
                        if col
                            .items
                            .iter()
                            .any(|i| matches!(i, crate::app::state::BrowseItem::Track { .. }))
                            && !col.tracks.is_empty()
                        {
                            state
                                .tag_nav
                                .columns
                                .get(state.tag_nav.focused_column - 1)
                                .and_then(|parent| parent.selected_item())
                                .map(|item| item.key().to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(key) = album_key {
            state.set_status("Refreshing album tracks...".to_string());
            return vec![MillerAction::RefreshAlbumTracks { album_key: key }.into()];
        }
    }

    let category = match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Library => Some(RefreshCategory::Artists),
            BrowseCategory::Playlists => Some(RefreshCategory::Playlists),
            BrowseCategory::Folders => Some(RefreshCategory::Folders),
            cat if cat.is_tag_section() => RefreshCategory::for_tag_section(cat),
            _ => None,
        },
        _ => None,
    };

    if let Some(cat) = category
        && !state.cache_mgmt.background_refresh.contains(&cat)
    {
        state.set_status(format!("Refreshing {}...", cat.display_name()));
        return vec![SystemAction::RefreshCategory(cat).into()];
    }
    vec![]
}

/// Map current view state to its primary RefreshCategory.
pub fn current_view_category(state: &AppState) -> Option<crate::app::state::RefreshCategory> {
    use crate::app::state::RefreshCategory;

    match state.view {
        View::Browse => match state.browse_category {
            BrowseCategory::Library => Some(RefreshCategory::Artists),
            BrowseCategory::Playlists => Some(RefreshCategory::Playlists),
            BrowseCategory::Folders => Some(RefreshCategory::Folders),
            cat if cat.is_tag_section() => RefreshCategory::for_tag_section(cat),
            _ => None,
        },
        _ => None,
    }
}

/// Runs on tick, so leaving a view open does not keep its cache stale forever.
/// Inactive libraries and unvisited directories are checked on their next open;
/// there is no background daemon or whole-share crawl.
pub fn refresh_due_caches(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let now = std::time::Instant::now();
    if state.library_loading
        || state
            .cache_mgmt
            .next_refresh_check
            .is_some_and(|at| at > now)
    {
        return;
    }
    state.cache_mgmt.next_refresh_check = Some(now + std::time::Duration::from_secs(3600));
    {
        crate::app::sources::refresh_due_caches(state, tx);
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use crate::{
        app::state::{BrowseColumn, BrowseItem},
        library::models::Track,
    };

    #[test]
    fn failed_detail_retry_keeps_the_browsed_album_and_its_tracks() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.category_column_focused = false;
        let track = Track {
            rating_key: "track".into(),
            parent_rating_key: Some("album".into()),
            ..Default::default()
        };
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Album",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        )];
        state
            .track_pane_similar
            .insert("track".into(), Err("Connection failed".into()));
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = crate::app::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        state.sources.audiomuse.connections.insert(
            crate::app::sources::audiomuse::key(&session),
            crate::audiomuse::Connection {
                url: "http://127.0.0.1:1".into(),
                username: "fixture".into(),
                server: "fixture".into(),
            },
        );
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(session));
        state.track_pane_open = true;
        let actions = refresh_current_view(&mut state);
        assert!(
            matches!(actions.as_slice(), [Action::Data(DataAction::LoadTrackPaneSimilar { rating_key })] if rating_key == "track")
        );
        assert!(!state.track_pane_similar.contains_key("track"));
        assert_eq!(
            state.artist_nav.columns[0].tracks[0]
                .parent_rating_key
                .as_deref(),
            Some("album")
        );
    }
}
