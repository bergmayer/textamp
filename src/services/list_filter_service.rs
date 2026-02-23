//! Inline list filter service.
//!
//! Provides weighted, priority-based filtering for browse lists.
//! Used for real-time filtering as the user types.

use crate::app::state::ListFilterResults;

/// Maximum number of results to return by default.
pub const DEFAULT_MAX_RESULTS: usize = 100;

/// Strip punctuation for fuzzy comparison (keeps alphanumeric + whitespace).
fn normalize_for_search(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
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

    let query_lower = query.to_lowercase();
    let query_normalized = normalize_for_search(&query_lower);
    let short_query = query.len() < 2;

    let mut priority1: Vec<usize> = Vec::new(); // Starts with
    let mut priority2: Vec<usize> = Vec::new(); // Word starts with
    let mut priority3: Vec<usize> = Vec::new(); // Contains
    let mut priority4: Vec<usize> = Vec::new(); // Normalized match (punctuation-insensitive)

    for (idx, item) in items.iter().enumerate() {
        let title = get_title(item).to_lowercase();

        if title.starts_with(&query_lower) {
            priority1.push(idx);
        } else if !short_query {
            // For longer queries, check word boundaries and contains
            if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
                priority2.push(idx);
            } else if title.contains(&query_lower) {
                priority3.push(idx);
            } else {
                // Fuzzy: try punctuation-insensitive match
                let title_normalized = normalize_for_search(&title);
                if title_normalized.contains(&query_normalized) {
                    priority4.push(idx);
                }
            }
        }
    }

    // Combine results in priority order
    let mut matched_indices = priority1;
    matched_indices.extend(priority2);
    matched_indices.extend(priority3);
    matched_indices.extend(priority4);

    let total_matches = matched_indices.len();
    let has_more = matched_indices.len() > max_results;
    matched_indices.truncate(max_results);

    ListFilterResults {
        matched_indices,
        total_matches,
        has_more,
    }
}

