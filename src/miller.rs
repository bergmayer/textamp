//! Generic Miller column navigation.
//!
//! Provides reusable traits and state for Miller-column-style navigation
//! across different content types (browse items, folders, stations, etc.).

use serde::{Deserialize, Serialize};

/// Trait for a single column in a Miller columns layout.
///
/// Each concrete column type (BrowseColumn, FolderColumn, StationColumn)
/// implements this so `MillerState<C>` can navigate generically.
pub trait MillerColumn: Clone + std::fmt::Debug {
    /// Number of items in this column.
    fn item_count(&self) -> usize;
    /// Currently selected index.
    fn selected_index(&self) -> usize;
    /// Set the selected index.
    fn set_selected_index(&mut self, idx: usize);
}

/// Generic navigation state for Miller column layouts.
///
/// `C` is the concrete column type (e.g. `BrowseColumn`, `FolderColumn`).
/// Shared navigation logic lives here; type-specific methods go on
/// `impl MillerState<ConcreteColumn>` blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "C: Serialize", deserialize = "C: Deserialize<'de>"))]
pub struct MillerState<C: MillerColumn> {
    /// Columns from left to right.
    pub columns: Vec<C>,
    /// Which column currently has focus (0-indexed).
    pub focused_column: usize,
    /// Loading indicator.
    pub loading: bool,
}

impl<C: MillerColumn> Default for MillerState<C> {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            focused_column: 0,
            loading: false,
        }
    }
}

impl<C: MillerColumn> MillerState<C> {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the focused column.
    pub fn focused(&self) -> Option<&C> {
        self.columns.get(self.focused_column)
    }

    /// Get the focused column mutably.
    pub fn focused_mut(&mut self) -> Option<&mut C> {
        self.columns.get_mut(self.focused_column)
    }

    /// Check if we can go left (focus previous column).
    pub fn can_go_left(&self) -> bool {
        self.focused_column > 0
    }

    /// Move focus left.
    pub fn focus_left(&mut self) {
        if self.focused_column > 0 {
            self.focused_column -= 1;
        }
    }

    /// Move focus right (if there's a column to the right).
    pub fn focus_right(&mut self) -> bool {
        if self.focused_column + 1 < self.columns.len() {
            self.focused_column += 1;
            true
        } else {
            false
        }
    }

    /// Add a new column to the right, removing any columns after current focus.
    pub fn push_column(&mut self, column: C) {
        self.truncate_right();
        self.columns.push(column);
        self.focused_column = self.columns.len() - 1;
    }

    /// Clear columns to the right of the focused column.
    pub fn truncate_right(&mut self) {
        self.columns.truncate(self.focused_column + 1);
    }

    /// Navigate up in current column.
    pub fn move_up(&mut self) {
        if let Some(col) = self.focused_mut() {
            let idx = col.selected_index();
            if idx > 0 {
                col.set_selected_index(idx - 1);
            }
        }
    }

    /// Navigate down in current column.
    pub fn move_down(&mut self) {
        if let Some(col) = self.focused_mut() {
            let max = col.item_count().saturating_sub(1);
            let idx = col.selected_index();
            if idx < max {
                col.set_selected_index(idx + 1);
            }
        }
    }

    /// Move to a specific index in the focused column.
    pub fn move_to(&mut self, index: usize) {
        if let Some(col) = self.focused_mut() {
            if index < col.item_count() {
                col.set_selected_index(index);
            }
        }
    }

    /// Get the number of columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Check if empty (no columns).
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Check if focus is at root column.
    pub fn is_at_root(&self) -> bool {
        self.focused_column == 0
    }

    /// Replace the child column (focused_column + 1) without changing focus.
    /// If a child column exists, replaces it and truncates anything beyond.
    /// If no child column exists, pushes the new column.
    pub fn replace_child_column(&mut self, column: C) {
        let child_idx = self.focused_column + 1;
        if child_idx < self.columns.len() {
            self.columns[child_idx] = column;
            self.columns.truncate(child_idx + 1);
        } else if child_idx == self.columns.len() {
            self.columns.push(column);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug)]
    struct TestColumn {
        items: Vec<String>,
        selected: usize,
    }

    impl TestColumn {
        fn new(items: &[&str]) -> Self {
            Self {
                items: items.iter().map(|s| s.to_string()).collect(),
                selected: 0,
            }
        }
    }

    impl MillerColumn for TestColumn {
        fn item_count(&self) -> usize {
            self.items.len()
        }
        fn selected_index(&self) -> usize {
            self.selected
        }
        fn set_selected_index(&mut self, idx: usize) {
            self.selected = idx;
        }
    }

    fn col(items: &[&str]) -> TestColumn {
        TestColumn::new(items)
    }

    #[test]
    fn new_state_is_empty() {
        let state: MillerState<TestColumn> = MillerState::new();
        assert!(state.is_empty());
        assert_eq!(state.column_count(), 0);
        assert_eq!(state.focused_column, 0);
        assert!(state.focused().is_none());
    }

