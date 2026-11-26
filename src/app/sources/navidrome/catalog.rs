use super::*;
use crate::library::catalog::{Album, Artist, Playlist};
use crate::library::models::Genre;
use crate::library::track::Track;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    artists: Vec<Artist>,
    pub(crate) albums: Vec<Album>,
    tracks: Vec<Track>,
    pub(crate) playlists: Vec<Playlist>,
    libraries: Vec<crate::navidrome::MusicFolder>,
    extensions: std::collections::HashSet<String>,
}

/// Catalog and derived browsing data arrive together, under the same request guard.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedCatalog {
    catalog: Catalog,
    artists: crate::services::compilations::ArtistIndex,
    lookup: crate::services::catalog_lookup::TrackLookup,
    #[serde(skip)]
    availability: crate::services::radio::Availability,
}

impl PreparedCatalog {
    fn browse(&self) -> Self {
        Self {
            catalog: Catalog {
                artists: self.catalog.artists.clone(),
                albums: self.catalog.albums.clone(),
                playlists: self.catalog.playlists.clone(),
                libraries: self.catalog.libraries.clone(),
                extensions: self.catalog.extensions.clone(),
                tracks: self.catalog.tracks.iter().take(500).cloned().collect(),
            },
            artists: self.artists.clone(),
            lookup: Default::default(),
            availability: Default::default(),
        }
    }
    pub(crate) fn save_browse(
        &self,
        ticket: Option<crate::library::cache::Ticket>,
        timestamp: u64,
    ) {
        let Some(ticket) = ticket else {
            return;
        };
        if let Err(error) = ticket.write_at(&self.browse(), timestamp) {
            tracing::warn!(%error, "Save browse catalog cache");
        }
    }
}

