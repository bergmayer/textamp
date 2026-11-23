//! Item-based viewport continuity between presentations with different geometry.
use super::{AppState, BrowseCategory};

/// A stable first-visible item and the fraction clipped above its viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnAnchor {
    pub column: usize,
    pub item_key: String,
    pub fraction: f32,
    pub pointer_pinned: bool,
}
/// Viewport feedback belongs to one source generation and browse destination.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseAnchors {
    pub generation: u64,
    pub category: BrowseCategory,
    pub columns: Vec<ColumnAnchor>,
}
impl BrowseAnchors {
    /// Resolve the current position by item identity, never by a stale row index.
    pub fn offset<'a>(
        &self,
        state: &AppState,
        column: usize,
        mut keys: impl Iterator<Item = &'a str>,
    ) -> Option<usize> {
        if self.generation != state.library_generation || self.category != state.browse_category {
            return None;
        }
        let anchor = self.columns.iter().find(|anchor| anchor.column == column)?;
        keys.position(|key| key == anchor.item_key)
    }
    /// Resolve an anchor within the visible order of a filtered column.
    pub fn display_offset<'a>(
        &self,
        state: &AppState,
        column: usize,
        keys: impl Iterator<Item = &'a str>,
        display_indices: Option<&[usize]>,
    ) -> Option<usize> {
        let original = self.offset(state, column, keys)?;
        match display_indices {
            Some(indices) => indices.iter().position(|&index| index == original),
            None => Some(original),
        }
    }
}
/// Capture terminal feedback after rendering without retaining terminal geometry.
pub fn capture_terminal(state: &AppState) -> BrowseAnchors {
    let columns = state
        .hit_regions
        .miller_offsets
        .iter()
        .filter_map(|&(column, display)| {
            let original = state
                .list_filter
                .results
                .as_ref()
                .filter(|_| {
                    state.list_filter.active
                        && state.list_filter.category == state.browse_category
                        && state.list_filter.column == column
                })
                .and_then(|results| results.matched_indices.get(display))
                .copied()
                .unwrap_or(display);
            let key = if let Some(nav) = state.browse_nav() {
                nav.columns.get(column)?.items.get(original)?.key()
            } else {
                state
                    .folder_state
                    .as_ref()?
                    .columns
                    .get(column)?
                    .items
                    .get(original)?
                    .key
                    .as_str()
            };
            Some(ColumnAnchor {
                column,
                item_key: key.into(),
                fraction: 0.,
                pointer_pinned: state
                    .scroll
                    .browse
                    .is_some_and(|(index, _)| index == column),
            })
        })
        .collect();
    BrowseAnchors {
        generation: state.library_generation,
        category: state.browse_category,
        columns,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchors_follow_identity_and_reject_another_source() {
        let mut state = AppState::new();
        let anchors = BrowseAnchors {
            generation: state.library_generation,
            category: state.browse_category,
            columns: vec![ColumnAnchor {
                column: 0,
                item_key: "b".into(),
                fraction: 0.3,
                pointer_pinned: true,
            }],
        };
        assert_eq!(
            anchors.offset(&state, 0, ["c", "a", "b"].into_iter()),
            Some(2)
        );
        assert_eq!(
            anchors.display_offset(&state, 0, ["a", "x", "b"].into_iter(), Some(&[0, 2])),
            Some(1)
        );
        assert_eq!(
            anchors.display_offset(&state, 0, ["a", "x", "b"].into_iter(), Some(&[0])),
            None
        );
        state.library_generation += 1;
        assert_eq!(anchors.offset(&state, 0, ["b"].into_iter()), None);
    }
}
