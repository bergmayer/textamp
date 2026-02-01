//! Navigation service for list navigation and scrolling.
//!
//! Provides pure functions for common list navigation operations:
//! - Index adjustment with bounds checking
//! - Scroll offset calculation for centered selection
//! - Jump-to-letter navigation
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and can be used with any frontend.
//! All functions are pure and have no side effects.

/// Service for list navigation operations.
pub struct NavigationService;

impl NavigationService {
    /// Adjust an index by a delta with bounds checking.
    ///
    /// Returns the new index clamped to valid range [0, len-1].
    /// If len is 0, returns 0.
    ///
    /// # Examples
    /// ```
    /// use textamp::services::NavigationService;
    ///
    /// // Move down in a list of 10 items
    /// assert_eq!(NavigationService::adjust_index(0, 1, 10), 1);
    /// // Can't go past end
    /// assert_eq!(NavigationService::adjust_index(9, 1, 10), 9);
    /// // Can't go before start
    /// assert_eq!(NavigationService::adjust_index(0, -1, 10), 0);
    /// ```
    #[inline]
    pub fn adjust_index(current: usize, delta: isize, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let new_index = current as isize + delta;
        new_index.clamp(0, len as isize - 1) as usize
    }

    /// Set index to a specific target value with bounds checking.
    ///
    /// Special values:
    /// - `isize::MAX` or `-1`: Jump to last item
    /// - `0`: Jump to first item
    /// - Any other value: Clamped to valid range
    ///
    /// # Examples
    /// ```
    /// use textamp::services::NavigationService;
    ///
    /// // Jump to specific index
    /// assert_eq!(NavigationService::set_index(5, 10), 5);
    /// // Jump to last with MAX
    /// assert_eq!(NavigationService::set_index(isize::MAX, 10), 9);
    /// // Clamp to bounds
    /// assert_eq!(NavigationService::set_index(100, 10), 9);
    /// ```
    #[inline]
    pub fn set_index(target: isize, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        if target == isize::MAX || target == -1 {
            len.saturating_sub(1)
        } else {
            (target as usize).min(len.saturating_sub(1))
        }
    }

    /// Calculate scroll offset to keep selected item centered in viewport.
    ///
    /// The selected item is kept in the center of the viewport when possible.
    /// At the start or end of the list, the viewport won't scroll past bounds.
    ///
    /// # Examples
    /// ```
    /// use textamp::services::NavigationService;
    ///
    /// // At start of list, no scroll
    /// assert_eq!(NavigationService::calc_scroll_offset(0, 10, 100), 0);
    /// // In middle, center the selection
    /// assert_eq!(NavigationService::calc_scroll_offset(50, 10, 100), 45);
    /// // At end, don't scroll past bounds
    /// assert_eq!(NavigationService::calc_scroll_offset(99, 10, 100), 90);
    /// ```
    #[inline]
    pub fn calc_scroll_offset(selected: usize, viewport_height: usize, total: usize) -> usize {
        if total == 0 || viewport_height == 0 {
            return 0;
        }

        let half_height = viewport_height / 2;

        if selected < half_height {
            0
        } else if selected + half_height >= total {
            total.saturating_sub(viewport_height)
        } else {
            selected.saturating_sub(half_height)
        }
    }

    /// Find the index of the first item starting with a given letter.
    ///
    /// The search is case-insensitive and ignores leading "The " prefix.
    ///
    /// # Examples
    /// ```
    /// use textamp::services::NavigationService;
    ///
    /// let items = vec!["Alice", "Bob", "The Beatles", "Charlie"];
    /// assert_eq!(NavigationService::jump_to_letter(&items, 'b', |s| s.to_string()), Some(1));
    /// // "The Beatles" matches 'B' because "The " is ignored
    /// assert_eq!(NavigationService::jump_to_letter(&items, 'c', |s| s.to_string()), Some(3));
    /// assert_eq!(NavigationService::jump_to_letter(&items, 'z', |s| s.to_string()), None);
    /// ```
    pub fn jump_to_letter<T, F>(items: &[T], letter: char, title_fn: F) -> Option<usize>
    where
        F: Fn(&T) -> String,
    {
        let target = letter.to_ascii_lowercase();
        items.iter().position(|item| {
            let title = title_fn(item);
            let normalized = Self::normalize_title(&title);
            normalized
                .chars()
                .next()
                .map(|c| c.to_ascii_lowercase() == target)
                .unwrap_or(false)
        })
    }

