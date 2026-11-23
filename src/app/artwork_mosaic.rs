//! Album-cover collages for aggregate library rows.
use crate::app::{
    AppState,
    state::{AllTracksScope, BrowseItem},
};
use std::collections::HashSet;

pub fn key(item: &BrowseItem) -> Option<String> {
    Some(match item {
        BrowseItem::AllTracks {
            scope: AllTracksScope::Artist { artist_key, .. },
            ..
        } => format!("mosaic:artist:{artist_key}"),
        BrowseItem::AllTracks {
            scope: AllTracksScope::CompilationsByArtist { artist_key, .. },
            ..
        }
        | BrowseItem::CompilationTracks { artist_key, .. } => {
            format!("mosaic:compilations:{artist_key}")
        }
        BrowseItem::AllTracks {
            scope: AllTracksScope::Library,
            ..
        } => "mosaic:library".into(),
        BrowseItem::AllTracks {
            scope: AllTracksScope::AllCompilations,
            ..
        }
        | BrowseItem::Compilations => "mosaic:compilations".into(),
        _ => return None,
    })
}
pub fn requests(state: &AppState) -> Vec<(String, String)> {
    let Some(nav) = state.browse_nav() else {
        return vec![];
    };
    let mut keys = HashSet::new();
    let mut result = vec![];
    for item in nav
        .columns
        .iter()
        .filter(|column| column.artwork_visible)
        .flat_map(|column| &column.items)
    {
        let Some(key) = key(item) else { continue };
        if state.artwork.grid_cache.contains_key(&key)
            || state.artwork.grid_pending.contains(&key)
            || !keys.insert(key.clone())
        {
            continue;
        }
        let (albums, artist, compilations) = match item {
            BrowseItem::AllTracks {
                scope: AllTracksScope::Artist { artist_key, .. },
                ..
            } => (state.library.albums.as_ref(), Some(artist_key), false),
            BrowseItem::AllTracks {
                scope: AllTracksScope::CompilationsByArtist { artist_key, .. },
                ..
            }
            | BrowseItem::CompilationTracks { artist_key, .. } => {
                (&state.library.compilations.albums, Some(artist_key), true)
            }
            BrowseItem::AllTracks {
                scope: AllTracksScope::Library,
                ..
            } => (state.library.albums.as_ref(), None, false),
            _ => (&state.library.compilations.albums, None, true),
        };
        let mut seen = HashSet::new();
        let thumbs: Vec<_> = albums
            .iter()
            .filter(|album| {
                artist.is_none_or(|artist| {
                    if compilations {
                        state
                            .library
                            .compilations
                            .artist_map
                            .get(artist)
                            .is_some_and(|keys| keys.contains(&album.rating_key))
                    } else {
                        album.parent_rating_key.as_ref() == Some(artist)
                            || state
                                .library
                                .compilations
                                .single_artist
                                .get(artist)
                                .is_some_and(|albums| {
                                    albums
                                        .iter()
                                        .any(|candidate| candidate.rating_key == album.rating_key)
                                })
                    }
                })
            })
            .filter_map(|album| album.thumb.as_ref())
            .filter(|thumb| !thumb.is_empty() && seen.insert((*thumb).clone()))
            .take(9)
            .cloned()
            .collect();
        if !thumbs.is_empty() {
            result.push((
                key,
                format!(
                    "mosaic:{}",
                    serde_json::to_string(&thumbs).expect("string list")
                ),
            ));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_art_never_shares_an_artist_portrait_key() {
        let artist = BrowseItem::ArtistInfo {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
            thumb: None,
        };
        let all = BrowseItem::AllTracks {
            scope: AllTracksScope::Artist {
                artist_key: "artist".into(),
                artist_name: "Artist".into(),
            },
            thumb: None,
        };
        let comps = BrowseItem::CompilationTracks {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
        };
        assert!(key(&artist).is_none());
        assert_ne!(key(&all), key(&comps));
        assert_ne!(key(&all).as_deref(), Some(artist.key()));
    }
}
