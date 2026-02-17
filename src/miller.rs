//! Generic Miller column navigation.
//!
//! Provides reusable traits and state for Miller-column-style navigation
//! across different content types (browse items, folders, stations, etc.).

use serde::{Deserialize, Serialize};

/// Trait for items displayed in Miller columns.
pub trait MillerItem: Clone + std::fmt::Debug {
    /// Unique key for this item.
    fn key(&self) -> &str;
    /// Display title.
    fn title(&self) -> &str;
    /// Whether drilling into this item produces a sub-column.
    fn is_drillable(&self) -> bool;
}

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
    /// If no child column exists, does nothing (auto-drill only replaces).
    pub fn replace_child_column(&mut self, column: C) {
        let child_idx = self.focused_column + 1;
        if child_idx < self.columns.len() {
            self.columns[child_idx] = column;
            self.columns.truncate(child_idx + 1);
        }
    }
}
