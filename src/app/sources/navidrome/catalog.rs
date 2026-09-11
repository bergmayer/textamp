use super::*;
use crate::library::catalog::{Album, Artist, Playlist};
use crate::library::models::Genre;
use crate::library::track::Track;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    artists: Vec<Artist>,
    albums: Vec<Album>,
    tracks: Vec<Track>,
    playlists: Vec<Playlist>,
    libraries: Vec<crate::navidrome::MusicFolder>,
    extensions: std::collections::HashSet<String>,
}

/// Catalog and derived browsing data arrive together, under the same request guard.
#[derive(Debug, Clone)]
pub struct PreparedCatalog {
    catalog: Catalog,
    artists: crate::services::compilations::ArtistIndex,
}

fn read_catalog(
    ticket: &crate::library::cache::Ticket,
    legacy: Option<(crate::library::cache::Ticket, String)>,
) -> anyhow::Result<Option<crate::library::cache::Hit<Catalog>>> {
    let ttl = crate::library::cache::REFRESH_INTERVAL;
    if let Some(hit) = ticket.read(ttl)? {
        return Ok(Some(hit));
    }
    let Some((legacy, folder)) = legacy else {
        return Ok(None);
    };
    let hit = legacy
        .read::<Catalog>(ttl)?
        .filter(|hit| matches!(hit.value.libraries.as_slice(), [only] if only.id == folder));
    if let Some(hit) = &hit {
        if let Err(error) = ticket.write_at(&hit.value, hit.timestamp) {
            tracing::warn!("Migrate Navidrome cache: {error}");
        }
    }
    Ok(hit)
}

pub fn load(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    start(state, tx, true);
}
pub fn open(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let category = crate::app::state::RefreshCategory::Artists;
    if state.sources.listing.is_some()
        || state.cache_mgmt.failures.contains_key(&category)
        || state
            .cache_mgmt
            .category_timestamps
            .get(&category)
            .is_some_and(|timestamp| {
                !crate::library::cache::refresh_due(*timestamp, crate::library::cache::now())
            })
    {
        return;
    }
    start(state, tx, false);
}
pub fn check_staleness(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let category = super::super::super::state::RefreshCategory::Artists;
    if state.sources.listing.is_none()
        && !state.cache_mgmt.failures.contains_key(&category)
        && state
            .cache_mgmt
            .category_timestamps
            .get(&category)
            .is_none_or(|timestamp| {
                crate::library::cache::refresh_due(*timestamp, crate::library::cache::now())
            })
    {
        open(state, tx);
    }
}
fn start(state: &mut AppState, tx: &mpsc::Sender<Event>, force: bool) {
    state.track_pane_similar.retain(|_, result| result.is_ok());
    let Some(mut session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    state.sources.listing = None;
    state.sources.list_id = state.sources.list_id.wrapping_add(1);
    let request_id = state.sources.list_id;
    let generation = state.library_generation;
    state.library_loading = true;
    state.library.artists_loading = true;
    state.library.playlists_loading = true;
    let tx = tx.clone();
    let ticket = session.cache_store().map(|store| store.ticket("catalog"));
    // Pre-deduplication builds cached the sole folder under the aggregate key.
    // Accept that cache only when its own directory confirms the same sole folder.
    let legacy = match session.source.libraries.as_slice() {
        [only] if session.client.folder.as_ref() == Some(&only.id) => {
            // Bypass canonical_folder here: this is the historical aggregate
            // identity, not the sole-folder identity used by current sessions.
            crate::library::cache::Store::new((
                "navidrome",
                &session.source.id,
                session.source.url.trim_end_matches('/'),
                &session.source.username,
                None::<String>,
            ))
            .ok()
            .map(|store| (store.ticket("catalog"), only.id.clone()))
        }
        _ => None,
    };
    let task = crate::app::tasks::spawn(async move {
        let ticket = match ticket {
            Ok(ticket) => Some(ticket),
            Err(error) => {
                tracing::warn!("Navidrome cache: {error}");
                None
            }
        };
        if !force {
            if let Some(ticket) = ticket.clone() {
                match crate::app::tasks::spawn_blocking(move || {
                    read_catalog(&ticket, legacy).map(|hit| {
                        hit.map(|hit| crate::library::cache::Hit {
                            value: hit.value.prepare(),
                            stale: hit.stale,
                            timestamp: hit.timestamp,
                        })
                    })
                })
                .await
                {
                    Ok(Ok(Some(hit))) => {
                        let refreshing = hit.stale;
                        if tx
                            .send(Event::Effect(
                                NavAction::Catalog {
                                    generation,
                                    request_id,
                                    refreshing,
                                    timestamp: hit.timestamp,
                                    result: Ok(hit.value),
                                }
                                .into(),
                            ))
                            .await
                            .is_err()
                            || !refreshing
                        {
                            return;
                        }
                    }
                    Ok(Ok(None)) => {}
                    other => tracing::warn!(
                        "Navidrome cache unavailable: {}",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            _ => unreachable!(),
                        }
                    ),
                }
            }
        }
        let result = fetch(&mut session).await;
        // Transfer the catalog to the worker and back instead of cloning a
        // potentially enormous library solely to serialize it.
        let result = match (ticket, result) {
            (ticket, Ok(catalog)) => {
                match crate::app::tasks::spawn_blocking(move || {
                    let saved = ticket.map(|ticket| ticket.write(&catalog)).transpose();
                    (catalog.prepare(), saved)
                })
                .await
                {
                    Ok((catalog, saved)) => {
                        if let Err(error) = saved {
                            tracing::warn!("Navidrome cache write: {error}");
                            let _ = tx
                                .send(Event::Effect(
                                    crate::app::action::SystemAction::ShowError(format!(
                                        "Save Navidrome cache: {error}"
                                    ))
                                    .into(),
                                ))
                                .await;
                        }
                        Ok(catalog)
                    }
                    Err(error) => Err(format!("Cache worker failed: {error}")),
                }
            }
            (_, Err(error)) => Err(error),
        };
        let _ = tx
            .send(Event::Effect(
                NavAction::Catalog {
                    generation,
                    request_id,
                    refreshing: false,
                    timestamp: crate::library::cache::now(),
                    result,
                }
                .into(),
            ))
            .await;
    });
    state.sources.listing = Some(TaskLease::new(&task));
}

