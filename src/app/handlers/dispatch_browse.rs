//! Browse dispatch handlers: LoadStations, LoadTagList, LoadTagAlbums,
//! RefreshTagView, OpenTrackDetails, OpenInLibrary.

use crate::app::action::{BrowseAction, MillerAction, SystemAction};
use crate::app::state::{BrowseCategory, BrowseColumn, BrowseItem, RefreshCategory, StationColumn};
use crate::app::{Action, AppState, Event};

use anyhow::Result;

use super::helpers;
use tokio::sync::mpsc;

/// Dispatch browse-related actions. Returns follow-up actions.
pub async fn dispatch(
    _event_tx: &mpsc::Sender<Event>,
    action: BrowseAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        BrowseAction::StationsLoaded {
            library_key,
            result,
        } => {
            if state.active_library.as_ref() != Some(&library_key) {
                return Ok(follow_ups);
            }
            state.stations_loading = false;
            state.station_nav.loading = false;
            match result {
                Ok(mut stations) => {
                    helpers::append_station_action_items(
                        &mut stations,
                        state.queue.shuffle_undo_queue.is_some(),
                    );
                    state.station_nav.columns.clear();
                    state.station_nav.columns.push(StationColumn::new(
                        None,
                        "stations".to_string(),
                        stations.clone(),
                    ));
                    state.station_nav.focused_column = 0;
                    state.stations = stations;
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
        }

        BrowseAction::TagListLoaded {
            library_key,
            section,
            result,
        } => {
            if state.active_library.as_ref() != Some(&library_key) {
                return Ok(follow_ups);
            }
            set_tag_loading(state, section, false);
            match result {
                Ok(items) => {
                    store_tag_list(state, section, items);
                    if state.browse_category == section {
                        follow_ups.push(BrowseAction::RefreshTagView.into());
                    } else {
                        state.tag_nav.loading = false;
                    }
                }
                Err(error) => {
                    state.set_error(error.message);
                    state.tag_nav.loading = false;
                }
            }
        }

        BrowseAction::TagAlbumsLoaded {
            library_key,
            section,
            tag_key,
            tag_title,
            replace_child,
            result,
        } => {
            let selected_tag_matches = state
                .tag_nav
                .columns
                .first()
                .and_then(|column| column.items.get(column.selected_index))
                .is_some_and(
                    |item| matches!(item, BrowseItem::Genre { key, .. } if key == &tag_key),
                );
            if state.active_library.as_ref() != Some(&library_key)
                || state.browse_category != section
                || !selected_tag_matches
            {
                return Ok(follow_ups);
            }
            state.library.right_panel_loading = false;
            match result {
                Ok(mut albums) => {
                    albums.sort_by_cached_key(|album| {
                        album.parent_title.as_deref().unwrap_or("").to_lowercase()
                    });
                    state.library.tag_albums = albums.clone();
                    let items =
                        BrowseItem::from_albums(&albums, &state.library.album_display_artist);
                    let col = BrowseColumn::new(tag_title, items);
                    state.tag_nav.drill_column(col, replace_child);
                }
                Err(error) => {
                    state.set_error(error.message);
                }
            }
        }
        BrowseAction::RefreshTagView => {
            // Populate column 0 of tag_nav with the active section's tag list.
            let section = state.browse_category;
            if !section.is_tag_section() {
                return Ok(follow_ups);
            }
            let list_empty = state.tag_list_for(section).is_empty();
            if list_empty && !is_tag_loading(state, section) {
                follow_ups.push(BrowseAction::LoadTagList(section).into());
                state.tag_nav.loading = true;
                return Ok(follow_ups);
            }
            let items = BrowseItem::from_genres(state.tag_list_for(section));
            state.tag_nav.reset(section.name(), items);
            state.tag_nav.loading = false;
        }
        BrowseAction::OpenTrackDetails => {
            // The pane is a passive viewer of `focused_track()` — it
            // opens but does NOT steal keyboard focus. The user keeps
            // arrow-keying in the tracks column; the pane content
            // updates automatically as the focused track changes.
            //
            // To act on the pane (play, navigate similar tracks),
            // the user explicitly moves focus to it via Right when
            // they're on a Track row that already has the pane open
            // — see the per-category Miller-column handlers. This
            // satisfies the "Up/Down stays in one column, Left/Right
            // crosses columns" rule.
            if let Some(track) = state.focused_track() {
                if let Some(album_key) = track.parent_rating_key.clone() {
                    if !state.artwork.grid_cache.contains_key(&album_key)
                        && !state.artwork.grid_pending.contains(&album_key)
                    {
                        if let Some(thumb) = track.parent_thumb.clone() {
                            follow_ups
                                .push(SystemAction::LoadAlbumArt(vec![(album_key, thumb)]).into());
                        }
                    }
                }
            }
            state.track_pane_open = true;
        }
        BrowseAction::CloseTrackDetails => {
            state.track_pane_open = false;
        }
        BrowseAction::OpenInLibrary {
            artist_key,
            artist_name,
            album_key,
            album_title,
        } => {
            state.track_pane_open = false;
            if let Some(ak) = album_key {
                state.search.pending_album_key = Some(ak);
            }
            if let Some(at) = album_title {
                state.library.selected_album_title = at;
            }
            state.library.selected_artist_name = artist_name;
            state.set_view(crate::app::state::View::Browse);
            state.set_browse_category(crate::app::state::BrowseCategory::Library, false);

            if state.artist_nav.columns.is_empty() {
                let items = state.build_artist_root_items();
                let title = format!("artists ({})", state.library.artists.len());
                state
                    .artist_nav
                    .columns
                    .push(BrowseColumn::new(title, items));
            } else {
                state.artist_nav.columns.truncate(1);
            }
            state.artist_nav.focused_column = 0;

            if let Some(col) = state.artist_nav.columns.first_mut() {
                if let Some(idx) = col.items.iter().position(
                    |item| matches!(item, BrowseItem::Artist { key, .. } if key == &artist_key),
                ) {
                    col.selected_index = idx;
                }
            }

            if let Some(idx) = state
                .library
                .artists
                .iter()
                .position(|a| a.rating_key == artist_key)
            {
                state.list_state.artists_index = idx;
            }

            follow_ups.push(
                MillerAction::LoadArtistAlbumsForMiller {
                    artist_key,
                    replace_child: false,
                }
                .into(),
            );
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(follow_ups)
}

fn set_tag_loading(state: &mut AppState, section: BrowseCategory, val: bool) {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres_loading = val,
        BrowseCategory::ArtistGenres => state.library.artist_genres_loading = val,
        BrowseCategory::Moods => state.library.moods_loading = val,
        BrowseCategory::Styles => state.library.styles_loading = val,
        BrowseCategory::Decades => state.library.decades_loading = val,
        BrowseCategory::Years => state.library.years_loading = val,
        BrowseCategory::Collections => state.library.collections_loading = val,
        BrowseCategory::Countries => state.library.countries_loading = val,
        BrowseCategory::Labels => state.library.labels_loading = val,
        BrowseCategory::Formats => state.library.formats_loading = val,
        BrowseCategory::Studios => state.library.studios_loading = val,
        _ => {}
    }
}

fn is_tag_loading(state: &AppState, section: BrowseCategory) -> bool {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres_loading,
        BrowseCategory::ArtistGenres => state.library.artist_genres_loading,
        BrowseCategory::Moods => state.library.moods_loading,
        BrowseCategory::Styles => state.library.styles_loading,
        BrowseCategory::Decades => state.library.decades_loading,
        BrowseCategory::Years => state.library.years_loading,
        BrowseCategory::Collections => state.library.collections_loading,
        BrowseCategory::Countries => state.library.countries_loading,
        BrowseCategory::Labels => state.library.labels_loading,
        BrowseCategory::Formats => state.library.formats_loading,
        BrowseCategory::Studios => state.library.studios_loading,
        _ => false,
    }
}

fn store_tag_list(
    state: &mut AppState,
    section: BrowseCategory,
    items: Vec<crate::library::models::Genre>,
) {
    match section {
        BrowseCategory::AlbumGenres => state.library.album_genres = items,
        BrowseCategory::ArtistGenres => state.library.artist_genres = items,
        BrowseCategory::Moods => state.library.moods = items,
        BrowseCategory::Styles => state.library.styles = items,
        BrowseCategory::Decades => state.library.decades = items,
        BrowseCategory::Years => state.library.years = items,
        BrowseCategory::Collections => state.library.collections = items,
        BrowseCategory::Countries => state.library.countries = items,
        BrowseCategory::Labels => state.library.labels = items,
        BrowseCategory::Formats => state.library.formats = items,
        BrowseCategory::Studios => state.library.studios = items,
        _ => {}
    }
}

// Helper exposed for other dispatch handlers that need to know if the
// currently-active tag section's data is already loaded.
pub fn current_tag_section_refresh(_state: &AppState) -> Option<RefreshCategory> {
    RefreshCategory::for_tag_section(_state.browse_category)
}
