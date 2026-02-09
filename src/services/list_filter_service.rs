//! Inline list filter service.
//!
//! Provides weighted, priority-based filtering for browse lists.
//! Used for real-time filtering as the user types.

use crate::app::state::ListFilterResults;

/// Maximum number of results to return by default.
pub const DEFAULT_MAX_RESULTS: usize = 100;

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
    let short_query = query.len() < 2;

    let mut priority1: Vec<usize> = Vec::new(); // Starts with
    let mut priority2: Vec<usize> = Vec::new(); // Word starts with
    let mut priority3: Vec<usize> = Vec::new(); // Contains

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
            }
        }
    }

    // Combine results in priority order
    let mut matched_indices = priority1;
    matched_indices.extend(priority2);
    matched_indices.extend(priority3);

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
) -> ListFilterResults {
    use crate::app::state::BrowseItem;

    if query.is_empty() {
        return ListFilterResults::default();
    }

    let query_lower = query.to_lowercase();
    let short_query = query.len() < 2;

    let mut priority1: Vec<usize> = Vec::new(); // Starts with
    let mut priority2: Vec<usize> = Vec::new(); // Word starts with
    let mut priority3: Vec<usize> = Vec::new(); // Contains
    let mut priority4: Vec<usize> = Vec::new(); // Year match

    for (idx, item) in items.iter().enumerate() {
        let title = item.title().to_lowercase();

        if title.starts_with(&query_lower) {
            priority1.push(idx);
        } else if !short_query {
            if title.split_whitespace().any(|w| w.starts_with(&query_lower)) {
                priority2.push(idx);
            } else if title.contains(&query_lower) {
                priority3.push(idx);
            } else if let BrowseItem::Album { year: Some(year), .. } = item {
                if year.to_string().contains(&query_lower) {
                    priority4.push(idx);
                }
            }
        } else {
            // Short query: check year match for single-char digits too
            if let BrowseItem::Album { year: Some(year), .. } = item {
                if year.to_string().contains(&query_lower) {
                    priority4.push(idx);
                }
            }
        }
    }

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
    items: &[crate::api::models::Station],
    query: &str,
    max_results: usize,
) -> ListFilterResults {
    filter_with_priority(items, query, |item| &item.title, max_results)
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