/// Complete catalog fetch shared by normal loading and inactive-library cache scans.
pub(crate) async fn fetch(session: &mut Session) -> Result<Catalog, String> {
    tokio::time::timeout(std::time::Duration::from_secs(300), async {
        super::discover(session).await?;
        let (artists, albums, mut songs, playlists) = tokio::try_join!(
            session.client.artists(),
            session.client.albums("alphabeticalByName", &[]),
            session.client.songs(""),
            session.client.playlists()
        )?;
        songs.sort_by_key(|t| {
            (
                t.album_id.clone(),
                t.disc_number.unwrap_or(1),
                t.track.unwrap_or(0),
                t.title.clone(),
            )
        });
        Ok(Catalog {
            artists: artists.into_iter().map(|a| session.artist(a)).collect(),
            albums: albums.into_iter().map(|a| session.album(a)).collect(),
            tracks: songs.into_iter().map(|t| session.track(t)).collect(),
            playlists: playlists.into_iter().map(|p| session.playlist(p)).collect(),
            libraries: session.source.libraries.clone(),
            extensions: session.extensions.clone(),
        })
    })
    .await
    .map_err(|_| anyhow::anyhow!("Catalog request timed out"))
    .and_then(|r: anyhow::Result<Catalog>| r)
    .map_err(|e| format!("{e:#}"))
}
impl Catalog {
    /// Run on a blocking worker when loading either disk or server data.
    pub fn prepare(mut self) -> PreparedCatalog {
        let artists = crate::services::compilations::ArtistIndex::build(
            &self.artists,
            &self.albums,
            &self.tracks,
        );
        let existing: std::collections::HashSet<_> =
            self.artists.iter().map(|a| a.rating_key.clone()).collect();
        // Compilation performers may have no album-artist record on the
        // server. Keep them searchable; the root list hides compilation-only ones.
        self.artists.extend(
            artists
                .track_artists
                .iter()
                .filter(|a| {
                    (artists
                        .compilations
                        .single_artist
                        .contains_key(&a.rating_key)
                        || artists.compilations.artist_map.contains_key(&a.rating_key))
                        && !existing.contains(&a.rating_key)
                })
                .cloned(),
        );
        self.artists.sort_by_key(|a| helpers::sort_key(&a.title));
        self.albums.sort_by_key(|a| helpers::sort_key(&a.title));
        self.playlists.sort_by_key(|a| helpers::sort_key(&a.title));
        PreparedCatalog {
            catalog: self,
            artists,
        }
    }

    pub(crate) fn track_ids(&self, session: &Session) -> std::collections::HashSet<String> {
        self.tracks
            .iter()
            .filter_map(|track| session.id(&track.rating_key).ok())
            .collect()
    }
}

