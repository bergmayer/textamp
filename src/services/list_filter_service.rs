//! Inline list filter service.
//!
//! Provides weighted, priority-based filtering for browse lists.
//! Used for real-time filtering as the user types.

use crate::app::state::ListFilterResults;

/// Maximum number of results to return by default.
pub const DEFAULT_MAX_RESULTS: usize = 100;

/// Fold accented/diacritical characters to ASCII equivalents.
fn fold_to_ascii(c: char) -> Option<char> {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => Some('a'),
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => Some('e'),
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' => Some('i'),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' | 'ø' => Some('o'),
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => Some('u'),
        'ñ' | 'ń' | 'ņ' | 'ň' => Some('n'),
        'ç' | 'ć' | 'ĉ' | 'č' => Some('c'),
        'ð' | 'ď' => Some('d'),
        'ý' | 'ÿ' => Some('y'),
        'ś' | 'ŝ' | 'ş' | 'š' => Some('s'),
        'ź' | 'ż' | 'ž' => Some('z'),
        'ł' => Some('l'),
        'ř' => Some('r'),
        'ť' => Some('t'),
        'þ' => Some('t'),
        'æ' => Some('a'), // ae ligature → a (close enough for search)
        'ß' => Some('s'), // eszett → s
        _ => None,
    }
}

/// Normalize a string for fuzzy comparison: fold accents to ASCII, strip punctuation.
fn normalize_for_search(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() || c.is_whitespace() {
                Some(c)
            } else if c.is_alphanumeric() {
                // Non-ASCII alphanumeric: try accent folding, keep original if no mapping
                Some(fold_to_ascii(c).unwrap_or(c))
            } else {
                None // strip punctuation
            }
        })
        .collect()
}

/// Lower-cased text paired with the punctuation/accent-folded form used by
/// every list search. Keeping the predicates here prevents each ranking loop
/// from rebuilding the same compound conditions.
struct SearchText {
    raw: String,
    normalized: String,
}

impl SearchText {
    fn new(value: &str) -> Self {
        let raw = value.to_lowercase();
        let normalized = normalize_for_search(&raw);
        Self { raw, normalized }
    }

    fn exact(&self, query: &Self) -> bool {
        self.raw == query.raw || self.normalized == query.normalized
    }

    fn starts_with(&self, query: &Self) -> bool {
        self.raw.starts_with(&query.raw) || self.normalized.starts_with(&query.normalized)
    }

    fn word_starts_with(&self, query: &Self) -> bool {
        self.raw
            .split_whitespace()
            .any(|word| word.starts_with(&query.raw))
            || self
                .normalized
                .split_whitespace()
                .any(|word| word.starts_with(&query.normalized))
    }

    fn last_word_starts_with(&self, query: &Self) -> bool {
        self.raw
            .split_whitespace()
            .next_back()
            .unwrap_or("")
            .starts_with(&query.raw)
            || self
                .normalized
                .split_whitespace()
                .next_back()
                .unwrap_or("")
                .starts_with(&query.normalized)
    }

    fn contains(&self, query: &Self) -> bool {
        self.raw.contains(&query.raw) || self.normalized.contains(&query.normalized)
    }
}

#[derive(Clone, Copy)]
enum FilterRank {
    StartsWith = 0,
    WordStartsWith = 1,
    Contains = 2,
}

