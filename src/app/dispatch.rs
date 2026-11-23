//! Shared action dispatch.
//!
//! The TUI routes actions through `dispatch_action`: one implementation
//! owns action ordering and follow-up dispatch.
//!
//! Handlers live in `crate::app::handlers::dispatch_*`; this file is just
//! the router. The source boundary consumes provider requests before shared dispatch;
//! `sources::routing` rejects an unhandled request instead of falling through
//! to a server effect. Shared reducers and explicit account management
//! remain independent of the active provider.

use anyhow::Result;
use tokio::sync::mpsc;

use crate::app::Action;
use crate::app::AppState;
use crate::app::action::SystemAction;
use crate::app::event::Event;
use crate::app::handlers;
use crate::audio::AudioPlayer;
use crate::config::Config;

/// Dispatch an `Action` and all of its follow-up actions.
///
/// Returns when every Action (and any it spawns synchronously) has been
/// routed. Long-running I/O is handled by each dispatch module spawning
/// its own tokio tasks that emit results back on `event_tx`.
pub async fn dispatch_action(
    action: Action,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    config: &mut Config,
    event_tx: &mpsc::Sender<Event>,
) -> Result<()> {
    let previous = super::queue_edit::pending(state);
    let result = dispatch_pending_actions(action, state, audio, config, event_tx).await;
    super::queue_edit::cancel_changed(previous, state);
    super::lyrics::reconcile(state, event_tx);
    result
}

async fn dispatch_pending_actions(
    action: Action,
    state: &mut AppState,
    audio: &mut AudioPlayer,
    config: &mut Config,
    event_tx: &mpsc::Sender<Event>,
) -> Result<()> {
    let mut pending: Vec<Action> = vec![action];

    while let Some(next) = pending.pop() {
        if match &next {
            Action::System(SystemAction::LoadWaveform) => {
                !state.visualizers_available && !state.playback_waveform_available
            }
            Action::System(SystemAction::LoadSpectrogram) => !state.visualizers_available,
            _ => false,
        } {
            continue;
        }
        if let Some(actions) = crate::app::downloads::route(&next, state) {
            pending.extend(actions.into_iter().rev());
            continue;
        }
        if crate::app::downloads::suppress_offline(&next, state) {
            continue;
        }
        if crate::app::sources::sonic::action_blocked(state, &next) {
            continue;
        }
        let queue_edit = matches!(next, Action::Queue(_));
        let provider_actions = crate::app::sources::route(&next, state, audio, event_tx);
        let routed = if let Some(actions) = provider_actions {
            Ok(actions)
        } else {
            match next {
                Action::PlayerSync(command) => {
                    super::player_sync::dispatch(command, state, config, event_tx)
                }
                Action::Source(a) => {
                    crate::app::sources::dispatch(a, state, audio, config, event_tx).await
                }
                Action::System(a) => {
                    handlers::dispatch_system::dispatch(event_tx, a, state, audio.analysis_input())
                        .await
                }
                Action::Navigation(a) => {
                    handlers::dispatch_navigation::dispatch(event_tx, a, state)
                }
                Action::Data(a) => handlers::dispatch_data::dispatch(event_tx, config, a, state),
                Action::Miller(a) => handlers::dispatch_miller::dispatch(event_tx, a, state, audio),
                Action::Playback(a) => {
                    handlers::dispatch_playback::dispatch(event_tx, a, state, audio)
                }
                Action::Queue(a) => handlers::dispatch_queue::dispatch(event_tx, a, state, audio),
                Action::Search(a) => handlers::dispatch_search::dispatch(event_tx, a, state),
                Action::Browse(a) => handlers::dispatch_browse::dispatch(event_tx, a, state),
                Action::Folders(_) => Ok(vec![
                    SystemAction::ShowError("Unsupported folder operation".into()).into(),
                ]),
                Action::Radio(a) => handlers::dispatch_radio::dispatch(event_tx, a, state, audio),
                Action::Settings(a) => {
                    handlers::dispatch_settings::dispatch(event_tx, config, a, state, audio)
                }
            }
        };

        let mut follow_ups = routed?;
        if queue_edit {
            super::queue_modes::prefetch(state, event_tx);
        }
        follow_ups.extend(restore_column_artwork(state, config));
        follow_ups.extend(crate::app::library_details::finish(state));

        // Depth-first, in returned order, matching recursive dispatch without
        // allocating a boxed future for every follow-up.
        for f in follow_ups.into_iter().rev() {
            pending.push(f);
        }
    }

    crate::app::sources::sonic::reconcile(state);
    super::sources::navidrome::album_notes::reconcile(state, event_tx);
    crate::app::sources::dialogs::restore_parent(state);
    Ok(())
}

