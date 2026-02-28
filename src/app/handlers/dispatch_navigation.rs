//! Navigation dispatch handlers: SetView, NextView, PrevView, NextMode, PrevMode,
//! SetCategory, ToggleFocus.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, Focus, GenreContentType, RightPanelMode, View};
use crate::plex::PlexClient;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch navigation actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    // Deactivate inline filter on view or category change
    if matches!(action, Action::SetView(_) | Action::SetCategory(_)) && state.list_filter.active {
        state.list_filter.deactivate();
    }

    match action {
        Action::SetView(view) => {
            // Clear artwork cache when leaving Similar view to force re-render
            // (Similar popup's Clear widget can corrupt terminal images)
            if state.view == View::Similar {
                crate::ui::screens::now_playing::clear_artwork_cache();
            }
            state.set_view(view);
            // Load stations when entering Queue view if not already loaded
            if view == View::Queue
                && state.station_nav.columns.is_empty()
                && !state.station_nav.loading
            {
                follow_ups.push(Action::LoadStations);
            }
            // Load waveform and spectrogram when entering NowPlaying view
            if view == View::NowPlaying {
                follow_ups.push(Action::LoadWaveform);
                follow_ups.push(Action::LoadSpectrogram);
            }
        }
        Action::NextView => {
            // Tab: cycle through views in displayed tab bar order
            // Order: Library → Playlists → Genres → Folders → Queue → Now Playing → Library
            if state.view == View::NowPlaying {
                state.set_view(View::Browse);
                follow_ups.push(Action::SetCategory(BrowseCategory::Library));
            } else if state.view == View::Queue {
                state.set_view(View::NowPlaying);
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
                    }
                    BrowseCategory::Playlists => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Genres));
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Folders));
                    }
                    BrowseCategory::Folders => {
                        state.set_view(View::Queue);
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.set_view(View::Browse);
            }
        }
        Action::PrevView => {
            // Shift+Tab: cycle backwards through views in displayed tab bar order
            // Order: Library ← Playlists ← Genres ← Folders ← Queue ← Now Playing ← Library
            if state.view == View::NowPlaying {
                state.set_view(View::Queue);
            } else if state.view == View::Queue {
                state.set_view(View::Browse);
                follow_ups.push(Action::SetCategory(BrowseCategory::Folders));
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        state.set_view(View::NowPlaying);
                    }
                    BrowseCategory::Playlists => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Library));
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
                    }
                    BrowseCategory::Folders => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Genres));
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.set_view(View::Browse);
            }
        }
        Action::SetCategory(category) => {
            if state.browse_category != category {
                // Unshuffle Library root when leaving, so "All Artists" is in correct position
                if state.browse_category == BrowseCategory::Library {
                    if let Some(col) = state.artist_nav.columns.first_mut() {
                        if col.is_shuffled() {
                            col.unshuffle();
                        }
                    }
                }
                state.browse_category = category;
                state.focus = Focus::Left;
                // Clear right panel
                state.right_panel_mode = RightPanelMode::Empty;
                state.selected_artist_albums.clear();
                state.selected_album_tracks.clear();

                // Load category data if needed (and not already loading)
                match category {
                    BrowseCategory::Library => {
                        if state.artists.is_empty() && !state.artists_loading {
                            helpers::load_artists(event_tx, state, client);
                        } else {
                            // Build second column synchronously from cached data
                            follow_ups.extend(super::dispatch_data::auto_drill_from_cache(state));
                        }
                    }
                    BrowseCategory::Playlists => {
                        if state.playlists.is_empty() && !state.playlists_loading {
                            helpers::load_playlists(event_tx, state, client);
                        } else {
                            // Rebuild root column from state.playlists to ensure it's populated
                            // (guards against stale/empty nav from preload race conditions)
                            let items = crate::app::state::BrowseItem::from_playlists(&state.playlists);
                            state.playlist_nav.reset("playlists", items);
                            // Build second column synchronously (async fallback for smart playlists)
                            follow_ups.extend(super::dispatch_data::auto_drill_from_cache(state));
                        }
                    }
                    BrowseCategory::Genres => {
                        if state.genre_tab == crate::app::state::GenreTab::All {
                            follow_ups.push(Action::RefreshGenreView);
                        } else {
                            // Load the appropriate content based on current genre content type
                            match state.genre_content_type {
                                GenreContentType::Genres => {
                                    if state.genres.is_empty() && !state.genres_loading {
                                        follow_ups.push(Action::LoadGenres);
                                    }
                                }
                                GenreContentType::ArtistGenres => {
                                    if state.artist_genres.is_empty() && !state.artist_genres_loading {
                                        follow_ups.push(Action::LoadArtistGenres);
                                    }
                                }
                                GenreContentType::AlbumGenres => {
                                    if state.album_genres.is_empty() && !state.album_genres_loading {
                                        follow_ups.push(Action::LoadAlbumGenres);
                                    }
                                }
                                GenreContentType::Moods => {
                                    if state.moods.is_empty() && !state.moods_loading {
                                        follow_ups.push(Action::LoadMoods);
                                    }
                                }
                                GenreContentType::Styles => {
                                    if state.styles.is_empty() && !state.styles_loading {
                                        follow_ups.push(Action::LoadStyles);
                                    }
                                }
                            }
                        }
                    }
                    BrowseCategory::Folders => {
                        if state.folder_state.is_none() {
                            follow_ups.push(Action::LoadFolderRoot);
                        }
                    }
                }
            }
        }
        Action::ToggleFocus => {
            state.focus = match state.focus {
                Focus::Left => Focus::Right,
                Focus::Right => Focus::Left,
            };
        }
        _ => unreachable!("dispatch_navigation called with non-navigation action: {:?}", action),
    }
    Ok(follow_ups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::PlexClientInfo;

    fn setup() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>, AppState, PlexClient) {
        let (tx, rx) = mpsc::channel(100);
        let state = AppState::new();
        let client = PlexClient::new(PlexClientInfo::default());
        (tx, rx, state, client)
    }

    #[tokio::test]
    async fn toggle_focus_left_to_right() {
        let (tx, _rx, mut state, mut client) = setup();
        state.focus = Focus::Left;

        dispatch(&tx, Action::ToggleFocus, &mut state, &mut client).await.unwrap();
        assert_eq!(state.focus, Focus::Right);
    }

    #[tokio::test]
    async fn toggle_focus_right_to_left() {
        let (tx, _rx, mut state, mut client) = setup();
        state.focus = Focus::Right;

        dispatch(&tx, Action::ToggleFocus, &mut state, &mut client).await.unwrap();
        assert_eq!(state.focus, Focus::Left);
    }

    #[tokio::test]
    async fn set_view_changes_state() {
        let (tx, _rx, mut state, mut client) = setup();

        dispatch(&tx, Action::SetView(View::Queue), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Queue);

        dispatch(&tx, Action::SetView(View::Help), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Help);
    }

    #[tokio::test]
    async fn set_view_queue_requests_load_stations() {
        let (tx, _rx, mut state, mut client) = setup();

        let follow_ups = dispatch(&tx, Action::SetView(View::Queue), &mut state, &mut client).await.unwrap();
        assert!(follow_ups.iter().any(|a| matches!(a, Action::LoadStations)));
    }

    #[tokio::test]
    async fn set_view_now_playing_requests_waveform() {
        let (tx, _rx, mut state, mut client) = setup();

        let follow_ups = dispatch(&tx, Action::SetView(View::NowPlaying), &mut state, &mut client).await.unwrap();
        assert!(follow_ups.iter().any(|a| matches!(a, Action::LoadWaveform)));
        assert!(follow_ups.iter().any(|a| matches!(a, Action::LoadSpectrogram)));
    }

    #[tokio::test]
    async fn next_view_cycles_library_to_playlists() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;

        let follow_ups = dispatch(&tx, Action::NextView, &mut state, &mut client).await.unwrap();
        // Should request SetCategory(Playlists) as a follow-up
        assert!(follow_ups.iter().any(|a| matches!(a, Action::SetCategory(BrowseCategory::Playlists))));
    }

    #[tokio::test]
    async fn next_view_playlists_to_genres() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Playlists;

        let follow_ups = dispatch(&tx, Action::NextView, &mut state, &mut client).await.unwrap();
        assert!(follow_ups.iter().any(|a| matches!(a, Action::SetCategory(BrowseCategory::Genres))));
    }

    #[tokio::test]
    async fn next_view_queue_to_now_playing() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Queue;

        dispatch(&tx, Action::NextView, &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[tokio::test]
    async fn next_view_now_playing_to_library() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::NowPlaying;

        let follow_ups = dispatch(&tx, Action::NextView, &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Browse);
        assert!(follow_ups.iter().any(|a| matches!(a, Action::SetCategory(BrowseCategory::Library))));
    }

    #[tokio::test]
    async fn prev_view_library_to_now_playing() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;

        dispatch(&tx, Action::PrevView, &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[tokio::test]
    async fn prev_view_now_playing_to_queue() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::NowPlaying;

        dispatch(&tx, Action::PrevView, &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Queue);
    }

    #[tokio::test]
    async fn set_category_changes_and_resets_focus() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.focus = Focus::Right;

        dispatch(&tx, Action::SetCategory(BrowseCategory::Genres), &mut state, &mut client).await.unwrap();
        assert_eq!(state.browse_category, BrowseCategory::Genres);
        assert_eq!(state.focus, Focus::Left);
    }

    #[tokio::test]
    async fn set_category_same_is_noop() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.focus = Focus::Right; // keep right focus

        dispatch(&tx, Action::SetCategory(BrowseCategory::Library), &mut state, &mut client).await.unwrap();
        // Category didn't change, so focus should remain Right
        assert_eq!(state.focus, Focus::Right);
    }
}
