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
            // Tab: cycle through main views only
            // Order: Library → Playlists → Queue → Now Playing → Library
            // Genre/Folder categories are accessed via Ctrl+G / Ctrl+O, not Tab.
            if state.view == View::NowPlaying {
                // From Now Playing, go to Library
                state.set_view(View::Browse);
                follow_ups.push(Action::SetCategory(BrowseCategory::Library));
            } else if state.view == View::Queue {
                // From Queue, go to Now Playing
                state.set_view(View::NowPlaying);
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
                    }
                    _ => {
                        // From Playlists, Genres, or Folders → Queue
                        state.set_view(View::Queue);
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.set_view(View::Browse);
            }
        }
        Action::PrevView => {
            // Shift+Tab: cycle backwards through main views
            // Order: Library ← Playlists ← Queue ← Now Playing ← Library
            if state.view == View::NowPlaying {
                // From Now Playing, go to Queue
                state.set_view(View::Queue);
            } else if state.view == View::Queue {
                // From Queue, go to Playlists
                state.set_view(View::Browse);
                follow_ups.push(Action::SetCategory(BrowseCategory::Playlists));
            } else if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        state.set_view(View::NowPlaying);
                    }
                    _ => {
                        // From Playlists, Genres, or Folders → Library
                        follow_ups.push(Action::SetCategory(BrowseCategory::Library));
                    }
                }
            } else {
                // From other views (Help, Settings, Search, Similar), go to Browse
                state.set_view(View::Browse);
            }
        }
        Action::NextMode => {
            // Shift+Down: cycle modes within current category
            if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        // Library has no modes to cycle
                    }
                    BrowseCategory::Playlists => {
                        // Playlists has no modes to cycle
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::CycleGenreTab);
                    }
                    BrowseCategory::Folders => {
                        // Folders has no modes to cycle
                    }
                }
            }
        }
        Action::PrevMode => {
            // Shift+Up: cycle modes backwards within current category
            if state.view == View::Browse {
                match state.browse_category {
                    BrowseCategory::Library => {
                        // Library has no modes to cycle
                    }
                    BrowseCategory::Playlists => {
                        // Playlists has no modes to cycle
                    }
                    BrowseCategory::Genres => {
                        follow_ups.push(Action::CycleGenreTab);
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