fn restore_column_artwork(state: &mut AppState, config: &Config) -> Vec<Action> {
    let Some(library_key) = state.active_library.as_deref() else {
        return vec![];
    };
    let views = config
        .ui
        .library_view_settings
        .get(library_key)
        .map(|settings| &settings.columns);
    let category = state.browse_category;
    let mut enabled_columns = Vec::new();
    {
        let Some(nav) = state.browse_nav_mut() else {
            return vec![];
        };
        for index in 0..nav.columns.len() {
            let Some(key) = crate::app::state::column_artwork_key(category, &nav.columns, index)
            else {
                continue;
            };
            let visible = views
                .and_then(|views| views.get(&key))
                .copied()
                .unwrap_or(true);
            let Some(column) = nav.columns.get_mut(index) else {
                continue;
            };
            if column.artwork_visible != visible {
                column.artwork_visible = visible;
                if visible {
                    enabled_columns.push(index);
                }
            }
        }
    }
    let mut batch = Vec::new();
    if let Some(nav) = state.browse_nav() {
        for index in enabled_columns {
            batch.extend(crate::app::handlers::dispatch_miller::collect_art_to_load(
                nav.columns.get(index),
                &state.artwork.grid_cache,
                &state.artwork.grid_pending,
            ));
        }
    }
    if batch.is_empty() {
        vec![]
    } else {
        vec![SystemAction::LoadAlbumArt(batch).into()]
    }
}

/// Translate a core event into zero or more `Action`s without dispatching.
///
/// This is the complement to `dispatch_action`. The TUI handles terminal
/// input separately and passes asynchronous results through this reducer.
pub fn handle_core_event(
    event: Event,
    state: &mut AppState,

    event_tx: &mpsc::Sender<Event>,
) -> Vec<Action> {
    let previous = super::queue_edit::pending(state);
    let actions = match event {
        Event::RadioResult {
            generation,
            navigation_generation,
            event,
        } => {
            if navigation_generation.map_or(generation == state.radio_generation, |id| {
                id == state.station_navigation_generation
            }) {
                handle_core_event(*event, state, event_tx)
            } else {
                vec![]
            }
        }

        Event::LibraryResult { generation, event } => {
            if generation == state.library_generation {
                handle_core_event(*event, state, event_tx)
            } else {
                vec![]
            }
        }
        event => handlers::events::handle_app_event(event, state, event_tx),
    };
    super::queue_edit::cancel_changed(previous, state);
    crate::app::sources::sonic::reconcile(state);
    super::sources::navidrome::album_notes::reconcile(state, event_tx);
    actions
}

#[cfg(test)]
mod artwork_visibility_tests {
    use super::*;
    use crate::app::state::{BrowseColumn, BrowseItem};
    #[test]
    fn artwork_visibility_follows_column_type_instead_of_parent_or_frontend() {
        let album = |key: &str| {
            BrowseColumn::new(
                key,
                vec![BrowseItem::Album {
                    key: key.into(),
                    title: key.into(),
                    year: None,
                    thumb: None,
                    artist: String::new(),
                    is_placeholder: false,
                }],
            )
        };
        let mut state = AppState::new();
        state.active_library = Some("library".into());
        state.artist_nav.columns = vec![
            album("first artist"),
            album("second artist"),
            BrowseColumn::new_with_tracks(
                "Tracks",
                BrowseItem::from_tracks(&[crate::library::models::Track {
                    rating_key: "track".into(),
                    ..Default::default()
                }]),
                vec![],
            ),
        ];
        let mut config = Config::default();
        restore_column_artwork(&mut state, &config);
        assert!(
            state
                .artist_nav
                .columns
                .iter()
                .all(|column| column.artwork_visible)
        );
        let key = state.column_artwork_key(0).unwrap();
        config.set_column_artwork("library", &key, false);
        restore_column_artwork(&mut state, &config);
        assert!(!state.artist_nav.columns[0].artwork_visible);
        assert!(!state.artist_nav.columns[1].artwork_visible);
        assert!(state.artist_nav.columns[2].artwork_visible);
        state.artist_nav.columns[0] = album("another artist");
        state.desktop_frontend = true;
        restore_column_artwork(&mut state, &config);
        assert!(!state.artist_nav.columns[0].artwork_visible);
        state.active_library = Some("other library".into());
        restore_column_artwork(&mut state, &config);
        assert!(
            state
                .artist_nav
                .columns
                .iter()
                .all(|column| column.artwork_visible)
        );
    }
}
