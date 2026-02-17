//! Search and filter service for library content.
//!
//! Provides pure functions for filtering library items based on search queries.
//! Supports both API search results and local filtering.
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and can be used with any frontend.
//! All functions are pure and have no side effects.

use crate::plex::models::{Album, Artist, Genre, Playlist, SearchResults, Track};

/// A filtered item with display title and key for selection.
#[derive(Debug, Clone)]
pub struct FilteredItem {
    /// Display title shown to the user.
    pub title: String,
    /// Unique key for this item (rating_key or similar).
    pub key: String,
}

impl FilteredItem {
    /// Create a new filtered item.
    pub fn new(title: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            key: key.into(),
        }
    }

    /// Convert to a tuple for backward compatibility.
    pub fn as_tuple(&self) -> (String, String) {
        (self.title.clone(), self.key.clone())
    }
}

impl From<FilteredItem> for (String, String) {
    fn from(item: FilteredItem) -> Self {
        (item.title, item.key)
    }
}

/// Service for filtering library content.
pub struct SearchFilterService;

impl SearchFilterService {
    /// Filter artists by query.
    ///
    /// If `api_results` is provided, uses those. Otherwise filters `local_artists`.
    pub fn filter_artists(
        query: &str,
        api_results: Option<&SearchResults>,
        local_artists: &[Artist],
    ) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            results
                .artists
                .iter()
                .map(|a| FilteredItem::new(&a.title, &a.rating_key))
                .collect()
        } else {
            local_artists
                .iter()
                .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query_lower))
                .map(|a| FilteredItem::new(&a.title, &a.rating_key))
                .collect()
        }
    }

    /// Filter album artists by query.
    ///
    /// Extracts unique album artists from albums and filters them.
    /// Always uses local albums since API doesn't return album artists.
    pub fn filter_album_artists(query: &str, albums: &[Album]) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        let mut artists: Vec<FilteredItem> = albums
            .iter()
            .filter_map(|a| {
                let artist = a.parent_title.as_deref().unwrap_or("");
                if !artist.is_empty()
                    && (query.is_empty() || artist.to_lowercase().contains(&query_lower))
                {
                    Some(FilteredItem::new(artist, &a.rating_key))
                } else {
                    None
                }
            })
            .collect();

        // Deduplicate by artist name (case-insensitive), keeping first occurrence
        artists.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        artists.dedup_by(|a, b| a.title.to_lowercase() == b.title.to_lowercase());

        artists
    }

    /// Filter albums by query.
    ///
    /// If `api_results` is provided, uses those supplemented with local year matches.
    /// Otherwise filters `local_albums` by title or year.
    pub fn filter_albums(
        query: &str,
        api_results: Option<&SearchResults>,
        local_albums: &[Album],
    ) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            let mut items: Vec<FilteredItem> = results
                .albums
                .iter()
                .map(|a| FilteredItem::new(Self::format_album(a), &a.rating_key))
                .collect();

            // Supplement with local albums matching by year (Plex API doesn't search by year)
            let existing_keys: std::collections::HashSet<&str> =
                results.albums.iter().map(|a| a.rating_key.as_str()).collect();
            for a in local_albums {
                if !existing_keys.contains(a.rating_key.as_str())
                    && Self::album_year_matches(a, &query_lower)
                {
                    items.push(FilteredItem::new(Self::format_album(a), &a.rating_key));
                }
            }

            items
        } else {
            local_albums
                .iter()
                .filter(|a| {
                    query.is_empty()
                        || a.title.to_lowercase().contains(&query_lower)
                        || Self::album_year_matches(a, &query_lower)
                })
                .map(|a| FilteredItem::new(Self::format_album(a), &a.rating_key))
                .collect()
        }
    }

    /// Filter playlists by query.
    ///
    /// Always uses local filtering since API search doesn't return playlists.
    pub fn filter_playlists(query: &str, playlists: &[Playlist]) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        playlists
            .iter()
            .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query_lower))
            .map(|p| FilteredItem::new(&p.title, &p.rating_key))
            .collect()
    }

    /// Filter tracks by query.
    ///
    /// If `api_results` is provided, uses those. Otherwise filters `local_tracks`.
    pub fn filter_tracks(
        query: &str,
        api_results: Option<&SearchResults>,
        local_tracks: &[Track],
    ) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            results
                .tracks
                .iter()
                .map(|t| {
                    FilteredItem::new(
                        format!("{} - {}", t.title, t.track_artist()),
                        &t.rating_key,
                    )
                })
                .collect()
        } else {
            local_tracks
                .iter()
                .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query_lower))
                .map(|t| {
                    FilteredItem::new(
                        format!("{} - {}", t.title, t.track_artist()),
                        &t.rating_key,
                    )
                })
                .collect()
        }
    }

    /// Filter genres by query.
    ///
    /// Always uses local filtering.
    pub fn filter_genres(query: &str, genres: &[Genre]) -> Vec<FilteredItem> {
        let query_lower = query.to_lowercase();

        genres
            .iter()
            .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query_lower))
            .map(|g| FilteredItem::new(&g.title, g.effective_key().to_string()))
            .collect()
    }

    /// Count filtered items for a given category without allocating the full result.
    ///
    /// Useful for navigation bounds checking.
    pub fn count_filtered_artists(
        query: &str,
        api_results: Option<&SearchResults>,
        local_artists: &[Artist],
    ) -> usize {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            results.artists.len()
        } else {
            local_artists
                .iter()
                .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query_lower))
                .count()
        }
    }

    /// Count filtered album artists.
    pub fn count_filtered_album_artists(query: &str, albums: &[Album]) -> usize {
        // We need to deduplicate to get accurate count
        Self::filter_album_artists(query, albums).len()
    }

    /// Count filtered albums.
    pub fn count_filtered_albums(
        query: &str,
        api_results: Option<&SearchResults>,
        local_albums: &[Album],
    ) -> usize {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            // Count API results + local year matches (deduplicated)
            let existing_keys: std::collections::HashSet<&str> =
                results.albums.iter().map(|a| a.rating_key.as_str()).collect();
            let year_matches = local_albums.iter()
                .filter(|a| !existing_keys.contains(a.rating_key.as_str())
                    && Self::album_year_matches(a, &query_lower))
                .count();
            results.albums.len() + year_matches
        } else {
            local_albums
                .iter()
                .filter(|a| {
                    query.is_empty()
                        || a.title.to_lowercase().contains(&query_lower)
                        || Self::album_year_matches(a, &query_lower)
                })
                .count()
        }
    }

    /// Count filtered playlists.
    pub fn count_filtered_playlists(query: &str, playlists: &[Playlist]) -> usize {
        let query_lower = query.to_lowercase();

        playlists
            .iter()
            .filter(|p| query.is_empty() || p.title.to_lowercase().contains(&query_lower))
            .count()
    }

    /// Count filtered tracks.
    pub fn count_filtered_tracks(
        query: &str,
        api_results: Option<&SearchResults>,
        local_tracks: &[Track],
    ) -> usize {
        let query_lower = query.to_lowercase();

        if let Some(results) = api_results {
            results.tracks.len()
        } else {
            local_tracks
                .iter()
                .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query_lower))
                .count()
        }
    }

    /// Count filtered genres.
    pub fn count_filtered_genres(query: &str, genres: &[Genre]) -> usize {
        let query_lower = query.to_lowercase();

        genres
            .iter()
            .filter(|g| query.is_empty() || g.title.to_lowercase().contains(&query_lower))
            .count()
    }

    /// Format an album for display: "Title (Year) - Artist" or "Title - Artist" if no year.
    fn format_album(album: &Album) -> String {
        let artist = album.artist_name();
        if let Some(year) = album.year {
            format!("{} ({}) - {}", album.title, year, artist)
        } else {
            format!("{} - {}", album.title, artist)
        }
    }

    /// Check if an album's year matches the query string.
    fn album_year_matches(album: &Album, query_lower: &str) -> bool {
        if let Some(year) = album.year {
            year.to_string().contains(query_lower)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artist(title: &str, key: &str) -> Artist {
        Artist {
            title: title.to_string(),
            rating_key: key.to_string(),
            ..Default::default()
        }
    }

    fn make_album(title: &str, key: &str, artist: Option<&str>) -> Album {
        Album {
            title: title.to_string(),
            rating_key: key.to_string(),
            parent_title: artist.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn make_genre(title: &str, key: &str) -> Genre {
        Genre {
            title: title.to_string(),
            key: key.to_string(),
            tag: None,
            fast_key: None,
            filter: None,
            count: None,
            id: None,
            rating_key: None,
        }
    }

    #[test]
    fn test_filter_artists_local() {
        let artists = vec![
            make_artist("Alice", "1"),
            make_artist("Bob", "2"),
            make_artist("Charlie", "3"),
        ];

        let results = SearchFilterService::filter_artists("", None, &artists);
        assert_eq!(results.len(), 3);

        let results = SearchFilterService::filter_artists("ali", None, &artists);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Alice");

        let results = SearchFilterService::filter_artists("xyz", None, &artists);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_filter_album_artists_dedup() {
        let albums = vec![
            make_album("Album 1", "1", Some("Artist A")),
            make_album("Album 2", "2", Some("Artist A")), // Duplicate artist
            make_album("Album 3", "3", Some("Artist B")),
        ];

        let results = SearchFilterService::filter_album_artists("", &albums);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_filter_genres() {
        let genres = vec![
            make_genre("Rock", "1"),
            make_genre("Electronic", "2"),
            make_genre("Jazz", "3"),
        ];

        let results = SearchFilterService::filter_genres("", &genres);
        assert_eq!(results.len(), 3);

        let results = SearchFilterService::filter_genres("rock", &genres);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rock");
    }

    #[test]
    fn test_case_insensitive_filter() {
        let artists = vec![make_artist("The Beatles", "1")];

        let results = SearchFilterService::filter_artists("BEATLES", None, &artists);
        assert_eq!(results.len(), 1);

        let results = SearchFilterService::filter_artists("beatles", None, &artists);
        assert_eq!(results.len(), 1);
    }
}