fn filter_rank(text: &SearchText, query: &SearchText, short_query: bool) -> Option<FilterRank> {
    if text.starts_with(query) {
        Some(FilterRank::StartsWith)
    } else if short_query {
        None
    } else if text.word_starts_with(query) {
        Some(FilterRank::WordStartsWith)
    } else if text.contains(query) {
        Some(FilterRank::Contains)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum SearchRank {
    Exact = 0,
    LastWordStartsWith = 1,
    StartsWith = 2,
    WordStartsWith = 3,
    Contains = 4,
}

fn search_rank(text: &SearchText, query: &SearchText) -> Option<SearchRank> {
    if text.exact(query) {
        Some(SearchRank::Exact)
    } else if text.starts_with(query) {
        Some(SearchRank::StartsWith)
    } else if text.last_word_starts_with(query) {
        Some(SearchRank::LastWordStartsWith)
    } else if text.word_starts_with(query) {
        Some(SearchRank::WordStartsWith)
    } else if text.contains(query) {
        Some(SearchRank::Contains)
    } else {
        None
    }
}

fn finish_filter<const N: usize>(
    buckets: [Vec<usize>; N],
    max_results: usize,
) -> ListFilterResults {
    let mut matched_indices: Vec<_> = buckets.into_iter().flatten().collect();
    let total_matches = matched_indices.len();
    let has_more = total_matches > max_results;
    matched_indices.truncate(max_results);

    ListFilterResults {
        matched_indices,
        total_matches,
        has_more,
    }
}

fn collect_ranked<T: Clone, const N: usize>(
    items: &[T],
    buckets: [Vec<usize>; N],
    max_results: usize,
) -> Vec<T> {
    buckets
        .into_iter()
        .flatten()
        .take(max_results)
        .map(|index| items[index].clone())
        .collect()
}

/// Filter items with priority-based matching.
///
/// Returns indices of matching items in priority order:
/// 1. Items where name starts with query
/// 2. Items where any word starts with query (skipped for queries < 2 chars)
/// 3. Items where name contains query (skipped for queries < 2 chars)
///
/// # Arguments
/// * `items` - The items to filter
/// * `query` - The search query
/// * `get_title` - Function to extract the title from an item
/// * `max_results` - Maximum number of results to return
pub fn filter_with_priority<T, F>(
    items: &[T],
    query: &str,
    get_title: F,
    max_results: usize,
) -> ListFilterResults
where
    F: Fn(&T) -> &str,
{
    if query.is_empty() {
        return ListFilterResults::default();
    }

    let short_query = query.len() < 2;
    let query = SearchText::new(query);
    let mut buckets: [Vec<usize>; 3] = Default::default();

    for (idx, item) in items.iter().enumerate() {
        let title = SearchText::new(get_title(item));
        if let Some(rank) = filter_rank(&title, &query, short_query) {
            buckets[rank as usize].push(idx);
        }
    }

    finish_filter(buckets, max_results)
}

/// Lightweight projection used to move large columns to a filtering worker
/// without cloning complete tracks, albums, artwork paths, and metadata.
#[derive(Debug, Clone)]
pub struct BrowseFilterRecord {
    pub title: String,
    pub album_year: Option<u16>,
    pub artist_key: Option<String>,
    pub is_compilations: bool,
}

fn artist_alias_matches(
    artist_key: &str,
    aliases_by_artist: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    query: &str,
) -> bool {
    aliases_by_artist.get(artist_key).is_some_and(|aliases| {
        let query = crate::services::artist_alias_service::normalize_artist_name(query);
        aliases.iter().any(|alias| {
            let alias = crate::services::artist_alias_service::normalize_artist_name(alias);
            alias.starts_with(&query) || alias.contains(&query)
        })
    })
}

pub fn browse_filter_records(items: &[crate::app::state::BrowseItem]) -> Vec<BrowseFilterRecord> {
    use crate::app::state::BrowseItem;

    items
        .iter()
        .map(|item| BrowseFilterRecord {
            title: item.title().to_string(),
            album_year: match item {
                BrowseItem::Album { year, .. } => *year,
                _ => None,
            },
            artist_key: match item {
                BrowseItem::Artist { key, .. } => Some(key.clone()),
                _ => None,
            },
            is_compilations: matches!(item, BrowseItem::Compilations),
        })
        .collect()
}

/// Filter lightweight browse records with year and artist-alias matching.
pub fn filter_browse_records(
    items: &[BrowseFilterRecord],
    query: &str,
    max_results: usize,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    compilation_artist_keys: &std::collections::HashSet<String>,
) -> ListFilterResults {
    if query.is_empty() {
        return ListFilterResults::default();
    }

    let query_lower = query.to_lowercase();
    let query_text = SearchText::new(&query_lower);
    let short_query = query.len() < 2;
    let mut buckets: [Vec<usize>; 4] = Default::default();

    // Track whether any compilation-only artist matched (to inject Compilations entry)
    let mut compilation_artist_matched = false;

    for (idx, item) in items.iter().enumerate() {
        let title = SearchText::new(&item.title);

        // Skip compilation-only artists (they appear only on compilations)
        if let Some(key) = item.artist_key.as_deref() {
            if !compilation_artist_keys.is_empty() && compilation_artist_keys.contains(key) {
                // Check if it matches the query — if so, flag for Compilations injection
                if filter_rank(&title, &query_text, short_query).is_some() {
                    compilation_artist_matched = true;
                }
                continue; // Skip this artist from results
            }
        }

        match filter_rank(&title, &query_text, short_query) {
            Some(rank) => buckets[rank as usize].push(idx),
            None => {
                if let Some(year) = item.album_year {
                    if year.to_string().contains(&query_lower) {
                        buckets[3].push(idx);
                    }
                } else if !short_query
                    && item
                        .artist_key
                        .as_deref()
                        .is_some_and(|key| artist_alias_matches(key, artist_aliases, &query_lower))
                {
                    buckets[3].push(idx);
                }
            }
        }
    }

    // If a compilation-only artist matched, inject the Compilations entry index
    // (find it in the items list)
    if compilation_artist_matched {
        if let Some(comp_idx) = items.iter().position(|item| item.is_compilations) {
            // Add at the end of priority4 if not already in results
            if !buckets.iter().any(|bucket| bucket.contains(&comp_idx)) {
                buckets[3].push(comp_idx);
            }
        }
    }

    finish_filter(buckets, max_results)
}

/// Filter BrowseItem lists with year matching for albums.
pub fn filter_browse_items(
    items: &[crate::app::state::BrowseItem],
    query: &str,
    max_results: usize,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    compilation_artist_keys: &std::collections::HashSet<String>,
) -> ListFilterResults {
    filter_browse_records(
        &browse_filter_records(items),
        query,
        max_results,
        artist_aliases,
        compilation_artist_keys,
    )
}

/// Wrapper for filtering folder items.
pub fn filter_folder_items(
    items: &[crate::services::FolderItem],
    query: &str,
    max_results: usize,
) -> ListFilterResults {
    filter_with_priority(items, query, |item| &item.title, max_results)
}

/// Wrapper for filtering stations.
pub fn filter_stations(
    items: &[crate::library::models::Station],
    query: &str,
    max_results: usize,
) -> ListFilterResults {
    filter_with_priority(items, query, |item| &item.title, max_results)
}

/// Search items with priority-based ranking and last-name prioritization.
///
/// Returns cloned items in priority order:
/// 1. Exact match (title == query)
/// 2. Last word starts with query (last-name heuristic: "J.S. Bach" for "bach")
/// 3. Title starts with query
/// 4. Any word starts with query
/// 5. Contains query as substring
/// 6. Normalized contains (punctuation-stripped)
pub fn search_with_ranking<T: Clone, F>(
    items: &[T],
    query: &str,
    get_title: F,
    max_results: usize,
) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    if query.is_empty() {
        return vec![];
    }

    let query = SearchText::new(query);
    let mut buckets: [Vec<usize>; 5] = Default::default();

    for (idx, item) in items.iter().enumerate() {
        let title = SearchText::new(get_title(item));
        if let Some(rank) = search_rank(&title, &query) {
            buckets[rank as usize].push(idx);
        }
    }

    collect_ranked(items, buckets, max_results)
}

/// Search albums with priority-based ranking, including year matching.
///
/// Same priorities as `search_with_ranking`, plus:
/// 7. Year matches query
pub fn search_albums_with_ranking(
    albums: &[crate::library::models::Album],
    query: &str,
    max_results: usize,
) -> Vec<crate::library::models::Album> {
    if query.is_empty() {
        return vec![];
    }

    let query_lower = query.to_lowercase();
    let query_text = SearchText::new(&query_lower);
    let mut buckets: [Vec<usize>; 6] = Default::default();

    for (idx, album) in albums.iter().enumerate() {
        let title = SearchText::new(&album.title);
        if let Some(rank) = search_rank(&title, &query_text) {
            buckets[rank as usize].push(idx);
        } else if album
            .year
            .is_some_and(|year| year.to_string().contains(&query_lower))
        {
            buckets[5].push(idx);
        }
    }

    collect_ranked(albums, buckets, max_results)
}

/// Search tracks with multi-field priority-based ranking.
///
/// Returns cloned tracks in priority order across title and artist fields:
/// 1. Exact title match
/// 2. Title starts with query
/// 3. Artist name starts with query
/// 4. Any word in title starts with query
/// 5. Any word in artist name starts with query
/// 6. Title contains query
/// 7. Artist name contains query
/// 8. Normalized contains (either field)
pub fn search_tracks_with_ranking(
    tracks: &[crate::library::models::Track],
    query: &str,
    max_results: usize,
) -> Vec<crate::library::models::Track> {
    if query.is_empty() {
        return vec![];
    }

    let query = SearchText::new(query);
    let mut buckets: [Vec<usize>; 7] = Default::default();

    for (idx, track) in tracks.iter().enumerate() {
        let title = SearchText::new(&track.title);
        let artist = SearchText::new(track.grandparent_title.as_deref().unwrap_or(""));
        let rank = if title.exact(&query) {
            Some(0)
        } else if title.starts_with(&query) {
            Some(1)
        } else if artist.starts_with(&query) {
            Some(2)
        } else if title.word_starts_with(&query) {
            Some(3)
        } else if artist.word_starts_with(&query) {
            Some(4)
        } else if title.contains(&query) {
            Some(5)
        } else if artist.contains(&query) {
            Some(6)
        } else {
            None
        };
        if let Some(rank) = rank {
            buckets[rank].push(idx);
        }
    }

    collect_ranked(tracks, buckets, max_results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_starts_with() {
        let items = vec!["Apple", "Banana", "Apricot", "Cherry"];
        let results = filter_with_priority(&items, "ap", |s| s, 100);

        // Should match "Apple" and "Apricot" (starts with "ap")
        assert_eq!(results.matched_indices, vec![0, 2]);
        assert_eq!(results.total_matches, 2);
        assert!(!results.has_more);
    }

    #[test]
    fn test_filter_priority_order() {
        let items = vec!["Beethoven", "The Beatles", "Beach Boys"];
        let results = filter_with_priority(&items, "be", |s| s, 100);

        // "Beethoven" and "Beach Boys" start with "be"
        // "The Beatles" has "Beatles" starting with "be"
        assert_eq!(results.matched_indices.len(), 3);
        assert_eq!(results.matched_indices[0], 0); // Beethoven first
        assert_eq!(results.matched_indices[1], 2); // Beach Boys second
        assert_eq!(results.matched_indices[2], 1); // The Beatles third (word match)
    }

    #[test]
    fn test_filter_short_query() {
        let items = vec!["Beethoven", "The Beatles", "Bach"];
        let results = filter_with_priority(&items, "b", |s| s, 100);

        // Short query: only "starts with" matches
        // "Beethoven" and "Bach" start with "b"
        // "The Beatles" does NOT match (word boundary check skipped for short queries)
        assert_eq!(results.matched_indices, vec![0, 2]);
    }

    #[test]
    fn test_filter_contains() {
        let items = vec!["Abbey Road", "Let It Be", "Rubber Soul"];
        let results = filter_with_priority(&items, "ber", |s| s, 100);

        // "Rubber Soul" contains "ber" (not at start or word boundary)
        assert_eq!(results.matched_indices, vec![2]);
    }

    #[test]
    fn test_filter_max_results() {
        let items: Vec<String> = (0..200).map(|i| format!("Item {}", i)).collect();
        let results = filter_with_priority(&items, "item", |s| s, 50);

        assert_eq!(results.matched_indices.len(), 50);
        assert_eq!(results.total_matches, 200);
        assert!(results.has_more);
    }

    #[test]
    fn test_filter_empty_query() {
        let items = vec!["Apple", "Banana"];
        let results = filter_with_priority(&items, "", |s| s, 100);

        assert!(results.matched_indices.is_empty());
        assert_eq!(results.total_matches, 0);
    }

    #[test]
    fn search_ranking_preserves_last_word_priority() {
        let items = vec!["Bach Ensemble", "Johann Sebastian Bach", "Bach"];
        let results = search_with_ranking(&items, "bach", |item| *item, 100);

        assert_eq!(
            results,
            vec!["Bach", "Johann Sebastian Bach", "Bach Ensemble"]
        );
    }

    #[test]
    fn album_search_falls_back_to_year() {
        let albums = vec![
            crate::library::models::Album {
                title: "Older Album".to_string(),
                year: Some(1999),
                ..Default::default()
            },
            crate::library::models::Album {
                title: "Newer Album".to_string(),
                year: Some(2024),
                ..Default::default()
            },
        ];

        let results = search_albums_with_ranking(&albums, "1999", 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Older Album");
    }

    #[test]
    fn track_search_ranks_title_before_artist() {
        let tracks = vec![
            crate::library::models::Track {
                title: "Something Else".to_string(),
                grandparent_title: Some("Blue Train".to_string()),
                ..Default::default()
            },
            crate::library::models::Track {
                title: "Blue Train".to_string(),
                grandparent_title: Some("John Coltrane".to_string()),
                ..Default::default()
            },
        ];

        let results = search_tracks_with_ranking(&tracks, "blue", 100);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Blue Train");
        assert_eq!(results[1].title, "Something Else");
    }
}
