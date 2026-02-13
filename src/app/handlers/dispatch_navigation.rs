//! Navigation dispatch handlers: SetView, NextView, PrevView, NextMode, PrevMode,
//! SetCategory, ToggleFocus.

use crate::app::{Action, AppState, Event};
use crate::app::state::{BrowseCategory, Focus, GenreContentType, RightPanelMode, View};
use crate::api::PlexClient;

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

    // Deactivate inline filter on any view/category change
    let deactivates_filter = matches!(
        action,
        Action::SetView(_) | Action::NextView | Action::PrevView | Action::SetCategory(_)
    );
    if deactivates_filter && state.list_filter.active {
        state.list_filter.active = false;
        state.list_filter.query.clear();
        state.list_filter.results = None;
        state.list_filter.loading = false;
        state.list_filter.selected = 0;
    }

    match action {
        Action::SetView(view) => {
            state.view = view;
        }
        Action::NextView => {
            // Tab: cycle through nav bar views
            // Order: Library → Playlists → Genres → Radio → Folders → Now Playing → Library
            if state.view == View::NowPlaying {
                // From Now Playing, go to Library
                state.view = View::Browse;
                follow_ups.push(Action::SetCategory(BrowseCategory::Library));
            } else if state.view == View::Browse {
                // Cycle through browse categories, then to Now Playing
                match state.browse_category {
                    BrowseCategory::Library => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
                    }
                    BrowseCategory::Playlists => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Genres));
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Radio));
                    }
                    BrowseCategory::Radio => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Folders));
                    }
                    BrowseCategory::Folders => {
                        state.view = View::NowPlaying;
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.view = View::Browse;
            }
        }
        Action::PrevView => {
            // Shift+Tab: cycle backwards through nav bar views
            // Order: Library ← Playlists ← Genres ← Radio ← Folders ← Now Playing ← Library
            if state.view == View::NowPlaying {
                // From Now Playing, go to Folders
                state.view = View::Browse;
                follow_ups.push(Action::SetCategory(BrowseCategory::Folders));
            } else if state.view == View::Browse {
                // Cycle backwards through browse categories, or to Now Playing
                match state.browse_category {
                    BrowseCategory::Library => {
                        state.view = View::NowPlaying;
                    }
                    BrowseCategory::Playlists => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Library));
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
                    }
                    BrowseCategory::Radio => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Genres));
                    }
                    BrowseCategory::Folders => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Radio));
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.view = View::Browse;
            }
        }
        Action::NextMode => {
            // Shift+Down: cycle modes within current category
            if state.view == View::NowPlaying {
                state.now_playing_mode = state.now_playing_mode.next();
                follow_ups.push(Action::RefreshNowPlayingView);
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        state.artist_view_mode = state.artist_view_mode.next();
                        follow_ups.push(Action::RefreshArtistView);
                    }
                    BrowseCategory::Playlists => {
                        // Playlists has no modes to cycle
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::CycleGenreTab);
                    }
                    BrowseCategory::Radio => {
                        // Radio has no modes to cycle
                    }
                    BrowseCategory::Folders => {
                        // Folders has no modes to cycle
                    }
                }
            }
        }
        Action::PrevMode => {
            // Shift+Up: cycle modes backwards within current category
            if state.view == View::NowPlaying {
                state.now_playing_mode = state.now_playing_mode.prev();
                follow_ups.push(Action::RefreshNowPlayingView);
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        state.artist_view_mode = state.artist_view_mode.prev();
                        follow_ups.push(Action::RefreshArtistView);
                    }
                    BrowseCategory::Playlists => {
                        // Playlists has no modes to cycle
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::CycleGenreTab);
                    }
                    BrowseCategory::Radio => {
                        // Radio has no modes to cycle
                    }
                    BrowseCategory::Folders => {
                        // Folders has no modes to cycle
                    }
                }
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
                        }
                    }
                    BrowseCategory::Playlists => {
                        if state.playlists.is_empty() && !state.playlists_loading {
                            helpers::load_playlists(event_tx, state, client);
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
                    BrowseCategory::Radio => {
                        if state.station_nav.columns.is_empty() && !state.station_nav.loading {
                            follow_ups.push(Action::LoadStations);
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