/// Filter BrowseItem lists with year matching for albums.
///
/// For Album items, also matches the year field against the query.
/// Year matches are placed in the lowest priority bucket so title matches come first.
pub fn filter_browse_items(
    items: &[crate::app::state::BrowseItem],
    query: &str,
    max_results: usize,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    compilation_artist_keys: &std::collections::HashSet<String>,
) -> ListFilterResults {
    use crate::app::state::BrowseItem;

    if query.is_empty() {
        return ListFilterResults::default();
    }

    let query_lower = query.to_lowercase();
    let query_normalized = normalize_for_search(&query_lower);
    let short_query = query.len() < 2;

    let mut priority1: Vec<usize> = Vec::new(); // Starts with
    let mut priority2: Vec<usize> = Vec::new(); // Word starts with
    let mut priority3: Vec<usize> = Vec::new(); // Contains
    let mut priority4: Vec<usize> = Vec::new(); // Normalized match (punctuation-insensitive)
    let mut priority5: Vec<usize> = Vec::new(); // Year match / alias match

    // Track whether any compilation-only artist matched (to inject Compilations entry)
    let mut compilation_artist_matched = false;

    for (idx, item) in items.iter().enumerate() {
        // Skip compilation-only artists (they appear only on compilations)
        if let BrowseItem::Artist { key, .. } = item {
            if !compilation_artist_keys.is_empty() && compilation_artist_keys.contains(key) {
                // Check if it matches the query — if so, flag for Compilations injection
                let title = item.title().to_lowercase();
                if title.starts_with(&query_lower)
                    || (!short_query && (title.split_whitespace().any(|w| w.starts_with(&query_lower))
                        || title.contains(&query_lower)))
                {
                    compilation_artist_matched = true;
                }
                continue; // Skip this artist from results
            }
        }

        let title = item.title().to_lowercase();

        if title.starts_with(&query_lower) {
            priority1.push(idx);
        } else if !short_query {
            if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
                priority2.push(idx);
            } else if title.contains(&query_lower) {
                priority3.push(idx);
            } else {
                // Fuzzy: try punctuation-insensitive match
                let title_normalized = normalize_for_search(&title);
                if title_normalized.contains(&query_normalized) {
                    priority4.push(idx);
                } else if let BrowseItem::Album { year: Some(year), .. } = item {
                    if year.to_string().contains(&query_lower) {
                        priority5.push(idx);
                    }
                } else if let BrowseItem::Artist { key, .. } = item {
                    // Check artist aliases (with normalization)
                    if let Some(aliases) = artist_aliases.get(key) {
                        let query_norm = crate::services::artist_alias_service::normalize_artist_name(&query_lower);
                        if aliases.iter().any(|alias| {
                            let a = crate::services::artist_alias_service::normalize_artist_name(alias);
                            a.starts_with(&query_norm) || a.contains(&query_norm)
                        }) {
                            priority5.push(idx);
                        }
                    }
                }
            }
        } else {
            // Short query: check year match for single-char digits too
            if let BrowseItem::Album { year: Some(year), .. } = item {
                if year.to_string().contains(&query_lower) {
                    priority5.push(idx);
                }
            }
        }
    }

    // If a compilation-only artist matched, inject the Compilations entry index
    // (find it in the items list)
    if compilation_artist_matched {
        if let Some(comp_idx) = items.iter().position(|item| matches!(item, BrowseItem::Compilations)) {
            // Add at the end of priority5 if not already in results
            if !priority1.contains(&comp_idx) && !priority2.contains(&comp_idx)
                && !priority3.contains(&comp_idx) && !priority4.contains(&comp_idx)
                && !priority5.contains(&comp_idx)
            {
                priority5.push(comp_idx);
            }
        }
    }

    let mut matched_indices = priority1;
    matched_indices.extend(priority2);
    matched_indices.extend(priority3);
    matched_indices.extend(priority4);
    matched_indices.extend(priority5);

    let total_matches = matched_indices.len();
    let has_more = matched_indices.len() > max_results;
    matched_indices.truncate(max_results);

    ListFilterResults {
        matched_indices,
        total_matches,
        has_more,
    }
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
    items: &[crate::plex::models::Station],
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

    let query_lower = query.to_lowercase();
    let query_normalized = normalize_for_search(&query_lower);

    let mut bucket1: Vec<usize> = Vec::new(); // Exact match
    let mut bucket2: Vec<usize> = Vec::new(); // Last word starts with
    let mut bucket3: Vec<usize> = Vec::new(); // Title starts with
    let mut bucket4: Vec<usize> = Vec::new(); // Any word starts with
    let mut bucket5: Vec<usize> = Vec::new(); // Contains
    let mut bucket6: Vec<usize> = Vec::new(); // Normalized contains

    for (idx, item) in items.iter().enumerate() {
        let title = get_title(item).to_lowercase();

        if title == query_lower {
            bucket1.push(idx);
        } else if title.starts_with(&query_lower) {
            bucket3.push(idx);
        } else {
            // Check last word starts with query (last-name heuristic)
            let last_word = title.split_whitespace().last().unwrap_or("");
            if last_word.starts_with(&query_lower) {
                bucket2.push(idx);
            } else if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
                bucket4.push(idx);
            } else if title.contains(&query_lower) {
                bucket5.push(idx);
            } else {
                let title_normalized = normalize_for_search(&title);
                if title_normalized.contains(&query_normalized) {
                    bucket6.push(idx);
                }
            }
        }
    }

    let mut result = Vec::new();
    for bucket in [bucket1, bucket2, bucket3, bucket4, bucket5, bucket6] {
        for idx in bucket {
            if result.len() >= max_results {
                return result;
            }
            result.push(items[idx].clone());
        }
    }
    result
}

