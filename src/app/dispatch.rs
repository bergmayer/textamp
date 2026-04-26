//! Shared action dispatch.
//!
//! The TUI's `EventLoop::dispatch` and the GUI's Iced application both
//! route `Action` values through `dispatch_action` so there is a single
//! source of truth for how Actions mutate state and trigger side-effects.
//!
//! Handlers live in `crate::app::handlers::dispatch_*`; this file is just
//! the router.

use anyhow::Result;
use tokio::sync::mpsc;

use crate::app::Action;
use crate::app::AppState;
use crate::app::event::{AuthEvent, Event};
use crate::app::handlers;
use crate::app::handlers::helpers;
use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::plex::{PlexAuth, PlexClient};

/// Dispatch an `Action` and all of its follow-up actions.
///
/// Returns when every Action (and any it spawns synchronously) has been
/// routed. Long-running I/O is handled by each dispatch module spawning
/// its own tokio tasks that emit results back on `event_tx`.
pub async fn dispatch_action(
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
    config: &mut Config,
    event_tx: &mpsc::Sender<Event>,
) -> Result<()> {
    let mut pending: Vec<Action> = vec![action];

    while let Some(next) = pending.pop() {
        let follow_ups = match next {
            Action::System(a) => {
                handlers::dispatch_system::dispatch(event_tx, config, a, state, client).await?
            }
            Action::Navigation(a) => {
                handlers::dispatch_navigation::dispatch(event_tx, a, state, client).await?
            }
            Action::Data(a) => {
                handlers::dispatch_data::dispatch(event_tx, config, a, state, client).await?
            }
            Action::Miller(a) => {
                handlers::dispatch_miller::dispatch(event_tx, a, state, client, audio).await?
            }
            Action::Playback(a) => {
                handlers::dispatch_playback::dispatch(event_tx, a, state, client, audio).await?
            }
            Action::Queue(a) => {
                handlers::dispatch_queue::dispatch(event_tx, a, state, client, audio).await?
            }
            Action::Search(a) => {
                handlers::dispatch_search::dispatch(event_tx, a, state, client).await?
            }
            Action::Browse(a) => {
                handlers::dispatch_browse::dispatch(event_tx, a, state, client).await?
            }
            Action::Folders(a) => {
                handlers::dispatch_folders::dispatch(event_tx, a, state, client, audio).await?
            }
            Action::Radio(a) => {
                handlers::dispatch_radio::dispatch(event_tx, a, state, client, audio).await?
            }
            Action::Settings(a) => {
                handlers::dispatch_settings::dispatch(event_tx, config, a, state, client, audio).await?
            }
        };

        // Follow-up actions get processed after the current batch.
        // Reverse so the first follow-up pops next (preserves apparent order).
        for f in follow_ups.into_iter().rev() {
            pending.push(f);
        }
    }

    Ok(())
}

/// Translate a core event into zero or more `Action`s without dispatching.
///
/// This is the complement to `dispatch_action` used by both front-ends.
/// TUI + GUI call this on every incoming event, then feed the returned
/// actions back to `dispatch_action`.
pub fn handle_core_event(
    event: Event,
    state: &mut AppState,
    client: &mut PlexClient,
    event_tx: &mpsc::Sender<Event>,
) -> Vec<Action> {
    handlers::events::handle_app_event(event, state, client, event_tx)
}