    /// Normalize a title for sorting/searching by removing leading "The ".
    fn normalize_title(title: &str) -> &str {
        let lower = title.to_lowercase();
        if lower.starts_with("the ") && title.len() > 4 {
            &title[4..]
        } else {
            title
        }
    }

    /// Page up/down navigation.
    ///
    /// Moves the selection by `page_size` items, respecting bounds.
    ///
    /// # Examples
    /// ```
    /// use textamp::services::NavigationService;
    ///
    /// // Page down
    /// assert_eq!(NavigationService::page_navigate(0, 10, 100, true), 10);
    /// // Page up
    /// assert_eq!(NavigationService::page_navigate(50, 10, 100, false), 40);
    /// // Don't go past end
    /// assert_eq!(NavigationService::page_navigate(95, 10, 100, true), 99);
    /// ```
    #[inline]
    pub fn page_navigate(current: usize, page_size: usize, total: usize, down: bool) -> usize {
        if total == 0 {
            return 0;
        }
        if down {
            (current + page_size).min(total.saturating_sub(1))
        } else {
            current.saturating_sub(page_size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjust_index_basic() {
        assert_eq!(NavigationService::adjust_index(5, 1, 10), 6);
        assert_eq!(NavigationService::adjust_index(5, -1, 10), 4);
        assert_eq!(NavigationService::adjust_index(5, 5, 10), 9);
        assert_eq!(NavigationService::adjust_index(5, -10, 10), 0);
    }

    #[test]
    fn test_adjust_index_bounds() {
        assert_eq!(NavigationService::adjust_index(9, 1, 10), 9);
        assert_eq!(NavigationService::adjust_index(0, -1, 10), 0);
    }

    #[test]
    fn test_adjust_index_empty() {
        assert_eq!(NavigationService::adjust_index(0, 1, 0), 0);
    }

    #[test]
    fn test_set_index() {
        assert_eq!(NavigationService::set_index(5, 10), 5);
        assert_eq!(NavigationService::set_index(0, 10), 0);
        assert_eq!(NavigationService::set_index(isize::MAX, 10), 9);
        assert_eq!(NavigationService::set_index(100, 10), 9);
    }

    #[test]
    fn test_set_index_empty() {
        assert_eq!(NavigationService::set_index(5, 0), 0);
    }

    #[test]
    fn test_calc_scroll_offset() {
        // At start
        assert_eq!(NavigationService::calc_scroll_offset(0, 10, 100), 0);
        assert_eq!(NavigationService::calc_scroll_offset(4, 10, 100), 0);
        // In middle
        assert_eq!(NavigationService::calc_scroll_offset(50, 10, 100), 45);
        // At end
        assert_eq!(NavigationService::calc_scroll_offset(99, 10, 100), 90);
        assert_eq!(NavigationService::calc_scroll_offset(95, 10, 100), 90);
    }

    #[test]
    fn test_calc_scroll_offset_edge_cases() {
        assert_eq!(NavigationService::calc_scroll_offset(0, 0, 100), 0);
        assert_eq!(NavigationService::calc_scroll_offset(0, 10, 0), 0);
        assert_eq!(NavigationService::calc_scroll_offset(5, 20, 10), 0);
    }

    #[test]
    fn test_jump_to_letter() {
        let items = vec!["Alice", "Bob", "Charlie", "David"];
        assert_eq!(NavigationService::jump_to_letter(&items, 'a', |s| s.to_string()), Some(0));
        assert_eq!(NavigationService::jump_to_letter(&items, 'b', |s| s.to_string()), Some(1));
        assert_eq!(NavigationService::jump_to_letter(&items, 'c', |s| s.to_string()), Some(2));
        assert_eq!(NavigationService::jump_to_letter(&items, 'z', |s| s.to_string()), None);
    }

    #[test]
    fn test_jump_to_letter_the_prefix() {
        let items = vec!["Alice", "The Beatles", "Charlie"];
        // "The Beatles" should match 'B' because "The " is stripped
        assert_eq!(NavigationService::jump_to_letter(&items, 'b', |s| s.to_string()), Some(1));
    }

    #[test]
    fn test_page_navigate() {
        assert_eq!(NavigationService::page_navigate(0, 10, 100, true), 10);
        assert_eq!(NavigationService::page_navigate(50, 10, 100, false), 40);
        assert_eq!(NavigationService::page_navigate(95, 10, 100, true), 99);
        assert_eq!(NavigationService::page_navigate(5, 10, 100, false), 0);
    }
}
