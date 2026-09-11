//! Provider operations shared by the sidebar and command palette. Browse
//! collections live in the sidebar; the palette exposes actions, not a second
//! copy of those navigation destinations.
use super::*;
use crate::app::action::SystemAction;
use crate::app::state::{ConfirmAction, ConfirmDialog, PaletteCommandKind, PaletteEntry};
use crate::navidrome::{array, Song};

#[derive(Debug, Clone)]
pub enum Command {
    Analysis,
    Collection(CollectionKind),
    Star(bool),
    Rating(u8),
    Lyrics,
    SaveQueue,
    RestoreQueue,
    RenamePlaylist(String),
    SetPlaylistName { id: String, name: String },
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
        }
    }
    pub fn action(self) -> Action {
        NavAction::Command(Command::Collection(self)).into()
    }
}

#[derive(Debug, Clone)]
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
    let mut params = session.client.scope();
    Ok(match kind {
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
        RecentlyPlayed | RecentlyAdded | MostPlayed => {
            let kind = match kind {
                RecentlyPlayed => "recent",
                MostPlayed => "frequent",
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
    })
}

pub fn entries(state: &AppState) -> Vec<PaletteEntry> {
    if state.sources.active.navidrome().is_none() {
        return vec![];
    }
    let mut commands = vec![(
        CollectionKind::RandomMix.label(),
        Command::Collection(CollectionKind::RandomMix),
    )];
    if super::super::sonic::enabled(state) && super::super::audiomuse::connection(state).is_some() {
        commands.extend(
            crate::audiomuse::Feature::ALL
                .into_iter()
                .filter(|feature| feature.is_search())
                .map(|feature| {
                    (
                        feature.label(),
                        Command::Collection(CollectionKind::AudioMuse(feature)),
                    )
                }),
        );
    }
    commands.extend([
        ("Save playback queue to Navidrome", Command::SaveQueue),
        (
            "Restore playback queue from Navidrome",
            Command::RestoreQueue,
        ),
    ]);
    if favorite_target(state).is_some() {
        commands.extend([
            ("Favorite selection", Command::Star(true)),
            ("Unfavorite selection", Command::Star(false)),
        ]);
    }
    if state.palette_target_track().is_some() {
        if super::super::sonic::enabled(state)
            && super::super::audiomuse::connection(state).is_some()
        {
            commands.push(("AudioMuse analysis", Command::Analysis));
        }
        commands.push(("Lyrics", Command::Lyrics));
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
    if state.browse_category == BrowseCategory::Playlists {
        if let Some(crate::app::state::BrowseItem::Playlist { key, .. }) = state
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
            state.popups.confirm_dialog = Some(ConfirmDialog {
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
            });
        }
        Command::RenamePlaylist(id) => {
            let name = state
                .library
                .playlists
                .iter()
                .find(|p| p.rating_key == id)
                .map(|p| p.title.clone())
                .unwrap_or_default();
            state.popups.input_dialog = Some(InputDialog {
                title: "Playlist name".into(),
                input: name.into(),
                action_type: InputDialogAction::NavidromePlaylistName { id },
            });
        }
        Command::Lyrics => {
            let Some(track) = state.palette_target_track() else {
                return;
            };
            state.sources.nav_request_id = state.sources.nav_request_id.wrapping_add(1);
            let request_id = state.sources.nav_request_id;
            state.popups.close_all();
            state.popups.text = Some(crate::app::state::TextPopup {
                title: format!("Lyrics · {}", track.title),
                text: "Loading…".into(),
                scroll: 0,
                request_id,
            });
            effects::spawn(state, tx, "lyrics", async move {
                let result = lyrics(&session, &track)
                    .await
                    .map_err(|e| format!("Lyrics: {e:#}"));
                Ok(vec![NavAction::Text { request_id, result }.into()])
            });
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
                Ok(vec![SystemAction::SetStatus(
                    if starred {
                        "Favorite saved"
                    } else {
                        "Favorite removed"
                    }
                    .into(),
                )
                .into()])
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
            effects::spawn(state, tx, "collection", async move {
                let items = load_collection(&session, kind).await?;
                Ok(vec![NavAction::List {
                    request_id,
                    kind,
                    items,
                }
                .into()])
            });
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
                    SystemAction::SetStatus("Playback queue saved".into()).into()
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
                Ok(vec![NavAction::RestoreQueue {
                    request_id,
                    tracks: songs.into_iter().map(|s| session.track(s)).collect(),
                    index,
                    position,
                }
                .into()])
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

async fn lyrics(session: &Session, track: &Track) -> anyhow::Result<String> {
    let text = if session.extensions.contains("songLyrics") {
        let response = session
            .client
            .call(
                "getLyricsBySongId",
                &[("id", session.id(&track.rating_key)?)],
            )
            .await?;
        let lyrics: Vec<serde_json::Value> = array(&response["lyricsList"], "structuredLyrics")?;
        let mut text = String::new();
        for lyric in lyrics {
            for line in array::<serde_json::Value>(&lyric, "line")? {
                if let Some(value) = line["value"].as_str() {
                    text.push_str(value);
                    text.push('\n');
                }
            }
            text.push('\n');
        }
        text
    } else {
        let response = session
            .client
            .call(
                "getLyrics",
                &[
                    ("artist", track.track_artist().into()),
                    ("title", track.title.clone()),
                ],
            )
            .await?;
        response["lyrics"]["value"]
            .as_str()
            .unwrap_or_default()
            .into()
    };
    anyhow::ensure!(
        !text.trim().is_empty(),
        "No lyrics are available for this song"
    );
    Ok(crate::util::sanitize_display_text(&text).into_owned())
}
