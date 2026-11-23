//! Track-count classification of catalog releases.
use crate::library::models::Album;
pub const DEFAULT_LIMIT: u32 = 2;
pub const MAX_LIMIT: u32 = 1000;
pub fn default_limit() -> u32 {
    DEFAULT_LIMIT
}
pub fn albums(
    albums: &[Album],
    compilations: &super::compilations::CompilationIndex,
    limit: u32,
) -> Vec<Album> {
    let counts: std::collections::HashMap<_, _> = albums
        .iter()
        .map(|album| (album.rating_key.as_str(), album.leaf_count))
        .collect();
    albums
        .iter()
        .filter(|album| {
            if compilations
                .complete_releases
                .contains_key(&album.rating_key)
            {
                return false;
            }
            let count = compilations.release_members.get(&album.rating_key).map_or(
                album.leaf_count,
                |members| {
                    members.iter().try_fold(0u32, |sum, key| {
                        sum.checked_add(counts.get(key.as_str()).copied().flatten()?)
                    })
                },
            );
            count.is_some_and(|count| count > 0 && count <= limit)
        })
        .cloned()
        .collect()
}