/// Kick off the shared authentication flow.
///
/// Mirrors `EventLoop::start_auth_task` exactly, just extracted so both the
/// TUI event loop and the GUI Iced application drive identical auth logic:
///  - Fast path: stored token + server_url + username → immediate
///    `AuthSuccess` (with a background server-discovery + connection-test
///    task that reconciles stale URLs and emits ServersDiscovered /
///    ServerConnectionSucceeded).
///  - Slow path: verify the stored token with plex.tv, discover servers,
///    pick a working connection, emit `AuthSuccess`.
///  - Nothing stored / token invalid: emit `AuthShowLogin`.
pub fn spawn_auth_task(event_tx: mpsc::Sender<Event>) {
    tokio::spawn(async move {
        if let Some(stored) = PlexAuth::load_token() {
            tracing::info!(
                "Loaded stored auth: client_identifier={}, server_url={:?}",
                stored.client_identifier, stored.server_url
            );

            // Fast path
            if let (Some(ref server_url), Some(ref username)) =
                (&stored.server_url, &stored.username)
            {
                tracing::info!("Fast path: using stored credentials (instant startup)");
                let _ = event_tx
                    .send(AuthEvent::AuthSuccess {
                        token: stored.token.clone(),
                        username: username.clone(),
                        server_url: server_url.clone(),
                        servers: vec![],
                        client_identifier: stored.client_identifier.clone(),
                        has_plex_pass: stored.has_plex_pass,
                    }.into())
                    .await;

                // Background validation + recovery.
                let event_tx_bg = event_tx.clone();
                let stored_bg = stored.clone();
                tokio::spawn(async move {
                    let auth = PlexAuth::from_stored_auth(&stored_bg);
                    let token = &stored_bg.token;
                    let client_id = &stored_bg.client_identifier;

                    let servers = match auth.get_servers(token).await {
                        Ok(s) => {
                            let _ = event_tx_bg.send(AuthEvent::ServersDiscovered(s.clone()).into()).await;
                            s
                        }
                        Err(e) => {
                            tracing::warn!("Background server discovery failed: {}", e);
                            return;
                        }
                    };

                    let working_url = if let Some(ref stored_id) = stored_bg.server_identifier {
                        if let Some(server) = servers.iter().find(|s| &s.client_identifier == stored_id) {
                            helpers::find_working_connection(server, token, client_id).await
                        } else {
                            helpers::find_working_connection_from_servers(&servers, token, client_id).await
                        }
                    } else {
                        helpers::find_working_connection_from_servers(&servers, token, client_id).await
                    };

                    if let Some(url) = working_url {
                        let server_name = servers
                            .iter()
                            .find(|s| s.connections.iter().any(|c| c.uri == url))
                            .map(|s| s.name.clone())
                            .unwrap_or_else(|| "Server".to_string());
                        let _ = event_tx_bg.send(AuthEvent::ServerConnectionSucceeded { server_name, url }.into()).await;
                    } else {
                        let server_name = stored_bg.server_name.clone()
                            .or_else(|| servers.first().map(|s| s.name.clone()))
                            .unwrap_or_else(|| "Server".to_string());
                        let _ = event_tx_bg.send(AuthEvent::ServerConnectionFailed { server_name }.into()).await;
                    }
                });
                return;
            }

            // Slow path: verify the token and find a live connection.
            let auth = PlexAuth::from_stored_auth(&stored);
            if let Ok(user) = auth.verify_token(&stored.token).await {
                let servers = auth.get_servers(&stored.token).await.unwrap_or_default();

                let final_server_url = if let Some(stored_id) = &stored.server_identifier {
                    if let Some(server) = servers.iter().find(|s| &s.client_identifier == stored_id) {
                        helpers::find_working_connection(server, &stored.token, &stored.client_identifier).await
                    } else {
                        helpers::find_working_connection_from_servers(&servers, &stored.token, &stored.client_identifier).await
                    }
                } else {
                    helpers::find_working_connection_from_servers(&servers, &stored.token, &stored.client_identifier).await
                };

                if let Some(url) = final_server_url {
                    let has_plex_pass = user.has_plex_pass();
                    let _ = event_tx
                        .send(AuthEvent::AuthSuccess {
                            token: stored.token,
                            username: user.username,
                            server_url: url,
                            servers,
                            client_identifier: stored.client_identifier,
                            has_plex_pass,
                        }.into())
                        .await;
                    return;
                }
            }
        }

        // Nothing valid — ask the UI to show the login form.
        let _ = event_tx.send(AuthEvent::AuthShowLogin.into()).await;
    });
}
