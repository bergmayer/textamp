//! Navidrome's getArtistInfo2 supplies catalog artists for the open album.
use super::{NavAction, Session, effects};
use crate::services::biography::Biography;
use crate::{
    app::{AppState, Event, View, details, state::PlayAllRow},
    library::models::Artist,
};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub(crate) struct Related {
    pub generation: u64,
    pub album: String,
    pub artist: String,
    pub document: Option<Biography>,
}
fn reconcile_target(state: &AppState) -> Option<(&str, &crate::app::state::BrowseColumn)> {
    if state.view != View::Browse || state.downloads.is_offline() {
        return None;
    }
    state
        .browse_nav()?
        .columns
        .iter()
        .rev()
        .find_map(|column| match &column.play_all_row {
            Some(PlayAllRow::Album { rating_key, .. }) => Some((rating_key.as_str(), column)),
            _ => None,
        })
}
pub(crate) fn reconcile(state: &mut AppState, events: &mpsc::Sender<Event>) {
    let Some(((album, column), session)) =
        reconcile_target(state).zip(state.sources.active.navidrome())
    else {
        state.sources.album_artists = None;
        state.sources.nav_tasks.remove("album-artists");
        return;
    };
    if state
        .sources
        .album_artists
        .as_ref()
        .is_some_and(|r| r.generation == state.library_generation && r.album == album)
    {
        return;
    }
    let Some((artist, name)) = details::album_artist(state, column) else {
        return;
    };
    let album = album.to_owned();
    if let Some(r) = &mut state.sources.album_artists
        && r.generation == state.library_generation
        && r.artist == artist
    {
        r.album = album;
        return;
    }
    let header = state.browse_nav().and_then(|nav| {
        nav.columns
            .iter()
            .filter_map(|c| c.artist_header.as_ref())
            .find(|info| info.artist_key == artist)
    });
    if header.is_some_and(|info| info.task.is_some()) {
        return;
    }
    if let Some(document) = header
        .map(|info| &info.document)
        .or_else(|| state.artist_bios.get(&artist))
        .cloned()
    {
        state.sources.nav_tasks.remove("album-artists");
        state.sources.album_artists = Some(Related {
            generation: state.library_generation,
            album,
            artist,
            document: Some(document),
        });
        return;
    }
    let mbid = state
        .library
        .artists
        .iter()
        .chain(&state.library.track_artists)
        .find(|item| item.rating_key == artist)
        .and_then(|item| item.music_brainz_id.clone());
    let session = session.clone();
    state.sources.album_artists = Some(Related {
        generation: state.library_generation,
        album,
        artist: artist.clone(),
        document: None,
    });
    let generation = state.library_generation;
    let tx = events.clone();
    effects::spawn(state, events, "album-artists", async move {
        let document = crate::app::handlers::helpers::biography::load_navidrome(
            &session, &artist, &name, mbid, generation, &tx,
        )
        .await
        .unwrap_or_else(|error| Biography {
            text: error,
            unavailable: true,
            ..Default::default()
        });
        Ok(vec![NavAction::AlbumArtists { artist, document }.into()])
    });
}
/// Recommendation IDs come from Navidrome even when the local performer was
/// indexed from a compilation credit under a synthetic key.
pub(crate) fn filter_recommendations(
    library: &crate::app::state::LibraryData,
    document: &mut Biography,
) {
    let excluded = &library.compilations.artist_keys;
    document.related_artists.retain(|artist| {
        !excluded.contains(&artist.rating_key)
            && (!excluded.contains(&crate::services::compilations::performer_key(&artist.title))
                || library
                    .artists
                    .iter()
                    .any(|a| a.rating_key == artist.rating_key))
    });
}

/// A first import can deliver artist metadata before it has indexed compilation tracks.
pub(super) fn reconcile_compilation_index(state: &mut AppState) {
    if state.library.compilations.artist_keys.is_empty() {
        return;
    }
    let library = &state.library;
    let open_columns = [
        &mut state.artist_nav,
        &mut state.playlist_nav,
        &mut state.tag_nav,
    ]
    .into_iter()
    .flat_map(|nav| &mut nav.columns)
    .filter_map(|column| match &mut column.details {
        Some(crate::app::state::BrowseDetails::Artist(info)) => Some(info),
        _ => column.artist_header.as_mut(),
    })
    .map(|info| &mut info.document);
    for document in open_columns
        .chain(state.artist_bios.documents_mut())
        .chain(
            state
                .sources
                .album_artists
                .as_mut()
                .and_then(|related| related.document.as_mut()),
        )
        .chain(
            state
                .popups
                .artist_bio_mut()
                .map(|popup| &mut popup.document),
        )
    {
        filter_recommendations(library, document);
    }
}

pub(crate) fn parse(session: &Session, key: &str, info: &serde_json::Value) -> Vec<Artist> {
    let mut seen = std::collections::HashSet::new();
    info["similarArtist"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<crate::navidrome::Artist>(value.clone()).ok())
        .filter(|artist| {
            !artist.id.is_empty()
                && artist.id != "-1"
                && session.key(&artist.id) != key
                && !artist.name.trim().is_empty()
                && seen.insert(artist.id.clone())
        })
        .take(20)
        .map(|artist| session.artist(artist))
        .collect()
}
