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

use crate::app::action::SystemAction;
use crate::app::event::Event;
use crate::app::handlers;
use crate::app::Action;
use crate::app::AppState;
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
    let mut pending: Vec<Action> = vec![action];

    while let Some(next) = pending.pop() {
        if crate::app::sources::sonic::action_blocked(state, &next) {
            continue;
        }
        let provider_actions = crate::app::sources::route(&next, state, audio, event_tx);
        if let Some(actions) = provider_actions {
            pending.extend(actions.into_iter().rev());
            continue;
        }
        let follow_ups = match next {
            Action::Source(a) => {
                crate::app::sources::dispatch(a, state, audio, config, event_tx).await?
            }
            Action::System(a) => {
                handlers::dispatch_system::dispatch(event_tx, config, a, state).await?
            }
            Action::Navigation(a) => {
                handlers::dispatch_navigation::dispatch(event_tx, a, state).await?
            }
            Action::Data(a) => {
                handlers::dispatch_data::dispatch(event_tx, config, a, state).await?
            }
            Action::Miller(a) => {
                handlers::dispatch_miller::dispatch(event_tx, a, state, audio).await?
            }
            Action::Playback(a) => {
                handlers::dispatch_playback::dispatch(event_tx, a, state, audio).await?
            }
            Action::Queue(a) => {
                handlers::dispatch_queue::dispatch(event_tx, a, state, audio).await?
            }
            Action::Search(a) => handlers::dispatch_search::dispatch(event_tx, a, state).await?,
            Action::Browse(a) => handlers::dispatch_browse::dispatch(event_tx, a, state).await?,
            Action::Folders(_) => {
                vec![SystemAction::ShowError("Unsupported folder operation".into()).into()]
            }
            Action::Radio(a) => {
                handlers::dispatch_radio::dispatch(event_tx, a, state, audio).await?
            }
            Action::Settings(a) => {
                handlers::dispatch_settings::dispatch(event_tx, config, a, state, audio).await?
            }
        };

        // Depth-first, in returned order, matching recursive dispatch without
        // allocating a boxed future for every follow-up.
        for f in follow_ups.into_iter().rev() {
            pending.push(f);
        }
    }

    crate::app::sources::sonic::reconcile(state);
    Ok(())
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
        Event::ConnectionResult { generation, event } => {
            if generation == state.connection_generation {
                handle_core_event(*event, state, event_tx)
            } else {
                vec![]
            }
        }
        event => handlers::events::handle_app_event(event, state, event_tx),
    };
    crate::app::sources::sonic::reconcile(state);
    actions
}