pub fn install(state: &mut AppState, prepared: PreparedCatalog) {
    let PreparedCatalog { catalog, artists } = prepared;
    let selected_playlist = state
        .playlist_nav
        .columns
        .first()
        .and_then(|col| col.selected_item())
        .map(|item| item.key().to_owned());
    if let ActiveSource::Navidrome(session) = &mut state.sources.active {
        session.source.libraries = catalog.libraries;
        session.client.folder = session
            .source
            .canonical_folder(session.client.folder.clone());
        state.active_library = Some(format!(
            "navidrome:{}:{}",
            session.source.id,
            session.client.folder.as_deref().unwrap_or("all")
        ));
        session.extensions = catalog.extensions;
    }
    state.library.artists_total = catalog.artists.len() as u32;
    state.library.albums_total = catalog.albums.len() as u32;
    state.library.artists = catalog.artists;
    state.library.albums = catalog.albums;
    state.library.all_tracks = catalog.tracks;
    state.library.playlists = catalog.playlists;
    super::radio::load(state);
    state.library.compilations = artists.compilations;
    state.library.track_artists = artists.track_artists;
    state.library.artist_aliases = artists.aliases;
    state.library.album_display_artist = artists.album_display;
    let items = state.build_artist_root_items();
    if state.sources.nav_collection.is_none() {
        state.artist_nav.update_root_items("artists", items);
    }
    state.playlist_nav.update_root_items(
        "playlists",
        crate::app::state::BrowseItem::from_playlists(&state.library.playlists),
    );
    if let Some(key) = selected_playlist {
        let root = &mut state.playlist_nav.columns[0];
        if let Some(index) = root.items.iter().position(|item| item.key() == key) {
            root.selected_index = index;
        } else {
            // A deleted playlist must not leave its cached tracks open under another name.
            state.sources.nav_tasks.remove("playlist-browse");
            state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
            state.playlist_nav.columns.truncate(1);
            state.playlist_nav.focused_column = 0;
            state.library.selected_album_title.clear();
        }
    }
    state.category_column_index = state
        .category_column_index
        .min(state.category_rows().len().saturating_sub(1));
    state.library.album_genres = tags(
        state
            .library
            .albums
            .iter()
            .flat_map(|a| a.genre.iter().map(|g| g.tag.clone())),
    );
    if state.browse_category == BrowseCategory::AlbumGenres {
        state.tag_nav.update_root_items(
            BrowseCategory::AlbumGenres.name(),
            crate::app::state::BrowseItem::from_genres(&state.library.album_genres),
        );
        state.tag_nav.loading = false;
    }
    state.clear_status();
}
fn tags(values: impl Iterator<Item = String>) -> Vec<Genre> {
    let values: std::collections::BTreeSet<_> = values.collect();
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort_by_cached_key(|name| name.to_lowercase());
    values
        .into_iter()
        .map(|title| Genre {
            key: title.clone(),
            title,

            count: None,
        })
        .collect()
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn sole_folder_reuses_legacy_cache_but_never_a_multi_folder_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let new = crate::library::cache::Store::at(dir.path().into(), ("nav", Some("1")))
            .unwrap()
            .ticket("catalog");
        let old = crate::library::cache::Store::at(dir.path().into(), ("nav", None::<String>))
            .unwrap()
            .ticket("catalog");
        let mut catalog = Catalog {
            artists: vec![],
            albums: vec![],
            tracks: vec![],
            playlists: vec![],
            libraries: vec![crate::navidrome::MusicFolder {
                id: "1".into(),
                name: "Music Library".into(),
            }],
            extensions: Default::default(),
        };
        old.write(&catalog).unwrap();
        assert!(read_catalog(&new, Some((old.clone(), "other".into())))
            .unwrap()
            .is_none());
        catalog.libraries.push(crate::navidrome::MusicFolder {
            id: "other".into(),

            name: "Other".into(),
        });
        old.write(&catalog).unwrap();
        assert!(read_catalog(&new, Some((old.clone(), "1".into())))
            .unwrap()
            .is_none());
        catalog.libraries.pop();
        old.write_at(&catalog, 123).unwrap();
        let hit = read_catalog(&new, Some((old, "1".into())))
            .unwrap()
            .unwrap();
        assert!(hit.stale);
        assert_eq!(hit.timestamp, 123);
        let migrated = read_catalog(&new, None).unwrap().unwrap();
        assert!(migrated.stale);
        assert_eq!(migrated.timestamp, 123);
    }
}
