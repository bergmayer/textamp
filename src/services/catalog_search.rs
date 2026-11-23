//! Ranked catalog search over shared immutable metadata snapshots.
use crate::library::models::{Album, Artist, Genre, Playlist, SearchResults, Track};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
pub struct CatalogSearch {
    pub artists: Arc<Vec<Artist>>,
    pub albums: Arc<Vec<Album>>,
    pub playlists: Arc<Vec<Playlist>>,
    pub genres: Arc<Vec<Genre>>,
    pub aliases: Arc<HashMap<String, HashSet<String>>>,
    pub tracks: Arc<Vec<Track>>,
}
impl CatalogSearch {
    pub fn item_count(&self) -> usize {
        self.artists.len()
            + self.albums.len()
            + self.playlists.len()
            + self.genres.len()
            + self.tracks.len()
    }
    pub fn search(&self, query: &str, cancelled: impl FnMut() -> bool) -> SearchResults {
        let catalog = IndexedCatalog::new(self.clone());
        catalog.search(query, cancelled)
    }
    fn same_snapshot(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.artists, &other.artists)
            && Arc::ptr_eq(&self.albums, &other.albums)
            && Arc::ptr_eq(&self.playlists, &other.playlists)
            && Arc::ptr_eq(&self.genres, &other.genres)
            && Arc::ptr_eq(&self.aliases, &other.aliases)
            && Arc::ptr_eq(&self.tracks, &other.tracks)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cache {
    current: Option<Arc<IndexedCatalog>>,
}
impl Cache {
    pub fn prepare(&mut self, catalog: CatalogSearch) -> Arc<IndexedCatalog> {
        if self
            .current
            .as_ref()
            .is_none_or(|current| !current.catalog.same_snapshot(&catalog))
        {
            self.current = Some(Arc::new(IndexedCatalog::new(catalog)));
        }
        self.current.as_ref().expect("catalog installed").clone()
    }
}

use super::list_filter_service::{SearchQuery, SearchRecord, ranked_indices};
#[derive(Debug, Default)]
struct Categories {
    artists: Vec<SearchRecord>,
    albums: Vec<SearchRecord>,
    playlists: Vec<SearchRecord>,
    genres: Vec<SearchRecord>,
    aliases: Vec<Vec<String>>,
}
impl Categories {
    fn build(catalog: &CatalogSearch, cancelled: &mut impl FnMut() -> bool) -> Option<Self> {
        let artists = records(
            catalog
                .artists
                .iter()
                .map(|item| SearchRecord::name(&item.title)),
            cancelled,
        )?;
        let albums = records(catalog.albums.iter().map(SearchRecord::album), cancelled)?;
        let playlists = records(
            catalog
                .playlists
                .iter()
                .map(|item| SearchRecord::name(&item.title)),
            cancelled,
        )?;
        let genres = records(
            catalog
                .genres
                .iter()
                .map(|item| SearchRecord::name(&item.title)),
            cancelled,
        )?;
        let mut aliases = Vec::with_capacity(catalog.artists.len());
        for artist in catalog.artists.iter() {
            if cancelled() {
                return None;
            }
            aliases.push(
                catalog
                    .aliases
                    .get(&artist.rating_key)
                    .into_iter()
                    .flatten()
                    .map(|name| super::artist_alias_service::normalize_artist_name(name))
                    .collect(),
            );
        }
        Some(Self {
            artists,
            albums,
            playlists,
            genres,
            aliases,
        })
    }
}

fn records(
    items: impl Iterator<Item = SearchRecord>,
    cancelled: &mut impl FnMut() -> bool,
) -> Option<Vec<SearchRecord>> {
    let mut result = Vec::new();
    for (i, record) in items.enumerate() {
        if i % 256 == 0 && cancelled() {
            return None;
        }
        result.push(record);
    }
    Some(result)
}

/// A bit per track in each posting lets three disjoint fragments shortlist even
/// single-edit matches, including a transposition across a fragment boundary.
#[derive(Debug)]
struct TrackIndex {
    rows: Vec<SearchRecord>,
    blocks: Vec<Vec<(u32, u64)>>,
}

impl TrackIndex {
    const BLOCK: usize = u64::BITS as usize;