    #[test]
    fn push_column_adds_and_focuses() {
        let mut state = MillerState::new();
        state.push_column(col(&["a", "b", "c"]));
        assert_eq!(state.column_count(), 1);
        assert_eq!(state.focused_column, 0);
        assert!(state.is_at_root());

        state.push_column(col(&["x", "y"]));
        assert_eq!(state.column_count(), 2);
        assert_eq!(state.focused_column, 1);
        assert!(!state.is_at_root());
    }

    #[test]
    fn push_column_truncates_right() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        state.push_column(col(&["c"]));
        assert_eq!(state.column_count(), 3);

        // Go back to col 0 and push — cols 1,2 should be removed
        state.focused_column = 0;
        state.push_column(col(&["d"]));
        assert_eq!(state.column_count(), 2);
        assert_eq!(state.focused_column, 1);
        assert_eq!(state.focused().unwrap().items[0], "d");
    }

    #[test]
    fn truncate_right_removes_after_focus() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        state.push_column(col(&["c"]));

        state.focused_column = 1;
        state.truncate_right();
        assert_eq!(state.column_count(), 2);
    }

    #[test]
    fn truncate_right_at_end_is_noop() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        // focused is already at 1 (last)
        state.truncate_right();
        assert_eq!(state.column_count(), 2);
    }

    #[test]
    fn focus_left() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        assert_eq!(state.focused_column, 1);
        assert!(state.can_go_left());

        state.focus_left();
        assert_eq!(state.focused_column, 0);
    }

    #[test]
    fn focus_left_at_zero() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.focused_column = 0;
        assert!(!state.can_go_left());

        state.focus_left();
        assert_eq!(state.focused_column, 0);
    }

    #[test]
    fn focus_right() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        state.focused_column = 0;

        assert!(state.focus_right());
        assert_eq!(state.focused_column, 1);
    }

    #[test]
    fn focus_right_at_end() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        // focused_column is 0, only 1 column
        assert!(!state.focus_right());
        assert_eq!(state.focused_column, 0);
    }

    #[test]
    fn move_up_and_down() {
        let mut state = MillerState::new();
        state.push_column(col(&["a", "b", "c"]));

        state.move_down();
        assert_eq!(state.focused().unwrap().selected_index(), 1);
        state.move_down();
        assert_eq!(state.focused().unwrap().selected_index(), 2);
        // At bottom, should clamp
        state.move_down();
        assert_eq!(state.focused().unwrap().selected_index(), 2);

        state.move_up();
        assert_eq!(state.focused().unwrap().selected_index(), 1);
        state.move_up();
        assert_eq!(state.focused().unwrap().selected_index(), 0);
        // At top, should clamp
        state.move_up();
        assert_eq!(state.focused().unwrap().selected_index(), 0);
    }

    #[test]
    fn move_to_direct_jump() {
        let mut state = MillerState::new();
        state.push_column(col(&["a", "b", "c", "d"]));

        state.move_to(3);
        assert_eq!(state.focused().unwrap().selected_index(), 3);

        state.move_to(1);
        assert_eq!(state.focused().unwrap().selected_index(), 1);
    }

    #[test]
    fn move_to_clamps_out_of_bounds() {
        let mut state = MillerState::new();
        state.push_column(col(&["a", "b"]));

        // Out of bounds — should be ignored (no change from 0)
        state.move_to(10);
        assert_eq!(state.focused().unwrap().selected_index(), 0);
    }

    #[test]
    fn replace_child_column_existing() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        state.push_column(col(&["b"]));
        state.push_column(col(&["c"]));

        // Focus col 0, replace col 1
        state.focused_column = 0;
        state.replace_child_column(col(&["replacement"]));
        assert_eq!(state.column_count(), 2); // col 2 truncated
        assert_eq!(state.focused_column, 0); // focus unchanged
        assert_eq!(state.columns[1].items[0], "replacement");
    }

    #[test]
    fn replace_child_column_at_end_appends() {
        let mut state = MillerState::new();
        state.push_column(col(&["a"]));
        // focused is at last column (0), replace_child appends
        state.focused_column = 0;
        state.replace_child_column(col(&["new"]));
        assert_eq!(state.column_count(), 2);
        assert_eq!(state.focused_column, 0); // focus unchanged
        assert_eq!(state.columns[1].items[0], "new");
    }

    #[test]
    fn column_count_and_is_at_root() {
        let mut state = MillerState::new();
        assert_eq!(state.column_count(), 0);
        assert!(state.is_at_root()); // focused_column 0 == 0

        state.push_column(col(&["a"]));
        assert_eq!(state.column_count(), 1);
        assert!(state.is_at_root());

        state.push_column(col(&["b"]));
        assert_eq!(state.column_count(), 2);
        assert!(!state.is_at_root());
    }

    #[test]
    fn move_on_empty_state_is_safe() {
        let mut state: MillerState<TestColumn> = MillerState::new();
        // These should all be no-ops, not panic
        state.move_up();
        state.move_down();
        state.move_to(5);
        state.focus_left();
        assert!(!state.focus_right());
    }
}
