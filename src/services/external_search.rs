//! External service search URL generation.
//!
//! Generates search URLs for Apple Music, Spotify, and YouTube.

/// Target service for external search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTarget {
    AppleMusic,
    Spotify,
    YouTube,
}

/// Generate a search URL for the given target service.
pub fn generate_search_url(target: SearchTarget, query: &str) -> String {
    let encoded = urlencoding::encode(query);
    match target {
        SearchTarget::AppleMusic => {
            if cfg!(target_os = "macos") {
                format!("music://music.apple.com/search?term={}", encoded)
            } else {
                format!("https://music.apple.com/search?term={}", encoded)
            }
        }
        SearchTarget::Spotify => {
            format!("https://open.spotify.com/search/{}", encoded)
        }
        SearchTarget::YouTube => {
            format!("https://www.youtube.com/results?search_query={}", encoded)
        }
    }
}