/// Search albums with priority-based ranking, including year matching.
///
/// Same priorities as `search_with_ranking`, plus:
/// 7. Year matches query
pub fn search_albums_with_ranking(
    albums: &[crate::plex::models::Album],
    query: &str,
    max_results: usize,
) -> Vec<crate::plex::models::Album> {
    if query.is_empty() {
        return vec![];
    }

    let query_lower = query.to_lowercase();
    let query_normalized = normalize_for_search(&query_lower);

    let mut bucket1: Vec<usize> = Vec::new(); // Exact match
    let mut bucket2: Vec<usize> = Vec::new(); // Last word starts with
    let mut bucket3: Vec<usize> = Vec::new(); // Title starts with
    let mut bucket4: Vec<usize> = Vec::new(); // Any word starts with
    let mut bucket5: Vec<usize> = Vec::new(); // Contains
    let mut bucket6: Vec<usize> = Vec::new(); // Normalized contains
    let mut bucket7: Vec<usize> = Vec::new(); // Year match

    for (idx, album) in albums.iter().enumerate() {
        let title = album.title.to_lowercase();

        if title == query_lower {
            bucket1.push(idx);
        } else if title.starts_with(&query_lower) {
            bucket3.push(idx);
        } else {
            let last_word = title.split_whitespace().last().unwrap_or("");
            if last_word.starts_with(&query_lower) {
                bucket2.push(idx);
            } else if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
                bucket4.push(idx);
            } else if title.contains(&query_lower) {
                bucket5.push(idx);
            } else {
                let title_normalized = normalize_for_search(&title);
                if title_normalized.contains(&query_normalized) {
                    bucket6.push(idx);
                } else if album.year.map(|y| y.to_string().contains(&query_lower)).unwrap_or(false) {
                    bucket7.push(idx);
                }
            }
        }
    }

    let mut result = Vec::new();
    for bucket in [bucket1, bucket2, bucket3, bucket4, bucket5, bucket6, bucket7] {
        for idx in bucket {
            if result.len() >= max_results {
                return result;
            }
            result.push(albums[idx].clone());
        }
    }
    result
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
    tracks: &[crate::plex::models::Track],
    query: &str,
    max_results: usize,
) -> Vec<crate::plex::models::Track> {
    if query.is_empty() {
        return vec![];
    }

    let query_lower = query.to_lowercase();
    let query_normalized = normalize_for_search(&query_lower);

    let mut bucket1: Vec<usize> = Vec::new(); // Exact title match
    let mut bucket2: Vec<usize> = Vec::new(); // Title starts with
    let mut bucket3: Vec<usize> = Vec::new(); // Artist starts with
    let mut bucket4: Vec<usize> = Vec::new(); // Any word in title starts with
    let mut bucket5: Vec<usize> = Vec::new(); // Any word in artist starts with
    let mut bucket6: Vec<usize> = Vec::new(); // Title contains
    let mut bucket7: Vec<usize> = Vec::new(); // Artist contains
    let mut bucket8: Vec<usize> = Vec::new(); // Normalized contains (either)

    for (idx, track) in tracks.iter().enumerate() {
        let title = track.title.to_lowercase();
        let artist = track.grandparent_title.as_deref().unwrap_or("").to_lowercase();

        if title == query_lower {
            bucket1.push(idx);
        } else if title.starts_with(&query_lower) {
            bucket2.push(idx);
        } else if artist.starts_with(&query_lower) {
            bucket3.push(idx);
        } else if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
            bucket4.push(idx);
        } else if artist.split_whitespace().any(|w| w.starts_with(&query_lower)) {
            bucket5.push(idx);
        } else if title.contains(&query_lower) {
            bucket6.push(idx);
        } else if artist.contains(&query_lower) {
            bucket7.push(idx);
        } else {
            let title_norm = normalize_for_search(&title);
            let artist_norm = normalize_for_search(&artist);
            if title_norm.contains(&query_normalized) || artist_norm.contains(&query_normalized) {
                bucket8.push(idx);
            }
        }
    }

    let mut result = Vec::new();
    for bucket in [bucket1, bucket2, bucket3, bucket4, bucket5, bucket6, bucket7, bucket8] {
        for idx in bucket {
            if result.len() >= max_results {
                return result;
            }
            result.push(tracks[idx].clone());
        }
    }
    result
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
}
