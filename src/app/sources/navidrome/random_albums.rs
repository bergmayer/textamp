//! Session-scoped Random browsing, loaded when opened.
use super::*;
use crate::app::state::BrowseCategory;
use crate::library::models::Album;

#[derive(Debug, Clone, Default)]
pub enum State {
    #[default]
    Idle,
    RefreshPending,
    Loading,
    Ready(Vec<Album>),
    Failed,
}

fn load(state: &mut AppState, tx: &mpsc::Sender<Event>, force: bool) {
    if !matches!(state.sources.random_albums, State::Idle) {
        return;
    }
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let generation = state.library_generation;
    state.sources.random_albums = State::Loading;
    effects::spawn(state, tx, "random-albums", async move {
        let result = async {
            if !force
                && let Some(hit) =
                    commands::cached_collection(&session, commands::CollectionKind::RandomAlbums)
                        .await
            {
                return Ok(hit.value);
            }
            commands::load_collection(&session, commands::CollectionKind::RandomAlbums).await
        }
        .await
        .map(|items| match items {
            commands::Collection::Albums(albums) => albums.into_iter().take(100).collect(),
            _ => unreachable!("Random albums returns albums"),
        })
        .map_err(|error| format!("{error:#}"));
        Ok(vec![
            NavAction::RandomAlbumsLoaded { generation, result }.into(),
        ])
    });
}

pub fn open(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    match &state.sources.random_albums {
        State::Ready(albums) => {
            let albums = albums.clone();
            let request_id = state.artist_nav_request_id;
            effects::spawn(state, tx, "collection", async move {
                Ok(vec![
                    NavAction::List {
                        request_id,
                        kind: commands::CollectionKind::RandomAlbums,
                        items: commands::Collection::Albums(albums),
                    }
                    .into(),
                ])
            });
        }
        State::Loading => {}
        State::Idle | State::RefreshPending | State::Failed => {
            let force = !matches!(state.sources.random_albums, State::Idle);
            state.sources.random_albums = State::Idle;
            load(state, tx, force);
        }
    }
}

pub fn loaded(
    state: &mut AppState,
    generation: u64,
    result: Result<Vec<Album>, String>,
) -> Vec<Action> {
    if generation != state.library_generation {
        return vec![];
    }
    let visible = state.sources.nav_collection == Some(commands::CollectionKind::RandomAlbums)
        && state.browse_category == BrowseCategory::Library;
    match result {
        Ok(albums) => {
            state.sources.random_albums = State::Ready(albums.clone());
            if visible {
                return vec![
                    NavAction::List {
                        request_id: state.artist_nav_request_id,
                        kind: commands::CollectionKind::RandomAlbums,
                        items: commands::Collection::Albums(albums),
                    }
                    .into(),
                ];
            }
        }
        Err(error) => {
            state.sources.random_albums = State::Failed;
            tracing::warn!("Preload random albums: {error}");
            if visible {
                state.artist_nav.loading = false;
                state.set_error(format!(
                    "Load random albums: {error}. Choose Refresh Library to retry."
                ));
            }
        }
    }
    vec![]
}
