//! Shared playlist pagination and presentation, independent of transport.
use crate::app::action::SystemAction;
use crate::app::event::PlaylistEvent;
use crate::app::{Action, AppState};

pub fn handle(event: PlaylistEvent, state: &mut AppState) -> Vec<Action> {
    match event {
        PlaylistEvent::PlaylistTracksForMillerFailed {
            library_key,
            request_id,
            playlist_key,
            error,
        } => {
            if state.active_library.as_ref() != Some(&library_key)
                || state.playlist_nav_request_id != request_id
            {
                return vec![];
            }
            let selected_key = state
                .playlist_nav
                .columns
                .first()
                .and_then(|column| column.items.get(column.selected_index))
                .map(|item| item.key());
            if selected_key != Some(playlist_key.as_str()) {
                return vec![];
            }
            state.playlist_nav.loading = false;

            state.set_error(error.message);
            vec![]
        }

        // First page of a lazy-loaded playlist column. `LoadMorePlaylistTracks`
        // fetches subsequent pages as the user scrolls.
        PlaylistEvent::PlaylistFirstPageLoaded {
            library_key,
            request_id,
            playlist_key,
            tracks,
            total,
        } => {
            if state.active_library.as_ref() != Some(&library_key)
                || state.playlist_nav_request_id != request_id
            {
                return vec![];
            }
            // Race guard: drop replies for a playlist the user has
            // already navigated away from (mirrors the original
            // PlaylistTracksForMillerLoaded path).
            let selected_key: Option<String> = state
                .playlist_nav
                .columns
                .first()
                .and_then(|c| c.items.get(c.selected_index))
                .map(|item| item.key().to_string());
            if selected_key.as_deref() != Some(playlist_key.as_str()) {
                tracing::debug!(
                    "Playlist first-page reply for {} arrived after user navigated to {:?} — discarding",
                    playlist_key,
                    selected_key,
                );
                return vec![];
            }
            state.playlist_nav.loading = false;

            state.playlist_nav.focused_column = 0;

            let playlist_name = state
                .playlist_nav
                .focused()
                .and_then(|c| c.selected_item())
                .map(|item| item.title().to_string())
                .unwrap_or_default();
            // Header reports the SERVER total, not the partial in-memory
            // count, so the user knows what they'll eventually scroll to.
            let n_visible = tracks.len();
            let total_n = total.map(|t| t as usize).unwrap_or(n_visible);
            let count_str = if total_n == 1 {
                "1 track".to_string()
            } else if total_n > n_visible {
                format!("{} tracks", total_n)
            } else {
                format!("{} tracks", n_visible)
            };
            let title = if playlist_name.is_empty() {
                format!("tracks \u{2014} {}", count_str)
            } else {
                format!("{} \u{2014} {}", playlist_name, count_str)
            };
            let items = crate::app::state::BrowseItem::from_tracks(&tracks);
            let mut col = crate::app::state::BrowseColumn::new_with_tracks(title, items, tracks);
            col.play_all_row = Some(crate::app::state::PlayAllRow::Playlist {
                rating_key: playlist_key.clone(),
                title: playlist_name.clone(),
            });
            col.header_selection = crate::app::state::ColumnHeaderSelection::Play;
            // Grouping belongs to the playlist; artwork visibility belongs to the column type.
            let saved_view = state
                .active_library
                .as_ref()
                .and_then(|lib| state.playlist_views.get(lib))
                .and_then(|m| m.get(&playlist_key))
                .copied()
                .unwrap_or_default();
            state.initialize_column_artwork(&mut col);
            let artwork_visible = col.artwork_visible;
            // Mark the column as lazy so the GUI scroll handler knows
            // to ask for more pages as the user scrolls down.
            col.lazy = Some(crate::app::state::LazyPlaylist {
                key: playlist_key.clone(),
                total,
                loading: false,
            });
            // Apply group-by-album AFTER the column is wired up.
            // group_by_album resets selected_index to 0 and rebuilds
            // items, so we have to set the column up first.
            if saved_view.group_by_album {
                col.group_by_album();
            }
            state.playlist_nav.push_column(col);

            // If artwork was restored ON, kick a full-list art batch
            // so every album cover loads (mirrors `toggle_artwork`'s
            // ON-path). The col was just pushed; read it back to
            // collect art keys.
            let mut follow_ups: Vec<Action> = Vec::new();
            if artwork_visible {
                let batch = super::dispatch_miller::collect_all_art_to_load(
                    state.playlist_nav.columns.last(),
                    &state.artwork.grid_cache,
                    &state.artwork.grid_pending,
                );
                if !batch.is_empty() {
                    follow_ups.push(SystemAction::LoadAlbumArt(batch).into());
                }
            }
            // If grouping was restored AND there are more pages, the
            // user expects all the albums to show — kick the next
            // page fetch so pagination can chain to the end.
            if saved_view.group_by_album {
                let next = state.playlist_nav.columns.last().and_then(|col| {
                    let lazy = col.lazy.as_ref()?;
                    let total = lazy.total? as usize;
                    if !lazy.loading && col.tracks.len() < total {
                        Some((lazy.key.clone(), col.tracks.len() as u32))
                    } else {
                        None
                    }
                });
                if let Some((pk, off)) = next {
                    follow_ups.push(
                        crate::app::action::MillerAction::LoadMorePlaylistTracks {
                            playlist_key: pk,
                            offset: off,
                        }
                        .into(),
                    );
                }
            }
            follow_ups
        } // Subsequent page — append to the existing column.
    }
}
