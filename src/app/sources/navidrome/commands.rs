//! Provider operations shared by the sidebar and command palette. Browse
//! collections live in the sidebar; the palette exposes actions, not a second
//! copy of those navigation destinations.
use super::*;
use crate::app::action::SystemAction;
use crate::app::state::{ConfirmAction, ConfirmDialog, PaletteCommandKind, PaletteEntry};
use crate::navidrome::{Song, array};

#[derive(Debug, Clone)]
pub enum Command {
    Analysis,
    Collection(CollectionKind),
    Star(bool),
    Rating(u8),
    Lyrics,
    SaveQueue,
    RestoreQueue,
    ChoosePlaylist(playlist_edit::Addition),
    PromptNewPlaylist(Option<playlist_edit::Addition>),
    CreatePlaylist {
        generation: u64,
        name: String,
        addition: Option<playlist_edit::Addition>,
    },
    Append {
        playlist: String,
        addition: playlist_edit::Addition,
    },
    EditEntry {
        playlist: String,
        index: usize,
        entries: Vec<(usize, String)>,
        edit: playlist_edit::Edit,
    },
    RenamePlaylist(String),
    SetPlaylistName {
        id: String,
        name: String,
    },
    ConfirmDeletePlaylist(String),
    DeletePlaylist(String),
    ConfirmReplacePlaylist(String),
    ReplacePlaylist(String),
}