fn read_browse_catalog(
    ticket: &crate::library::cache::Ticket,
    legacy: Option<(crate::library::cache::Ticket, String)>,
) -> anyhow::Result<Option<crate::library::cache::Hit<PreparedCatalog>>> {
    let ttl = crate::library::cache::REFRESH_INTERVAL;
    if let Some(hit) = ticket.read(ttl)? {
        return Ok(Some(hit));
    }
    let Some((legacy, folder)) = legacy else {
        return Ok(None);
    };
    Ok(legacy.read::<PreparedCatalog>(ttl)?.filter(
        |hit| matches!(hit.value.catalog.libraries.as_slice(), [only] if only.id == folder),
    ))
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
    if let Some(hit) = &hit
        && let Err(error) = ticket.write_at(&hit.value, hit.timestamp)
    {
        tracing::warn!("Migrate Navidrome cache: {error}");
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
    if state.notifications.error_target
        == Some(crate::app::state::NotificationTarget::RefreshLibrary)
    {
        state.clear_error();
    }
    let restore_browse = !matches!(
        state.library.catalog_loading,
        crate::app::catalog_loading::State::Ready
    );
    if restore_browse {
        state.library.catalog_preview_tracks = None;
        state.library.catalog_loading =
            crate::app::catalog_loading::State::Browsing { pending: None };
    }
    state.sources.listing = None;
    state.sources.list_id = state.sources.list_id.wrapping_add(1);
    let request_id = state.sources.list_id;
    let generation = state.library_generation;
    state.library_loading = true;
    state.library.artists_loading = true;
    state.library.playlists_loading = true;
    let tx = tx.clone();
    let store = session.cache_store();
    let browse_ticket = store.as_ref().ok().map(|s| s.ticket("browse-catalog-v1"));
    let playlists = store.as_ref().ok().map(|s| s.ticket("playlist-list"));
    let ticket = store.map(|s| s.ticket("catalog"));
    // Account-wide snapshots are valid for a sole folder only when their stored directory agrees.
    let legacy_store = match session.source.libraries.as_slice() {
        [only] if session.client.folder.as_ref() == Some(&only.id) => {
            crate::library::cache::Store::new((
                "navidrome",
                &session.source.id,
                session.source.url.trim_end_matches('/'),
                &session.source.username,
                None::<String>,
            ))
            .ok()
            .map(|store| (store, only.id.clone()))
        }
        _ => None,
    };
    let legacy = legacy_store
        .as_ref()
        .map(|(store, folder)| (store.ticket("catalog"), folder.clone()));
    let legacy_browse =
        legacy_store.map(|(store, folder)| (store.ticket("browse-catalog-v1"), folder));
    let task = crate::app::tasks::spawn(async move {
        let started = std::time::Instant::now();
        let mut browse = restore_browse.then(|| BrowseRequest {
            generation,
            request_id,
            events: tx.clone(),
        });
        let ticket = match ticket {
            Ok(ticket) => Some(ticket),
            Err(error) => {
                tracing::warn!("Navidrome cache: {error}");
                None
            }
        };
        let mut restored_browse = None;
        if !force {
            let browse_playlists = playlists.clone();
            if let Some(ticket) = browse_ticket.clone().filter(|_| restore_browse)
                && let Ok(Ok(Some(hit))) = crate::app::tasks::spawn_blocking(move || {
                    let mut hit = read_browse_catalog(&ticket, legacy_browse)?;
                    if let Some(hit) = &mut hit {
                        restore_playlists(
                            browse_playlists.as_ref(),
                            &mut hit.value.catalog.playlists,
                        );
                    }
                    Ok::<_, anyhow::Error>(hit)
                })
                .await
            {
                tracing::info!(
                    elapsed_ms = started.elapsed().as_millis(),
                    artists = hit.value.catalog.artists.len(),
                    albums = hit.value.catalog.albums.len(),
                    "Navidrome browse cache ready"
                );
                restored_browse = Some(hit.timestamp);
                browse = None;
                let _ = tx
                    .send(Event::Effect(
                        NavAction::BrowseCatalog {
                            generation,
                            request_id,
                            catalog: hit.value,
                        }
                        .into(),
                    ))
                    .await;
            }
            if let Some(ticket) = ticket.clone() {
                let browse_ticket = browse_ticket.clone();
                match crate::app::tasks::spawn_blocking(move || {
                    read_catalog(&ticket, legacy).map(|hit| {
                        hit.map(|mut hit| {
                            restore_playlists(playlists.as_ref(), &mut hit.value.playlists);
                            let value = hit.value.prepare();
                            if restored_browse != Some(hit.timestamp) {
                                value.save_browse(browse_ticket, hit.timestamp);
                            }
                            crate::library::cache::Hit {
                                value,
                                stale: hit.stale,
                                timestamp: hit.timestamp,
                            }
                        })
                    })
                })
                .await
                {
                    Ok(Ok(Some(hit))) => {
                        tracing::info!(
                            elapsed_ms = started.elapsed().as_millis(),
                            tracks = hit.value.catalog.tracks.len(),
                            stale = hit.stale,
                            "Navidrome full cache ready"
                        );
                        browse = None;
                        let refreshing = hit.stale;
                        if tx
                            .send(Event::Effect(
                                NavAction::Catalog {
                                    from_cache: true,
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
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "Navidrome network refresh started"
        );
        let result = fetch(&mut session, browse).await;
        // Transfer the catalog to the worker and back instead of cloning a
        // potentially enormous library solely to serialize it.
        let result = match (ticket, result) {
            (ticket, Ok(catalog)) => {
                match crate::app::tasks::spawn_blocking(move || {
                    let prepared = catalog.prepare();
                    let saved = ticket
                        .map(|ticket| ticket.write(&prepared.catalog))
                        .transpose();
                    prepared.save_browse(browse_ticket, crate::library::cache::now());
                    (prepared, saved)
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
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            success = result.is_ok(),
            "Navidrome catalog load finished"
        );
        let _ = tx
            .send(Event::Effect(
                NavAction::Catalog {
                    from_cache: false,
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

/// The active catalog request may publish browsing metadata before all tracks arrive.
pub(crate) struct BrowseRequest {
    generation: u64,
    request_id: u64,
    events: mpsc::Sender<Event>,
}

/// Fetch a complete catalog while optionally making its browsing roots available early.
pub(crate) async fn fetch(
    session: &mut Session,
    browse: Option<BrowseRequest>,
) -> Result<Catalog, String> {
    let started = std::time::Instant::now();
    // Individual requests have deadlines and pagination has a finite page limit.
    // A large catalog may keep making progress for longer than five minutes.
    async {
        super::discover(session).await?;
        let preview_request = browse.as_ref().map(|request| {
            (
                request.generation,
                request.request_id,
                request.events.clone(),
            )
        });
        let preview_session = session.clone();
        let metadata = async {
            let (artists, albums, playlists) = tokio::try_join!(
                session.client.artists(),
                session.client.albums("alphabeticalByName", &[]),
                fetch_playlists(session)
            )?;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                artists = artists.len(),
                albums = albums.len(),
                "Navidrome browse metadata fetched"
            );
            let catalog = Catalog {
                artists: artists.into_iter().map(|a| session.artist(a)).collect(),
                albums: albums.into_iter().map(|a| session.album(a)).collect(),
                tracks: vec![],
                playlists,
                libraries: session.source.libraries.clone(),
                extensions: session.extensions.clone(),
            };
            if let Some(browse) = browse {
                let (catalog, preview) = crate::app::tasks::spawn_blocking(move || {
                    let prepared = catalog.prepare();
                    let preview = prepared.browse();
                    (prepared.catalog, preview)
                })
                .await?;
                // Partial track data never replaces the persistent complete catalog.
                let _ = browse
                    .events
                    .send(Event::Effect(
                        NavAction::BrowseCatalog {
                            generation: browse.generation,
                            request_id: browse.request_id,
                            catalog: preview,
                        }
                        .into(),
                    ))
                    .await;
                Ok::<_, anyhow::Error>(catalog)
            } else {
                Ok(catalog)
            }
        };
        let (mut catalog, mut songs) = tokio::try_join!(
            metadata,
            session.client.songs_with_first_page("", move |first| {
                if let Some((generation, request_id, events)) = preview_request {
                    let tracks = first
                        .into_iter()
                        .map(|song| preview_session.track(song))
                        .collect();
                    let _ = events.try_send(Event::Effect(
                        NavAction::TrackPreview {
                            generation,
                            request_id,
                            tracks,
                        }
                        .into(),
                    ));
                }
            })
        )?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            tracks = songs.len(),
            "Navidrome track catalog fetched"
        );
        let playlist_ticket = session
            .cache_store()
            .ok()
            .map(|s| s.ticket("playlist-list"));
        let session = session.clone();
        let catalog = crate::app::tasks::spawn_blocking(move || {
            songs.sort_by(|a, b| {
                a.album_id
                    .cmp(&b.album_id)
                    .then_with(|| a.album_order().cmp(&b.album_order()))
            });
            restore_playlists(playlist_ticket.as_ref(), &mut catalog.playlists);
            catalog.tracks = songs.into_iter().map(|song| session.track(song)).collect();
            catalog
        })
        .await?;
        Ok(catalog)
    }
    .await
    .map_err(|e: anyhow::Error| format!("{e:#}"))
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
        self.artists
            .sort_by_cached_key(|a| helpers::sort_key(&a.title));
        self.albums
            .sort_by_cached_key(|a| helpers::sort_key(&a.title));
        self.playlists
            .sort_by_cached_key(|a| helpers::sort_key(&a.title));
        let lookup = crate::services::catalog_lookup::TrackLookup::build(
            &self.artists,
            &artists.track_artists,
            &self.albums,
            &self.tracks,
        );
        PreparedCatalog {
            availability: crate::services::radio::Availability::build(&self.tracks, &self.albums),
            lookup,
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
    let PreparedCatalog {
        catalog,
        artists,
        lookup,
        availability,
    } = prepared;
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
    state.library.artists = std::sync::Arc::new(catalog.artists);
    state.library.albums = std::sync::Arc::new(catalog.albums);
    state.library.all_tracks = std::sync::Arc::new(catalog.tracks);
    state.library.track_lookup = lookup;
    state.library.radio_availability = availability.into();
    state.library.audio_mood_cache = Default::default();
    state.library.visited_album_tracks.clear();
    state.replace_playlists(catalog.playlists);
    super::radio::load(state);
    state.library.compilations = artists.compilations;
    state.library.track_artists = artists.track_artists;
    state.library.artist_aliases = std::sync::Arc::new(artists.aliases);
    state.library.album_display_artist = artists.album_display;
    super::album_artists::reconcile_compilation_index(state);
    let items = state.build_artist_root_items();
    if state.desktop_library_root == crate::app::state::LibraryRoot::Singles {
        crate::app::album_collections::refresh_singles(state);
    } else if state.sources.nav_collection.is_none()
        && state.desktop_library_root != crate::app::state::LibraryRoot::Downloads
    {
        state.artist_nav.update_root_items("artists", items);
    }
    // An aggregate destination may have been opened before cached metadata arrived.
    for column in state.artist_nav.columns.iter_mut().skip(1) {
        if column.title == "all albums"
            && !column
                .items
                .iter()
                .any(|item| matches!(item, crate::app::state::BrowseItem::Album { .. }))
        {
            let mut items = crate::app::state::BrowseItem::from_albums(
                &state.library.albums,
                &state.library.album_display_artist,
            );
            items.push(crate::app::state::BrowseItem::AllTracks {
                scope: crate::app::state::AllTracksScope::Library,
                thumb: None,
            });
            column.items = items;
            column.selected_index = 0;
        } else if column.title == "all tracks" {
            column.replace_content(
                crate::app::state::BrowseItem::from_tracks(&state.library.all_tracks),
                state.library.all_tracks.as_ref().clone(),
            );
        }
    }
    state.category_column_index = state
        .category_column_index
        .min(state.category_rows().len().saturating_sub(1));
    state.library.album_genres = std::sync::Arc::new(tags(
        state
            .library
            .albums
            .iter()
            .flat_map(|a| a.genre.iter().map(|g| g.tag.clone())),
    ));
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

/// Playlist edits update this small snapshot independently of the track index.
/// Catalogs without a separate snapshot supply their embedded playlist list.
fn restore_playlists(
    ticket: Option<&crate::library::cache::Ticket>,
    playlists: &mut Vec<Playlist>,
) {
    if let Some(ticket) = ticket {
        match ticket.read(crate::library::cache::REFRESH_INTERVAL) {
            Ok(Some(hit)) => *playlists = hit.value,
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "Read cached playlist list"),
        }
    }
}

async fn fetch_playlists(session: &Session) -> anyhow::Result<Vec<Playlist>> {
    let ticket = session
        .cache_store()
        .map(|store| store.ticket("playlist-list"));
    let playlists: Vec<_> = session
        .client
        .playlists()
        .await?
        .into_iter()
        .map(|item| session.playlist(item))
        .collect();
    crate::app::tasks::spawn_blocking(move || {
        if let Err(error) = ticket.and_then(|ticket| ticket.write(&playlists)) {
            tracing::warn!(%error, "Persist playlist metadata");
        }
        playlists
    })
    .await
    .map_err(Into::into)
}

pub fn refresh_playlists(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    effects::spawn(state, tx, "playlist-list", async move {
        let playlists = fetch_playlists(&session).await?;
        Ok(vec![
            crate::app::action::DataAction::PlaylistsLoaded(playlists).into(),
        ])
    });
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn browse_cache_restores_roots_and_a_bounded_track_preview() {
        let dir = tempfile::tempdir().unwrap();
        let ticket = crate::library::cache::Store::at(dir.path().into(), "browse-fixture")
            .unwrap()
            .ticket("browse-catalog-v1");
        let catalog = Catalog {
            artists: vec![Artist {
                rating_key: "artist".into(),
                title: "Artist".into(),
                ..Default::default()
            }],
            albums: vec![Album {
                rating_key: "album".into(),
                title: "Album".into(),
                parent_rating_key: Some("artist".into()),
                ..Default::default()
            }],
            tracks: vec![Track {
                rating_key: "song".into(),
                parent_rating_key: Some("album".into()),
                ..Default::default()
            }],
            playlists: vec![],
            libraries: vec![],
            extensions: Default::default(),
        }
        .prepare();
        let timestamp =
            crate::library::cache::now() - crate::library::cache::REFRESH_INTERVAL.as_secs() - 1;
        catalog.save_browse(Some(ticket.clone()), timestamp);
        let hit = ticket
            .read::<PreparedCatalog>(crate::library::cache::REFRESH_INTERVAL)
            .unwrap()
            .unwrap();
        assert_eq!(hit.timestamp, timestamp);
        assert!(hit.stale);
        let restored = hit.value;
        assert_eq!(restored.catalog.artists.len(), 1);
        assert_eq!(restored.catalog.albums.len(), 1);
        assert_eq!(restored.catalog.tracks.len(), 1);
        assert_eq!(restored.lookup.len, 0);
        let mut state = AppState::new();
        install(&mut state, restored);
        assert!(
            state
                .build_artist_root_items()
                .iter()
                .any(|item| item.key() == "artist")
        );
        install(&mut state, catalog);
        assert_eq!(state.library.all_tracks.len(), 1);
    }

    #[test]
    fn compact_sole_folder_cache_is_usable_before_full_catalog_migration() {
        let dir = tempfile::tempdir().unwrap();
        let current = crate::library::cache::Store::at(dir.path().into(), ("nav", Some("1")))
            .unwrap()
            .ticket("browse-catalog-v1");
        let legacy = crate::library::cache::Store::at(dir.path().into(), ("nav", None::<String>))
            .unwrap()
            .ticket("browse-catalog-v1");
        let compact = Catalog {
            artists: vec![],
            albums: vec![],
            tracks: vec![],
            playlists: vec![],
            libraries: vec![crate::navidrome::MusicFolder {
                id: "1".into(),
                name: "Music".into(),
            }],
            extensions: Default::default(),
        }
        .prepare()
        .browse();
        legacy.write_at(&compact, 123).unwrap();
        assert!(
            read_browse_catalog(&current, Some((legacy.clone(), "other".into())))
                .unwrap()
                .is_none()
        );
        let hit = read_browse_catalog(&current, Some((legacy, "1".into())))
            .unwrap()
            .unwrap();
        assert_eq!(hit.timestamp, 123);
        assert!(hit.stale);
        assert!(hit.value.catalog.tracks.is_empty());
        assert!(read_browse_catalog(&current, None).unwrap().is_none());
    }

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
        assert!(
            read_catalog(&new, Some((old.clone(), "other".into())))
                .unwrap()
                .is_none()
        );
        catalog.libraries.push(crate::navidrome::MusicFolder {
            id: "other".into(),

            name: "Other".into(),
        });
        old.write(&catalog).unwrap();
        assert!(
            read_catalog(&new, Some((old.clone(), "1".into())))
                .unwrap()
                .is_none()
        );
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

#[cfg(test)]
mod startup_navigation_tests {
    use super::*;
    #[test]
    fn catalog_arrival_fills_an_aggregate_column_opened_during_startup() {
        let mut state = AppState::new();
        state.artist_nav = crate::app::state::BrowseNavigationState::with_root("artists", vec![]);
        state
            .artist_nav
            .columns
            .push(crate::app::state::BrowseColumn::new(
                "all albums",
                vec![crate::app::state::BrowseItem::AllTracks {
                    scope: crate::app::state::AllTracksScope::Library,
                    thumb: None,
                }],
            ));
        let catalog = Catalog {
            albums: vec![Album {
                rating_key: "album".into(),
                title: "Album".into(),
                ..Default::default()
            }],
            artists: vec![],
            tracks: vec![],
            playlists: vec![],
            libraries: vec![],
            extensions: Default::default(),
        };
        install(&mut state, catalog.prepare());
        assert!(
            state.artist_nav.columns[1]
                .items
                .iter()
                .any(|item| item.key() == "album")
        );
    }
}

/// Retain an early page until its catalog metadata has been published.
pub(crate) fn publish_preview(state: &mut AppState) -> Vec<Action> {
    if !state.library.artists_loading {
        if state.library.all_tracks.is_empty() {
            if let Some(tracks) = state.library.catalog_preview_tracks.take() {
                state.library.all_tracks = std::sync::Arc::new(tracks);
            }
        } else {
            state.library.catalog_preview_tracks = None;
        }
        return crate::app::catalog_loading::preview_ready(state)
            .into_iter()
            .collect();
    }
    vec![]
}

#[cfg(test)]
mod track_refresh_tests {
    use super::*;
    #[test]
    fn full_track_catalog_preserves_column_options_and_selected_identities() {
        use crate::app::state::{BrowseColumn, BrowseItem, ColumnSortMode, PlayAllRow};
        let track = |key: &str, title: &str| Track {
            rating_key: key.into(),
            title: title.into(),
            ..Default::default()
        };
        let tracks = vec![track("b", "Beta"), track("a", "Alpha")];
        let mut column =
            BrowseColumn::new_with_tracks("all tracks", BrowseItem::from_tracks(&tracks), tracks);
        column.apply_sort(ColumnSortMode::ByTitle);
        column.selected_index = 1;
        column.selected_set.insert(1);
        column.selection_anchor = Some(1);
        column.artwork_visible = true;
        column.play_all_row = Some(PlayAllRow::AllTracks {
            label: "All Tracks".into(),
        });
        let updated = vec![
            track("c", "Charlie"),
            track("b", "Beta"),
            track("a", "Alpha"),
        ];
        column.replace_content(BrowseItem::from_tracks(&updated), updated);
        assert_eq!(column.selected_item().unwrap().key(), "b");
        assert_eq!(
            column.items[*column.selected_set.first().unwrap()].key(),
            "b"
        );
        assert_eq!(column.items[column.selection_anchor.unwrap()].key(), "b");
        assert_eq!(column.sort_mode, ColumnSortMode::ByTitle);
        assert!(column.artwork_visible && column.play_all_row.is_some());
        assert_eq!(
            column
                .items
                .iter()
                .map(|item| item.key())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }
}
