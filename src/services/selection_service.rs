//! Selection service for determining context-aware selections.
//!
//! Provides pure functions to determine what is currently selected
//! based on view state, focus, and navigation indices.
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and helps determine selection context
//! without coupling to specific actions or event handling.

use crate::plex::models::{Album, Artist, Track};

/// The type of content that can be selected for radio/similar operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionContext {
    /// A track is selected.
    Track { key: String, title: String },
    /// An album is selected.
    Album { key: String, title: String },
    /// An artist is selected.
    Artist { key: String, title: String },
    /// Nothing suitable is selected.
    None,
}

impl SelectionContext {
    /// Create a track context from a Track.
    pub fn from_track(track: &Track) -> Self {
        Self::Track {
            key: track.rating_key.clone(),
            title: format!("{} - {}", track.artist_name(), track.title),
        }
    }

    /// Create an album context from an Album.
    pub fn from_album(album: &Album) -> Self {
        Self::Album {
            key: album.rating_key.clone(),
            title: format!("{} - {}", album.artist_name(), album.title),
        }
    }

    /// Create an artist context from an Artist.
    pub fn from_artist(artist: &Artist) -> Self {
        Self::Artist {
            key: artist.rating_key.clone(),
            title: artist.title.clone(),
        }
    }

    /// Check if this is a None context.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Get the key, if any.
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Track { key, .. } | Self::Album { key, .. } | Self::Artist { key, .. } => {
                Some(key)
            }
            Self::None => None,
        }
    }

    /// Get the title, if any.
    pub fn title(&self) -> Option<&str> {
        match self {
            Self::Track { title, .. } | Self::Album { title, .. } | Self::Artist { title, .. } => {
                Some(title)
            }
            Self::None => None,
        }
    }
}

/// Source type for similar content lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimilarSource {
    /// Load similar albums based on an album.
    Album { rating_key: String, title: String },
    /// Load similar tracks based on a track.
    Track { rating_key: String, title: String },
}

impl SimilarSource {
    /// Create from an album.
    pub fn from_album(album: &Album) -> Self {
        Self::Album {
            rating_key: album.rating_key.clone(),
            title: format!("{} - {}", album.artist_name(), album.title),
        }
    }

    /// Create from a track.
    pub fn from_track(track: &Track) -> Self {
        Self::Track {
            rating_key: track.rating_key.clone(),
            title: format!("{} - {}", track.artist_name(), track.title),
        }
    }
}

/// Service for determining selections in the UI.
pub struct SelectionService;

impl SelectionService {
    /// Get the track at a specific index from a list with history offset.
    ///
    /// Used for queue/now playing views where history items precede queue items.
    pub fn get_track_with_history<'a>(
        index: usize,
        history: &'a [Track],
        queue: &'a [Track],
    ) -> Option<&'a Track> {
        let history_len = history.len();
        if index < history_len {
            history.get(index)
        } else {
            queue.get(index - history_len)
        }
    }

    /// Get a track from a list by index.
    pub fn get_track_at<'a>(tracks: &'a [Track], index: usize) -> Option<&'a Track> {
        tracks.get(index)
    }

    /// Get an album from a list by index.
    pub fn get_album_at<'a>(albums: &'a [Album], index: usize) -> Option<&'a Album> {
        albums.get(index)
    }

    /// Get an artist from a list by index.
    pub fn get_artist_at<'a>(artists: &'a [Artist], index: usize) -> Option<&'a Artist> {
        artists.get(index)
    }

    /// Build selection context from an optional track.
    pub fn context_from_track(track: Option<&Track>) -> SelectionContext {
        match track {
            Some(t) => SelectionContext::from_track(t),
            None => SelectionContext::None,
        }
    }

    /// Build selection context from an optional album.
    pub fn context_from_album(album: Option<&Album>) -> SelectionContext {
        match album {
            Some(a) => SelectionContext::from_album(a),
            None => SelectionContext::None,
        }
    }

    /// Build selection context from an optional artist.
    pub fn context_from_artist(artist: Option<&Artist>) -> SelectionContext {
        match artist {
            Some(a) => SelectionContext::from_artist(a),
            None => SelectionContext::None,
        }
    }

    /// Build similar source from an optional track.
    pub fn similar_from_track(track: Option<&Track>) -> Option<SimilarSource> {
        track.map(SimilarSource::from_track)
    }

    /// Build similar source from an optional album.
    pub fn similar_from_album(album: Option<&Album>) -> Option<SimilarSource> {
        album.map(SimilarSource::from_album)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(title: &str, key: &str) -> Track {
        Track {
            title: title.to_string(),
            rating_key: key.to_string(),
            ..Default::default()
        }
    }

    fn make_album(title: &str, key: &str) -> Album {
        Album {
            title: title.to_string(),
            rating_key: key.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_selection_context_from_track() {
        let track = make_track("Song", "123");
        let ctx = SelectionContext::from_track(&track);
        assert!(matches!(ctx, SelectionContext::Track { .. }));
        assert_eq!(ctx.key(), Some("123"));
    }

    #[test]
    fn test_selection_context_from_album() {
        let album = make_album("Album", "456");
        let ctx = SelectionContext::from_album(&album);
        assert!(matches!(ctx, SelectionContext::Album { .. }));
        assert_eq!(ctx.key(), Some("456"));
    }

    #[test]
    fn test_get_track_with_history() {
        let history = vec![make_track("Old 1", "h1"), make_track("Old 2", "h2")];
        let queue = vec![make_track("New 1", "q1"), make_track("New 2", "q2")];

        // Index in history
        let track = SelectionService::get_track_with_history(0, &history, &queue);
        assert_eq!(track.map(|t| &t.rating_key), Some(&"h1".to_string()));

        // Index in queue (offset by history length)
        let track = SelectionService::get_track_with_history(2, &history, &queue);
        assert_eq!(track.map(|t| &t.rating_key), Some(&"q1".to_string()));

        // Out of bounds
        let track = SelectionService::get_track_with_history(10, &history, &queue);
        assert!(track.is_none());
    }

    #[test]
    fn test_context_from_none() {
        let ctx = SelectionService::context_from_track(None);
        assert!(ctx.is_none());
        assert_eq!(ctx.key(), None);
    }
}
