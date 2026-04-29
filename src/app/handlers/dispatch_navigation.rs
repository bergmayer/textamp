//! Navigation dispatch handlers: SetView, NextView, PrevView, NextMode, PrevMode,
//! SetCategory, ToggleFocus.

use crate::app::{Action, AppState, Event};
use crate::app::action::{NavigationAction, BrowseAction, FolderAction, SystemAction};
use crate::app::state::{BrowseCategory, Focus, GenreContentType, RightPanelMode, View};
use crate::plex::PlexClient;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch navigation actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    action: NavigationAction,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    // Deactivate inline filter on view or category change
    if matches!(action, NavigationAction::SetView(_) | NavigationAction::SetCategory(_)) && state.list_filter.active {
        state.list_filter.deactivate();
    }

    match action {
        NavigationAction::SetView(view) => {
            // Clear artwork cache when leaving Similar view to force re-render
            // (Similar popup's Clear widget can corrupt terminal images)
            #[cfg(feature = "tui")]
            if state.view == View::Similar {
                crate::ui::screens::now_playing::clear_artwork_cache();
            }
            state.set_view(view);
            // Load stations when entering Queue view if not already loaded
            if view == View::Queue
                && state.station_nav.columns.is_empty()
                && !state.station_nav.loading
            {
                follow_ups.push(BrowseAction::LoadStations.into());
            }
            // Load waveform and spectrogram when entering NowPlaying view
            if view == View::NowPlaying {
                follow_ups.push(SystemAction::LoadWaveform.into());
                follow_ups.push(SystemAction::LoadSpectrogram.into());
            }
        }
        NavigationAction::NextView => {
            // Tab: cycle through top-level views
            // Order: Browse → Queue → Now Playing → Browse
            if state.view == View::NowPlaying {
                state.set_view(View::Browse);
            } else if state.view == View::Queue {
                state.set_view(View::NowPlaying);
            } else if state.view == View::Browse {
                state.set_view(View::Queue);
            } else {
                state.set_view(View::Browse);
            }
        }
        NavigationAction::PrevView => {
            // Shift+Tab: cycle backwards through top-level views
            // Order: Browse ← Queue ← Now Playing ← Browse
            if state.view == View::NowPlaying {
                state.set_view(View::Queue);
            } else if state.view == View::Queue {
                state.set_view(View::Browse);
            } else if state.view == View::Browse {
                state.set_view(View::NowPlaying);
            } else {
                state.set_view(View::Browse);
            }
        }
        NavigationAction::SetCategory(category) => {
            // Picking a category implies "show me that part of the
            // library", so always switch to the Browse view too.
            // Without this, the View menu / Ctrl+L|P|G|O shortcuts
            // would silently change the category state but leave the
            // user stuck on Queue/Now Playing/Help/etc.
            if state.view != View::Browse {
                state.set_view(View::Browse);
            }

            // Always unfocus category column when a category is selected
            state.category_column_focused = false;

            if state.browse_category != category {
                // Unshuffle Library root when leaving, so "All Artists" is in correct position
                if state.browse_category == BrowseCategory::Library {
                    if let Some(col) = state.artist_nav.columns.first_mut() {
                        if col.is_shuffled() {
                            col.unshuffle();
                        }
                    }
                }
                state.set_browse_category(category);
                state.focus = Focus::Left;
                // Clear right panel
                state.library.right_panel_mode = RightPanelMode::Empty;
                state.library.selected_artist_albums.clear();
                state.library.selected_album_tracks.clear();

                // Load category data if needed (and not already loading)
                match category {
                    BrowseCategory::Library => {
                        if state.library.artists.is_empty() && !state.library.artists_loading {
                            helpers::load_artists(event_tx, state, client);
                        }
                    }
                    BrowseCategory::Playlists => {
                        if state.library.playlists.is_empty() && !state.library.playlists_loading {
                            helpers::load_playlists(event_tx, state, client);
                        } else {
                            // Rebuild root column from state.library.playlists to ensure it's populated
                            // (guards against stale/empty nav from preload race conditions)
                            let items = crate::app::state::BrowseItem::from_playlists(&state.library.playlists);
                            state.playlist_nav.reset("playlists", items);
                        }
                    }
                    BrowseCategory::Genres => {
                        if state.genre_tab == crate::app::state::GenreTab::All {
                            follow_ups.push(BrowseAction::RefreshGenreView.into());
                        } else {
                            // Load the appropriate content based on current genre content type
                            match state.library.genre_content_type {
                                GenreContentType::Genres => {
                                    if state.library.genres.is_empty() && !state.library.genres_loading {
                                        follow_ups.push(BrowseAction::LoadGenres.into());
                                    }
                                }
                                GenreContentType::ArtistGenres => {
                                    if state.library.artist_genres.is_empty() && !state.library.artist_genres_loading {
                                        follow_ups.push(BrowseAction::LoadArtistGenres.into());
                                    }
                                }
                                GenreContentType::AlbumGenres => {
                                    if state.library.album_genres.is_empty() && !state.library.album_genres_loading {
                                        follow_ups.push(BrowseAction::LoadAlbumGenres.into());
                                    }
                                }
                                GenreContentType::Moods => {
                                    if state.library.moods.is_empty() && !state.library.moods_loading {
                                        follow_ups.push(BrowseAction::LoadMoods.into());
                                    }
                                }
                                GenreContentType::Styles => {
                                    if state.library.styles.is_empty() && !state.library.styles_loading {
                                        follow_ups.push(BrowseAction::LoadStyles.into());
                                    }
                                }
                            }
                        }
                    }
                    BrowseCategory::Folders => {
                        if state.folder_state.is_none() {
                            follow_ups.push(FolderAction::LoadFolderRoot.into());
                        }
                    }
                }
            }
        }
        NavigationAction::ToggleFocus => {
            state.focus = match state.focus {
                Focus::Left => Focus::Right,
                Focus::Right => Focus::Left,
            };
        }
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

        dispatch(&tx, NavigationAction::ToggleFocus.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.focus, Focus::Right);
    }

    #[tokio::test]
    async fn toggle_focus_right_to_left() {
        let (tx, _rx, mut state, mut client) = setup();
        state.focus = Focus::Right;

        dispatch(&tx, NavigationAction::ToggleFocus.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.focus, Focus::Left);
    }

    #[tokio::test]
    async fn set_view_changes_state() {
        let (tx, _rx, mut state, mut client) = setup();

        dispatch(&tx, NavigationAction::SetView(View::Queue).into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Queue);

        dispatch(&tx, NavigationAction::SetView(View::Help).into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Help);
    }

    #[tokio::test]
    async fn set_view_queue_requests_load_stations() {
        let (tx, _rx, mut state, mut client) = setup();

        let follow_ups = dispatch(&tx, NavigationAction::SetView(View::Queue).into(), &mut state, &mut client).await.unwrap();
        assert!(follow_ups.iter().any(|a| matches!(a, Action::Browse(BrowseAction::LoadStations))));
    }

    #[tokio::test]
    async fn set_view_now_playing_requests_waveform() {
        let (tx, _rx, mut state, mut client) = setup();

        let follow_ups = dispatch(&tx, NavigationAction::SetView(View::NowPlaying).into(), &mut state, &mut client).await.unwrap();
        assert!(follow_ups.iter().any(|a| matches!(a, Action::System(SystemAction::LoadWaveform))));
        assert!(follow_ups.iter().any(|a| matches!(a, Action::System(SystemAction::LoadSpectrogram))));
    }

    #[tokio::test]
    async fn next_view_browse_to_queue() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library);

        dispatch(&tx, NavigationAction::NextView.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Queue);
    }

    #[tokio::test]
    async fn next_view_queue_to_now_playing() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Queue;

        dispatch(&tx, NavigationAction::NextView.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[tokio::test]
    async fn next_view_now_playing_to_browse() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::NowPlaying;

        dispatch(&tx, NavigationAction::NextView.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Browse);
    }

    #[tokio::test]
    async fn prev_view_library_to_now_playing() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library);

        dispatch(&tx, NavigationAction::PrevView.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[tokio::test]
    async fn prev_view_now_playing_to_queue() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::NowPlaying;

        dispatch(&tx, NavigationAction::PrevView.into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.view, View::Queue);
    }

    #[tokio::test]
    async fn set_category_changes_and_resets_focus() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library);
        state.focus = Focus::Right;

        dispatch(&tx, NavigationAction::SetCategory(BrowseCategory::Genres).into(), &mut state, &mut client).await.unwrap();
        assert_eq!(state.browse_category, BrowseCategory::Genres);
        assert_eq!(state.focus, Focus::Left);
    }

    #[tokio::test]
    async fn set_category_same_is_noop() {
        let (tx, _rx, mut state, mut client) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library);
        state.focus = Focus::Right; // keep right focus

        dispatch(&tx, NavigationAction::SetCategory(BrowseCategory::Library).into(), &mut state, &mut client).await.unwrap();
        // Category didn't change, so focus should remain Right
        assert_eq!(state.focus, Focus::Right);
    }
}