    fn build(tracks: &[Track], cancelled: &mut impl FnMut() -> bool) -> Option<Self> {
        let rows = records(tracks.iter().map(SearchRecord::track), cancelled)?;
        let mut blocks = Vec::new();
        if rows.len() > 4096 {
            blocks.resize_with(128 * 128, Vec::new);
            for (block, rows) in rows.chunks(Self::BLOCK).enumerate() {
                if cancelled() {
                    return None;
                }
                for (offset, row) in rows.iter().enumerate() {
                    let bit = 1u64 << offset;
                    for pair in row.searchable().as_bytes().windows(2) {
                        if pair[0] < 128 && pair[1] < 128 {
                            let list: &mut Vec<(u32, u64)> = &mut blocks[Self::key(pair)];
                            match list.last_mut() {
                                Some((id, mask)) if *id == block as u32 => *mask |= bit,
                                _ => list.push((block as u32, bit)),
                            }
                        }
                    }
                }
            }
        }
        Some(Self { rows, blocks })
    }

    fn key(pair: &[u8]) -> usize {
        usize::from(pair[0]) * 128 + usize::from(pair[1])
    }

    fn search(
        &self,
        query: &SearchQuery,
        limit: usize,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Vec<usize> {
        let Some(anchor) = query.anchor().filter(|_| !self.blocks.is_empty()) else {
            return ranked_indices(&self.rows, query, limit, cancelled);
        };
        let mut candidates = vec![0u64; self.rows.len().div_ceil(Self::BLOCK)];
        for part in 0..3 {
            let fragment =
                &anchor.as_bytes()[part * anchor.len() / 3..(part + 1) * anchor.len() / 3];
            if let Some(rarest) = fragment
                .windows(2)
                .map(|p| &self.blocks[Self::key(p)])
                .min_by_key(|list| list.len())
            {
                for &(block, mask) in rarest {
                    candidates[block as usize] |= mask;
                }
            }
        }
        let ids = candidates
            .into_iter()
            .enumerate()
            .flat_map(|(block, mut mask)| {
                std::iter::from_fn(move || {
                    if mask == 0 {
                        return None;
                    }
                    let offset = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    Some(block * Self::BLOCK + offset)
                })
            });
        super::list_filter_service::rank_indexed(
            ids.map(|i| (i, &self.rows[i])),
            query,
            limit,
            cancelled,
        )
    }
}

#[derive(Debug)]
pub struct IndexedCatalog {
    catalog: CatalogSearch,
    categories: std::sync::OnceLock<Categories>,
    tracks: std::sync::OnceLock<TrackIndex>,
}
impl IndexedCatalog {
    fn new(catalog: CatalogSearch) -> Self {
        Self {
            catalog,
            categories: Default::default(),
            tracks: Default::default(),
        }
    }
    pub fn ready(&self) -> bool {
        self.categories.get().is_some() && self.tracks.get().is_some()
    }
    pub fn item_count(&self) -> usize {
        self.catalog.item_count()
    }
    fn warm_categories(&self, cancelled: &mut impl FnMut() -> bool) {
        if self.categories.get().is_none()
            && let Some(index) = Categories::build(&self.catalog, cancelled)
        {
            let _ = self.categories.set(index);
        }
    }
    pub fn warm(&self, mut cancelled: impl FnMut() -> bool) {
        self.warm_categories(&mut cancelled);
        if self.tracks.get().is_none()
            && let Some(tracks) = TrackIndex::build(&self.catalog.tracks, &mut cancelled)
        {
            let _ = self.tracks.set(tracks);
        }
    }
    pub fn search(&self, query: &str, cancelled: impl FnMut() -> bool) -> SearchResults {
        self.search_inner(query, true, cancelled)
    }
    /// Category results remain available while track normalization runs independently.
    pub fn categories(&self, query: &str, cancelled: impl FnMut() -> bool) -> SearchResults {
        self.search_inner(query, false, cancelled)
    }
    fn search_inner(
        &self,
        query: &str,
        include_tracks: bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> SearchResults {
        let Some(plan) = SearchQuery::new(query) else {
            return SearchResults::default();
        };
        if include_tracks {
            self.warm(&mut cancelled);
        } else {
            self.warm_categories(&mut cancelled);
        }
        let Some(index) = self.categories.get().filter(|_| !cancelled()) else {
            return SearchResults::default();
        };
        let mut artist_indices = ranked_indices(&index.artists, &plan, 50, &mut cancelled);
        if artist_indices.len() < 50 {
            let normalized = super::artist_alias_service::normalize_artist_name(query);
            for (i, aliases) in index.aliases.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return SearchResults::default();
                }
                if !artist_indices.contains(&i)
                    && !normalized.is_empty()
                    && aliases.iter().any(|name| name.contains(&normalized))
                {
                    artist_indices.push(i);
                    if artist_indices.len() == 50 {
                        break;
                    }
                }
            }
        }
        let albums = ranked_indices(&index.albums, &plan, 50, &mut cancelled);
        let playlists = ranked_indices(&index.playlists, &plan, 50, &mut cancelled);
        let genres = ranked_indices(&index.genres, &plan, 50, &mut cancelled);
        let tracks = if include_tracks {
            let Some(tracks) = self.tracks.get() else {
                return SearchResults::default();
            };
            tracks.search(&plan, 100, &mut cancelled)
        } else {
            vec![]
        };
        if cancelled() {
            return SearchResults::default();
        }
        let titles = artist_indices
            .iter()
            .map(|&i| (2, &self.catalog.artists[i].title))
            .chain(albums.iter().map(|&i| (0, &self.catalog.albums[i].title)))
            .chain(
                playlists
                    .iter()
                    .map(|&i| (3, &self.catalog.playlists[i].title)),
            )
            .chain(genres.iter().map(|&i| (4, &self.catalog.genres[i].title)))
            .chain(tracks.iter().map(|&i| (1, &self.catalog.tracks[i].title)));
        let mut ranked: Vec<_> = titles
            .enumerate()
            .map(|(index, (kind, title))| {
                let relevance = plan.title_priority(title);
                // Album and song titles take priority for exact and near matches.
                // For broader searches, put a matching artist above their recordings.
                let kind = if relevance >= 2 {
                    match kind {
                        2 => 0,
                        0 => 1,
                        1 => 2,
                        other => other,
                    }
                } else {
                    kind
                };
                (relevance, kind, index)
            })
            .collect();
        ranked.sort_unstable();
        SearchResults {
            global_order: ranked.into_iter().map(|(_, _, index)| index).collect(),
            artists: artist_indices
                .into_iter()
                .map(|i| self.catalog.artists[i].clone())
                .collect(),
            albums: albums
                .into_iter()
                .map(|i| self.catalog.albums[i].clone())
                .collect(),
            playlists: playlists
                .into_iter()
                .map(|i| self.catalog.playlists[i].clone())
                .collect(),
            genres: genres
                .into_iter()
                .map(|i| self.catalog.genres[i].clone())
                .collect(),
            tracks: tracks
                .into_iter()
                .map(|i| self.catalog.tracks[i].clone())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_and_near_titles_precede_partial_artist_matches() {
        let query = "Blonde on Blonde";
        let catalog = CatalogSearch {
            artists: Arc::new(vec![Artist {
                title: "The Blonde on Blonde Ensemble".into(),
                ..Default::default()
            }]),
            albums: Arc::new(vec![Album {
                title: query.into(),
                parent_title: Some("Bob Dylan".into()),
                ..Default::default()
            }]),
            tracks: Arc::new(
                [query, "Blonde on Blond", "Blonde on Blonde (Live)"]
                    .into_iter()
                    .map(|title| Track {
                        title: title.into(),
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let results = catalog.search(query, || false);
        assert_eq!(results.total_count(), 5);
        use crate::app::{handlers::dispatch_search::resolve_global_index, state::SearchTab};
        let kinds: Vec<_> = (0..5)
            .map(|i| resolve_global_index(&results, i).0)
            .collect();
        assert_eq!(
            kinds,
            [
                SearchTab::Albums,
                SearchTab::Tracks,
                SearchTab::Tracks,
                SearchTab::Tracks,
                SearchTab::Artists
            ]
        );
    }
    #[test]
    fn surname_finds_artist_before_self_titled_album() {
        let catalog = CatalogSearch {
            artists: Arc::new(vec![Artist {
                title: "Bob Dylan".into(),
                ..Default::default()
            }]),
            albums: Arc::new(vec![Album {
                title: "Bob Dylan".into(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let results = catalog.search("dylan", || false);
        assert_eq!(results.global_order, [0, 1]);
    }
    fn fixture() -> CatalogSearch {
        CatalogSearch {
            artists: Arc::new(vec![Artist {
                rating_key: "artist".into(),
                title: "Björk".into(),
                ..Default::default()
            }]),
            albums: Arc::new(vec![Album {
                rating_key: "album".into(),
                title: "Debut".into(),
                parent_title: Some("Björk".into()),
                year: Some(1993),
                ..Default::default()
            }]),
            tracks: Arc::new(vec![Track {
                rating_key: "song".into(),
                title: "Human Behaviour".into(),
                grandparent_title: Some("Björk".into()),
                parent_title: Some("Debut".into()),
                genres: vec!["Electronic".into()],
                ..Default::default()
            }]),
            ..Default::default()
        }
    }
    #[test]
    fn category_results_are_available_before_tracks_and_catalog_changes_replace_the_index() {
        let mut cache = Cache::default();
        let mut catalog = fixture();
        let indexed = cache.prepare(catalog.clone());
        assert_eq!(
            indexed.categories("bjork", || false).albums[0].rating_key,
            "album"
        );
        assert!(!indexed.ready());
        assert_eq!(
            indexed.search("bjork", || false).tracks[0].rating_key,
            "song"
        );
        assert_eq!(
            indexed.search("human AND electronic", || false).tracks[0].rating_key,
            "song"
        );
        assert!(
            indexed
                .search("human AND NOT electronic", || false)
                .tracks
                .is_empty()
        );
        assert!(Arc::ptr_eq(&indexed, &cache.prepare(catalog.clone())));
        Arc::make_mut(&mut catalog.tracks)[0].title = "Venus as a Boy".into();
        let updated = cache.prepare(catalog);
        assert!(updated.search("human", || false).tracks.is_empty());
        assert_eq!(
            updated.search("venus", || false).tracks[0].rating_key,
            "song"
        );
        assert_eq!(
            indexed.search("human", || false).tracks[0].rating_key,
            "song"
        );
    }
    #[test]
    fn index_preserves_fuzzy_and_literal_matches_across_block_boundaries() {
        let mut tracks = vec![
            Track {
                title: "Other Song".into(),
                ..Default::default()
            };
            4100
        ];
        for (index, title) in [
            (0, "Behvaiour"),
            (63, "Behavour"),
            (64, "Behaviiour"),
            (127, "Beheviour"),
            (4099, "Behaviour"),
        ] {
            tracks[index].title = title.into();
        }
        let index = TrackIndex::build(&tracks, &mut || false).unwrap();
        let query = SearchQuery::new("behaviour").unwrap();
        assert_eq!(
            index.search(&query, 10, &mut || false),
            [4099, 0, 63, 64, 127]
        );
        let query = SearchQuery::new("\"behaviour\"").unwrap();
        assert_eq!(index.search(&query, 10, &mut || false), [4099]);
    }
    #[test]
    fn cancelled_index_builds_can_be_retried_without_publishing_partial_results() {
        let indexed = Cache::default().prepare(fixture());
        assert!(indexed.search("human", || true).tracks.is_empty());
        assert!(!indexed.ready());
        assert_eq!(
            indexed.search("human", || false).tracks[0].rating_key,
            "song"
        );
        assert!(indexed.search("human", || true).tracks.is_empty());
    }
    #[test]
    #[ignore = "development search timing"]
    fn catalog_search_latency() {
        let tracks = (0..250_000)
            .map(|i| Track {
                rating_key: format!("track-{i}"),
                title: if i % 1000 == 0 {
                    format!("Needle Song {i}")
                } else {
                    format!("Track {i}")
                },
                grandparent_title: Some(format!("Artist {}", i % 500)),
                parent_title: Some(format!("Album {}", i % 20_000)),
                genres: vec!["Rock".into()],
                ..Default::default()
            })
            .collect();
        let catalog = Cache::default().prepare(CatalogSearch {
            tracks: Arc::new(tracks),
            ..Default::default()
        });
        let mut times = vec![];
        for _ in 0..6 {
            let start = std::time::Instant::now();
            let result = catalog.search("needle", || false);
            times.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(result.tracks.len(), 100);
            assert_eq!(result.tracks[0].rating_key, "track-0");
        }
        let cold = times.remove(0);
        times.sort_by(f64::total_cmp);
        println!(
            "250k catalog search: cold={cold:.3}ms warm_median={:.3}ms",
            times[times.len() / 2]
        );
    }
}