/// Server-maintained views, not saved playlists and never playlist mutation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "feature", rename_all = "snake_case")]
pub enum CollectionKind {
    AudioMuse(crate::audiomuse::Feature),
    RecentlyPlayed,
    RecentlyAdded,
    MostPlayed,
    FavoriteSongs,
    FavoriteAlbums,
    FavoriteArtists,
    RandomMix,
    RandomAlbums,
    TopRatedAlbums,
}
impl CollectionKind {
    pub const SIDEBAR: [Self; 6] = [
        Self::RecentlyPlayed,
        Self::RecentlyAdded,
        Self::MostPlayed,
        Self::FavoriteSongs,
        Self::FavoriteAlbums,
        Self::FavoriteArtists,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AudioMuse(feature) => feature.label(),
            Self::RecentlyPlayed => "Recently played albums",
            Self::RecentlyAdded => "Recently added albums",
            Self::MostPlayed => "Most played albums",
            Self::FavoriteSongs => "Favorite songs",
            Self::FavoriteAlbums => "Favorite albums",
            Self::FavoriteArtists => "Favorite artists",
            Self::RandomMix => "Random song mix",
            Self::RandomAlbums => "Random albums",
            Self::TopRatedAlbums => "Top rated albums",
        }
    }
    pub fn action(self) -> Action {
        NavAction::Command(Command::Collection(self)).into()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Collection {
    Artists(Vec<crate::library::models::Artist>),
    Albums(Vec<crate::library::models::Album>),
    Tracks(Vec<Track>),
}

fn favorite_target(state: &AppState) -> Option<(&'static str, String)> {
    if let Some(track) = state.palette_target_track() {
        return Some(("id", track.rating_key));
    }
    if state.view != View::Browse || state.category_column_focused {
        return None;
    }
    if let Some((key, _)) = state.focused_album() {
        return Some(("albumId", key));
    }
    helpers::get_artist_for_bio(state).map(|(key, _)| ("artistId", key))
}

pub async fn load_collection(
    session: &Session,
    kind: CollectionKind,
) -> anyhow::Result<Collection> {
    use CollectionKind::*;
    let ticket = collection_ticket(session, kind);
    let mut params = session.client.scope();
    let collection = match kind {
        AudioMuse(_) => anyhow::bail!("AudioMuse collections use the analysis service"),
        FavoriteSongs | FavoriteAlbums | FavoriteArtists => {
            let response = session.client.call("getStarred2", &params).await?;
            let starred = &response["starred2"];
            match kind {
                FavoriteSongs => Collection::Tracks(
                    array::<Song>(starred, "song")?
                        .into_iter()
                        .map(|s| session.track(s))
                        .collect(),
                ),
                FavoriteAlbums => Collection::Albums(
                    array::<crate::navidrome::Album>(starred, "album")?
                        .into_iter()
                        .map(|a| session.album(a))
                        .collect(),
                ),
                _ => Collection::Artists(
                    array::<crate::navidrome::Artist>(starred, "artist")?
                        .into_iter()
                        .map(|a| session.artist(a))
                        .collect(),
                ),
            }
        }
        RandomMix => {
            params.push(("size", "100".into()));
            let response = session.client.call("getRandomSongs", &params).await?;
            Collection::Tracks(
                array::<Song>(&response["randomSongs"], "song")?
                    .into_iter()
                    .map(|s| session.track(s))
                    .collect(),
            )
        }
        RecentlyPlayed | RecentlyAdded | MostPlayed | RandomAlbums | TopRatedAlbums => {
            let kind = match kind {
                RecentlyPlayed => "recent",
                MostPlayed => "frequent",
                RandomAlbums => "random",
                TopRatedAlbums => "highest",
                _ => "newest",
            };
            params.extend([("type", kind.into()), ("size", "100".into())]);
            let response = session.client.call("getAlbumList2", &params).await?;
            Collection::Albums(
                array::<crate::navidrome::Album>(&response["albumList2"], "album")?
                    .into_iter()
                    .map(|a| session.album(a))
                    .collect(),
            )
        }
    };
    if let Some(ticket) = ticket {
        let saved = collection.clone();
        if let Err(error) = crate::app::tasks::spawn_blocking(move || ticket.write(&saved)).await? {
            tracing::warn!("Save album destination cache: {error}");
        }
    }
    Ok(collection)
}

fn collection_ticket(
    session: &Session,
    kind: CollectionKind,
) -> Option<crate::library::cache::Ticket> {
    use CollectionKind::*;
    let key = match kind {
        RecentlyPlayed => "albums-recently-played",
        RecentlyAdded => "albums-recently-added",
        MostPlayed => "albums-most-played",
        FavoriteAlbums => "albums-favorites",
        RandomAlbums => "albums-random",
        TopRatedAlbums => "albums-top-rated",
        _ => return None,
    };
    Some(session.cache_store().ok()?.ticket(key))
}
pub(super) async fn cached_collection(
    session: &Session,
    kind: CollectionKind,
) -> Option<crate::library::cache::Hit<Collection>> {
    let ticket = collection_ticket(session, kind)?;
    crate::app::tasks::spawn_blocking(move || ticket.read(crate::library::cache::REFRESH_INTERVAL))
        .await
        .ok()?
        .ok()
        .flatten()
}

pub fn entries(state: &AppState) -> Vec<PaletteEntry> {
    if state.sources.active.navidrome().is_none() {
        return vec![];
    }
    let mut commands = vec![(
        CollectionKind::RandomMix.label(),
        Command::Collection(CollectionKind::RandomMix),
    )];
    commands.extend(crate::audiomuse::Feature::ALL.into_iter().map(|feature| {
        (
            feature.label(),
            Command::Collection(CollectionKind::AudioMuse(feature)),
        )
    }));
    commands.extend([
        ("Save playback queue to Navidrome", Command::SaveQueue),
        (
            "Restore playback queue from Navidrome",
            Command::RestoreQueue,
        ),
    ]);
    if state.view == View::Browse && !state.category_column_focused && !state.palette.player_context
    {
        if let Some((key, _)) = state.focused_album().filter(|(key, _)| {
            state
                .sources
                .active
                .navidrome()
                .is_some_and(|s| s.id(key).is_ok())
        }) {
            commands.push((
                "Add Album to Playlist…",
                Command::ChoosePlaylist(playlist_edit::Addition::Album(key)),
            ));
        }
        if state.browse_category == BrowseCategory::Playlists
            && let Some(col) = state.playlist_nav.focused()
            && col.header_selection == crate::app::state::ColumnHeaderSelection::Items
        {
            let cursor = if col.selected_set.contains(&col.selected_index) {
                col.selected_index
            } else {
                col.selected_set
                    .first()
                    .copied()
                    .unwrap_or(col.selected_index)
            };
            for edit in playlist_edit::Edit::ALL {
                if let Some(command) = playlist_edit::row_command(state, col, cursor, edit) {
                    commands.push((edit.label(), command));
                }
            }
        }
    }
    if favorite_target(state).is_some() {
        commands.extend([
            ("Favorite selection", Command::Star(true)),
            ("Unfavorite selection", Command::Star(false)),
        ]);
    }
    if state.palette_target_track().is_some() {
        commands.push(("AudioMuse analysis", Command::Analysis));
        commands.push(("Server Lyrics", Command::Lyrics));
        for (label, rating) in [
            ("Clear song rating", 0),
            ("Rate song 1 star", 1),
            ("Rate song 2 stars", 2),
            ("Rate song 3 stars", 3),
            ("Rate song 4 stars", 4),
            ("Rate song 5 stars", 5),
        ] {
            commands.push((label, Command::Rating(rating)));
        }
    }
    if state.browse_category == BrowseCategory::Playlists
        && let Some(crate::app::state::BrowseItem::Playlist { key, .. }) = state
            .playlist_nav
            .columns
            .first()
            .and_then(|c| c.items.get(c.selected_index))
    {
        commands.extend([
            ("Rename playlist", Command::RenamePlaylist(key.clone())),
            (
                "Delete playlist",
                Command::ConfirmDeletePlaylist(key.clone()),
            ),
            (
                "Replace playlist with queue",
                Command::ConfirmReplacePlaylist(key.clone()),
            ),
        ]);
    }
    commands
        .into_iter()
        .map(|(label, command)| PaletteEntry {
            label: label.into(),
            hint: if matches!(
                command,
                Command::Analysis | Command::Collection(CollectionKind::AudioMuse(_))
            ) {
                "AudioMuse"
            } else {
                "Navidrome"
            }
            .into(),
            command: PaletteCommandKind::Navidrome(command),
            aliases: vec![],
        })
        .collect()
}

pub fn dispatch(command: Command, state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        state.set_status("Choose a Navidrome library first".into());
        return;
    };
    match command.clone() {
        Command::ChoosePlaylist(addition) => playlist_edit::choose(state, &session, addition),
        Command::PromptNewPlaylist(addition) => {
            if state.downloads.is_offline()
                || addition.as_ref().is_some_and(|a| !a.belongs_to(&session))
            {
                return;
            }
            state.popups.set_input_dialog(Some(InputDialog {
                title: "New Playlist".into(),
                input: String::new().into(),
                action_type: InputDialogAction::CreatePlaylist {
                    generation: state.library_generation,
                    addition,
                },
            }));
        }
        Command::CreatePlaylist {
            generation,
            name,
            addition,
        } => {
            if generation == state.library_generation
                && !state.downloads.is_offline()
                && addition.as_ref().is_none_or(|a| a.belongs_to(&session))
            {
                effects::save_playlist(state, tx, session, name, addition);
            }
        }
        Command::Append { playlist, addition } => {
            if state.downloads.is_offline()
                || !state
                    .library
                    .playlists
                    .iter()
                    .any(|p| p.rating_key == playlist && !p.smart)
            {
                return;
            }
            effects::spawn(
                state,
                tx,
                "playlist-write",
                playlist_edit::save(session, playlist, playlist_edit::Change::Append(addition)),
            );
        }
        Command::EditEntry {
            playlist,
            index,
            entries,
            edit,
        } => {
            let Some(col) = state
                .playlist_nav
                .columns
                .iter()
                .find(|c| playlist_edit::editable(state, c) == Some(playlist.as_str()))
            else {
                return;
            };
            if !entries
                .iter()
                .all(|(i, key)| col.tracks.get(*i).is_some_and(|t| &t.rating_key == key))
            {
                return;
            }
            let expected = col.tracks.iter().map(|t| t.rating_key.clone()).collect();
            effects::spawn(
                state,
                tx,
                "playlist-write",
                playlist_edit::save(
                    session,
                    playlist,
                    playlist_edit::Change::Edit {
                        expected,
                        indices: entries.into_iter().map(|(i, _)| i).collect(),
                        index,
                        edit,
                    },
                ),
            );
        }
        Command::Analysis => {
            super::super::audiomuse::track_info(state, tx);
        }
        Command::Collection(CollectionKind::AudioMuse(feature)) => {
            super::super::audiomuse::open(feature, false, state, tx);
        }
        Command::ConfirmDeletePlaylist(id) | Command::ConfirmReplacePlaylist(id) => {
            let delete = matches!(command, Command::ConfirmDeletePlaylist(_));
            let title = state
                .library
                .playlists
                .iter()
                .find(|p| p.rating_key == id)
                .map(|p| p.title.as_str())
                .unwrap_or("playlist");
            state.popups.set_confirm_dialog(Some(ConfirmDialog {
                title: if delete {
                    "Delete server playlist?"
                } else {
                    "Replace server playlist?"
                }
                .into(),
                message: format!(
                    "{} “{title}” on Navidrome?",
                    if delete {
                        "Delete"
                    } else {
                        "Replace all tracks in"
                    }
                ),
                selected_yes: false,
                on_confirm: if delete {
                    ConfirmAction::NavidromeDeletePlaylist(id)
                } else {
                    ConfirmAction::NavidromeReplacePlaylist(id)
                },
            }));
        }
        Command::RenamePlaylist(id) => {
            let name = state
                .library
                .playlists
                .iter()
                .find(|p| p.rating_key == id)
                .map(|p| p.title.clone())
                .unwrap_or_default();
            state.popups.set_input_dialog(Some(InputDialog {
                title: "Playlist name".into(),
                input: name.into(),
                action_type: InputDialogAction::NavidromePlaylistName { id },
            }));
        }
        Command::Lyrics => {
            if let Some(track) = state.palette_target_track() {
                crate::app::lyrics::open(state, tx, track, false);
            }
        }
        Command::Star(starred) => {
            let Some((parameter, key)) = favorite_target(state) else {
                return;
            };
            effects::spawn(state, tx, "metadata-write", async move {
                session
                    .client
                    .call(
                        if starred { "star" } else { "unstar" },
                        &[(parameter, session.id(&key)?)],
                    )
                    .await?;
                Ok(vec![
                    SystemAction::SetStatus(
                        if starred {
                            "Favorite saved"
                        } else {
                            "Favorite removed"
                        }
                        .into(),
                    )
                    .into(),
                ])
            });
        }
        Command::Rating(rating) => {
            let Some(track) = state.palette_target_track() else {
                return;
            };
            effects::spawn(state, tx, "metadata-write", async move {
                anyhow::ensure!(rating <= 5, "Rating must be between 0 and 5");
                session
                    .client
                    .call(
                        "setRating",
                        &[
                            ("id", session.id(&track.rating_key)?),
                            ("rating", rating.to_string()),
                        ],
                    )
                    .await?;
                Ok(vec![SystemAction::SetStatus("Rating saved".into()).into()])
            });
        }
        Command::Collection(kind) => {
            state.sources.nav_tasks.remove("audiomuse");
            state.set_view(View::Browse);
            state.browse_category = BrowseCategory::Library;
            state.sources.nav_collection = Some(kind);
            state.category_column_focused = false;
            state.desktop_library_root = match kind {
                CollectionKind::FavoriteArtists => crate::app::state::LibraryRoot::Artists,
                CollectionKind::FavoriteSongs | CollectionKind::RandomMix => {
                    crate::app::state::LibraryRoot::Tracks
                }
                _ => crate::app::state::LibraryRoot::Albums,
            };
            state.artist_nav_request_id = state.artist_nav_request_id.wrapping_add(1);
            let request_id = state.artist_nav_request_id;
            state.artist_nav =
                crate::app::state::BrowseNavigationState::with_root(kind.label(), vec![]);
            state.artist_nav.loading = true;
            state.select_mode = false;
            state.scroll.browse = None;
            state.miller_scroll_manual = false;
            state.alphabet_strip_focused = false;
            state.track_pane_focused = false;
            state.list_filter.deactivate();
            state.category_column_index = state.category_rows().iter().position(|row|
                matches!(row, crate::app::state::CategoryRow::NavidromeCollection(k) if *k == kind))
                .unwrap_or(0);
            if kind == CollectionKind::RandomAlbums {
                super::random_albums::open(state, tx);
                return;
            }
            let events = tx.clone();
            effects::spawn_scoped(
                state,
                tx,
                "collection",
                move |generation, operation_id| async move {
                    if let Some(hit) = cached_collection(&session, kind).await {
                        let _ = events
                            .send(Event::Effect(
                                NavAction::CollectionPreview {
                                    generation,
                                    operation_id,
                                    request_id,
                                    kind,
                                    items: hit.value,
                                }
                                .into(),
                            ))
                            .await;
                    }
                    let items = load_collection(&session, kind).await?;
                    Ok(vec![
                        NavAction::List {
                            request_id,
                            kind,
                            items,
                        }
                        .into(),
                    ])
                },
            );
        }
        Command::SaveQueue => {
            let tracks = state.queue.tracks.clone();
            let current = state.current_track().map(|t| t.rating_key.clone());
            let position = state.playback.position_ms;
            effects::spawn(state, tx, "queue-write", async move {
                let mut params = Vec::new();
                for track in tracks {
                    params.push(("id", session.id(&track.rating_key)?));
                }
                if let Some(current) = current {
                    params.push(("current", session.id(&current)?));
                    params.push(("position", position.to_string()));
                }
                session.client.call("savePlayQueue", &params).await?;
                Ok(vec![
                    SystemAction::SetStatus("Playback queue saved".into()).into(),
                ])
            });
        }
        Command::RestoreQueue => {
            state.queue_play_request_id = state.queue_play_request_id.wrapping_add(1);
            let request_id = state.queue_play_request_id;
            effects::spawn(state, tx, "queue-read", async move {
                let response = session.client.call("getPlayQueue", &[]).await?;
                // A missing playQueue means the user has never saved one.
                let queue = &response["playQueue"];
                if queue.is_null() {
                    anyhow::bail!("No playback queue is saved on Navidrome");
                }
                let songs: Vec<Song> = array(queue, "entry")?;
                anyhow::ensure!(!songs.is_empty(), "The saved playback queue is empty");
                let index = songs
                    .iter()
                    .position(|t| Some(t.id.as_str()) == queue["current"].as_str())
                    .unwrap_or(0);
                let position = queue["position"].as_u64().unwrap_or(0);
                Ok(vec![
                    NavAction::RestoreQueue {
                        request_id,
                        tracks: songs.into_iter().map(|s| session.track(s)).collect(),
                        index,
                        position,
                    }
                    .into(),
                ])
            });
        }
        Command::DeletePlaylist(id)
        | Command::ReplacePlaylist(id)
        | Command::SetPlaylistName { id, .. } => {
            let queue = state.queue.tracks.clone();
            let command = command.clone();
            effects::spawn(state, tx, "playlist-write", async move {
                let id = session.id(&id)?;
                match command {
                    Command::DeletePlaylist(_) => {
                        session.client.call("deletePlaylist", &[("id", id)]).await?;
                    }
                    Command::SetPlaylistName { name, .. } => {
                        anyhow::ensure!(!name.trim().is_empty(), "Playlist name is empty");
                        session
                            .client
                            .call("updatePlaylist", &[("playlistId", id), ("name", name)])
                            .await?;
                    }
                    _ => {
                        anyhow::ensure!(
                            !queue.is_empty(),
                            "Queue is empty; playlist left unchanged"
                        );
                        let mut params = vec![("playlistId", id)];
                        for track in queue {
                            params.push(("songId", session.id(&track.rating_key)?));
                        }
                        session.client.call("createPlaylist", &params).await?;
                    }
                }
                Ok(vec![
                    SystemAction::SetStatus("Playlist updated".into()).into(),
                    crate::app::action::DataAction::LoadPlaylists.into(),
                ])
            });
        }
    }
}
