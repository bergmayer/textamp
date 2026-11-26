//! Cheap immutable snapshots of the library-scoped image cache.
use crate::app::AppState;
use std::{collections::HashMap, sync::Arc};
pub type Images = Arc<HashMap<String, Arc<Vec<u8>>>>;
#[derive(Default)]
pub struct Cache {
    identity: Option<(u64, u64)>,
    images: Images,
}
impl Cache {
    pub fn project(&mut self, state: &AppState) -> Images {
        let identity = (state.library_generation, state.artwork.grid_revision);
        if self.identity != Some(identity) {
            let images = state
                .artwork
                .grid_cache
                .iter()
                .map(|(key, bytes)| {
                    let data = self
                        .images
                        .get(key)
                        .filter(|old| old.as_slice() == bytes.as_slice())
                        .cloned()
                        .unwrap_or_else(|| Arc::new(bytes.clone()));
                    (key.clone(), data)
                })
                .collect();
            self.images = Arc::new(images);
            self.identity = Some(identity);
        }
        self.images.clone()
    }
}

/// Preview art is independent of the row-cover visibility setting.
pub fn request(state: &AppState) -> Option<(String, String)> {
    use crate::app::state::{BrowseCategory, BrowseItem};
    if let Some(column) = state
        .browse_nav()
        .and_then(|n| n.focused())
        .filter(|_| crate::app::details::album_info_selection(state).is_some())
    {
        return crate::app::details::album_cover(state, column);
    }
    let track = if state.browse_category == BrowseCategory::Folders {
        state
            .folder_state
            .as_ref()
            .and_then(|nav| nav.focused())
            .and_then(|column| {
                column
                    .items
                    .get(column.selected_index)
                    .and_then(|item| column.tracks.get(&item.key))
            })
    } else {
        crate::app::sonic_details::selected_track(state).or_else(|| state.focused_track())
    };
    if let Some(track) = track {
        if let Some(thumb) = track.thumb.as_ref().filter(|thumb| !thumb.is_empty()) {
            return Some((track.rating_key.clone(), thumb.clone()));
        }
        let album = track.parent_rating_key.as_ref().and_then(|key| {
            state
                .library
                .albums
                .iter()
                .find(|album| &album.rating_key == key)
        });
        let thumb = track
            .parent_thumb
            .as_ref()
            .filter(|thumb| !thumb.is_empty())
            .or_else(|| {
                album
                    .and_then(|album| album.thumb.as_ref())
                    .filter(|thumb| !thumb.is_empty())
            });
        return thumb.map(|thumb| {
            (
                track
                    .parent_rating_key
                    .clone()
                    .unwrap_or_else(|| track.rating_key.clone()),
                thumb.clone(),
            )
        });
    }
    let column = state.browse_nav()?.focused()?;
    match column.items.get(column.selected_index) {
        Some(BrowseItem::Album {
            key,
            thumb: Some(thumb),
            ..
        }) => Some((key.clone(), thumb.clone())),
        _ => None,
    }
}
pub fn preview(state: &AppState) -> Option<Arc<Vec<u8>>> {
    if let Some(index) = state.artist_info_column_index()
        && let Some(crate::app::state::BrowseDetails::Artist(info)) = state
            .browse_nav()
            .and_then(|nav| nav.columns.get(index))
            .and_then(|column| column.details.as_ref())
        && let Some(photo) = info.document.images.get(info.image_index)
    {
        return Some(Arc::new(photo.data.clone()));
    }
    if let Some(track) =
        crate::app::sonic_details::selected_track(state).or_else(|| state.pane_track())
    {
        crate::app::details::track_cover(state, track)?;
        return state
            .artwork
            .grid_cache
            .get(&track.rating_key)
            .cloned()
            .map(Arc::new);
    }
    let request = request(state);
    request
        .as_ref()
        .and_then(|(key, _)| state.artwork.grid_cache.get(key))
        .or_else(|| {
            state
                .focused_track()
                .and_then(|track| track.parent_rating_key.as_ref())
                .and_then(|key| state.artwork.grid_cache.get(key))
        })
        .or_else(|| {
            state
                .browse_nav()
                .and_then(|nav| nav.focused())
                .and_then(|column| column.detail_art_key())
                .and_then(|key| state.artwork.grid_cache.get(key))
        })
        .map(|bytes| Arc::new(bytes.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::state::{BrowseColumn, BrowseItem},
        library::models::{Album, Track},
    };
    #[test]
    fn preview_falls_back_to_album_art_without_enabling_row_covers() {
        let mut state = AppState::new();
        let mut track = Track {
            rating_key: "track".into(),
            parent_rating_key: Some("album".into()),
            ..Default::default()
        };
        state.library.albums = std::sync::Arc::new(vec![Album {
            rating_key: "album".into(),
            thumb: Some("album-cover".into()),
            ..Default::default()
        }]);
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "tracks",
            BrowseItem::from_tracks(&[track.clone()]),
            vec![track.clone()],
        )];
        assert_eq!(
            request(&state),
            Some(("album".into(), "album-cover".into()))
        );
        state
            .artwork
            .grid_cache
            .insert("album".into(), vec![1, 2, 3]);
        assert_eq!(preview(&state), Some(std::sync::Arc::new(vec![1, 2, 3])));
        track.thumb = Some("individual-cover".into());
        state.artist_nav.columns[0].tracks = vec![track];
        assert_eq!(
            request(&state),
            Some(("track".into(), "individual-cover".into()))
        );
        assert_eq!(preview(&state), Some(std::sync::Arc::new(vec![1, 2, 3])));
        state.artwork.grid_cache.insert("track".into(), vec![4]);
        assert_eq!(preview(&state), Some(std::sync::Arc::new(vec![4])));
        assert!(!state.artist_nav.columns[0].artwork_visible);
    }
}
