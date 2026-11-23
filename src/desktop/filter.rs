//! Preserve raw Miller row identities while showing shared list-filter results.
use super::engine::Snapshot;
pub fn indices(snapshot: &Snapshot, column: usize) -> Option<&[usize]> {
    (snapshot.filter.active
        && snapshot.filter.column == column
        && !snapshot.filter.query.trim().is_empty())
    .then_some(snapshot.filter.results.as_ref())
    .flatten()
    .map(|result| result.matched_indices.as_slice())
}
pub fn album_info_index(snapshot: &Snapshot, column: usize) -> Option<usize> {
    snapshot
        .columns
        .get(column)
        .filter(|c| c.album_info)
        .map(|_| 0)
}
pub fn header_count(snapshot: &Snapshot, column: usize) -> usize {
    usize::from(album_info_index(snapshot, column).is_some())
}
pub fn row(snapshot: &Snapshot, column: usize, display: usize) -> Option<usize> {
    let display = display.checked_sub(header_count(snapshot, column))?;
    match indices(snapshot, column) {
        Some(indices) => indices.get(display).copied(),
        None => snapshot
            .columns
            .get(column)
            .and_then(|column| column.rows.get(display))
            .map(|_| display),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        AppState,
        state::{BrowseColumn, BrowseItem, ListFilterResults},
    };
    #[test]
    fn filtered_rows_keep_pointer_and_context_targets_in_the_original_column() {
        let mut state = AppState::new();
        state.artist_nav.columns = vec![BrowseColumn::new(
            "artists",
            (0..10)
                .map(|i| BrowseItem::Artist {
                    key: format!("artist-{i}"),
                    title: format!("Artist {i}"),
                    thumb: None,
                    is_placeholder: false,
                })
                .collect(),
        )];
        let mut snapshot = Snapshot {
            columns: super::super::browser::columns(&state),
            ..Default::default()
        };
        snapshot.filter.active = true;
        snapshot.filter.query = "artist".into();
        snapshot.filter.results = Some(ListFilterResults {
            matched_indices: vec![5, 1, 8],
            total_matches: 3,
            has_more: false,
        });
        let raw = row(&snapshot, 0, 0).unwrap();
        let target = snapshot.columns[0]
            .target(state.library_generation, 0, raw)
            .unwrap();
        assert_eq!(target.key, "artist-5");
        assert_eq!(target.row, 5);
        assert_eq!(row(&snapshot, 0, 3), None);
    }
}
