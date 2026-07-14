//! Settings dispatch handlers: Logout, AuthSignIn, AuthSelectServer, OpenSettings,
//! SaveCredentials, SettingsSelect, SettingsSignIn, SettingsDiscoverServers, SelectServer,
//! SelectLibrary, SaveSettings, ClearCache, and Adventure actions.

use crate::app::event::*;
use crate::app::{Action, AppState, Event};
use crate::app::action::{AsyncError, SettingsAction};
use crate::app::state::{ConnectionState, PlayStatus, PlaybackMode, QueueSortMode, SettingsSection, View};
use crate::plex::{PlexAuth, PlexClient};
use crate::audio::AudioPlayer;
use crate::plex::LibraryCache;
use crate::config::Config;
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;
use zeroize::{Zeroize, Zeroizing};

use super::helpers;

static CONFIG_SAVE_REVISION: AtomicU64 = AtomicU64::new(1);
static LAST_CONFIG_SAVE_REVISION: AtomicU64 = AtomicU64::new(0);
static CONFIG_SAVE_LOCK: Mutex<()> = Mutex::new(());

/// Serialize atomic config-file replacements on a blocking worker. A monotonic
/// revision prevents an older worker that starts late from overwriting a newer
/// in-memory settings snapshot.
fn save_config_in_background(
    event_tx: &mpsc::Sender<Event>,
    config: &Config,
    operation: &'static str,
) {
    let revision = CONFIG_SAVE_REVISION.fetch_add(1, Ordering::Relaxed);
    let snapshot = config.clone();
    let event_tx = event_tx.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = CONFIG_SAVE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if revision < LAST_CONFIG_SAVE_REVISION.load(Ordering::Acquire) {
            return;
        }
        // Claim the revision before touching disk. If this write fails, an
        // older snapshot must still not be allowed to "recover" by replacing
        // the newer in-memory configuration.
        LAST_CONFIG_SAVE_REVISION.store(revision, Ordering::Release);
        match crate::config::save_config(&snapshot) {
            Ok(()) => {}
            Err(error) => {
                let _ = event_tx.blocking_send(Event::Effect(
                    SettingsAction::PersistenceFailed {
                        operation: operation.to_string(),
                        error: error.to_string(),
                    }
                    .into(),
                ));
            }
        }
    });
}

fn drop_in_background<T: Send + 'static>(value: T) {
    tokio::task::spawn_blocking(move || drop(value));
}

fn reset_library_data(state: &mut AppState) {
    let library_sub_mode = state.library.library_sub_mode;
    let detection_request_id = state.library.compilations.detection_request_id;
    let previous = std::mem::replace(
        &mut state.library,
        crate::app::state::LibraryData::default(),
    );
    state.library.library_sub_mode = library_sub_mode;
    state.library.compilations.detection_request_id = detection_request_id;
    drop_in_background(previous);
}

fn clear_folder_cache(state: &mut AppState) {
    let previous = std::mem::take(&mut state.folder_contents_cache);
    drop_in_background(previous);
}

fn clear_playlist_track_cache(state: &mut AppState) {
    let previous = std::mem::take(&mut state.playlist_tracks_cache);
    drop_in_background(previous);
}

fn apply_library_cache_clear(
    count: usize,
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    client: &PlexClient,
) {
    // Reject every request launched from the pre-clear snapshot before
    // starting its replacement preloads.
    state.advance_library_generation();

    reset_library_data(state);
    let old_playlist_tracks = std::mem::take(&mut state.playlist_tracks_cache);
    drop_in_background(old_playlist_tracks);
    state.artist_nav = crate::app::state::BrowseNavigationState::new();
    state.tag_nav = crate::app::state::BrowseNavigationState::new();
    state.playlist_nav = crate::app::state::BrowseNavigationState::new();
    state.station_nav = crate::app::state::StationNavigationState::new();
    state.stations.clear();
    state.station_children_cache.clear();
    state.folder_state = None;
    state.list_filter.deactivate();
    state.list_state.reset();
    state.cache_mgmt.category_timestamps.clear();
    state.cache_mgmt.background_refresh.clear();
    state.cache_mgmt.preloads_in_progress.clear();
    state.cache_mgmt.dirty = true;
    state.library_loading = true;

    if let Some(lib_key) = state.active_library.clone() {
        let lib_name = state
            .libraries
            .iter()
            .find(|library| library.key == lib_key)
            .map(|library| library.title.clone())
            .unwrap_or_else(|| lib_key.clone());
        helpers::preload_all_library_data(event_tx, &lib_key, &lib_name, client, state);
    }

    state.library_cache_stats = Some((0, vec![]));
    state.set_status(format!(
        "Cleared {} library cache files, reloading...",
        count
    ));
}

fn apply_artwork_cache_clear(count: usize, state: &mut AppState) {
    state.artwork.clear_grid_art();
    state.artwork.grid_pending.clear();
    state.artwork.current_data = None;
    state.artwork.pending_thumb = None;
    state.artwork.loading = false;
    state.artwork.cache_stats = Some((0, 0));
    state.set_status(format!("Cleared {} artwork cache files", count));
}

/// Dispatch settings/auth/adventure actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: SettingsAction,
    state: &mut AppState,
    client: &mut PlexClient,
    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        SettingsAction::Logout => {
            let logout_storage_epoch = PlexAuth::begin_account_storage_epoch();
            state.advance_library_generation();
            reset_library_data(state);
            let marker_username = match &state.connection {
                ConnectionState::Connected { username, .. }
                | ConnectionState::Degraded { username, .. } => Some(username.clone()),
                _ => None,
            };
            // Record which account the on-disk cache belongs to so a
            // future sign-in with the same account (within 30 days)
            // can skip a full re-fetch. Written BEFORE we delete the
            // auth token. Disk work is deferred until after local playback and
            // account state have been invalidated.

            // Cache files on disk are intentionally preserved here.
            // The next sign-in compares the account marker and either
            // keeps the cache (same user + < 30 days old) or clears
            // it then. Wiping eagerly here would force a multi-minute
            // re-fetch on every sign-in / sign-out cycle.

            // Reset connection and display state
            state.connection = ConnectionState::Disconnected;
            client.clear_session();
            state.active_library = None;
            state.libraries.clear();
            state.available_servers.clear();
            state.connected_server_url = None;
            state.active_server_id = None;
            state.artwork.cache_stats = None;
            state.library_cache_stats = None;
            state.waveform_cache_stats = None;

            // Clear browse data
            state.library.artists.clear();
            state.library.albums.clear();
            state.library.playlists.clear();
            state.library.album_genres.clear();
            state.library.artist_genres.clear();
            state.library.album_genres.clear();
            state.library.moods.clear();
            state.library.styles.clear();
            state.stations.clear();
            state.library.all_tracks.clear();
            state.library.track_artists.clear();
            state.library.artist_aliases.clear();
            state.library.album_display_artist.clear();
            state.library.compilations.albums.clear();
            state.library.compilations.artist_keys.clear();
            state.library.compilations.track_artist_keys.clear();
            state.library.compilations.artist_map.clear();
            state.library.compilations.single_artist.clear();
            state.library.compilations.detected = false;
            state.library.compilations.detecting = false;
            state.library.compilations.detection_request_id = state.library.compilations.detection_request_id.wrapping_add(1);

            state.library.selected_artist_albums.clear();
            state.library.selected_album_tracks.clear();
            state.library.tag_albums.clear();
            state.folder_state = None;
            clear_folder_cache(state);
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;

            // Clear playback state
            state.queue.tracks.clear();
            state.queue.index = None;
            state.queue.original.clear();

            // Clear navigation state
            state.station_nav.columns.clear();
            state.station_nav.focused_column = 0;
            state.list_state.reset();

            // Clear session/runtime state
            state.cache_mgmt.category_timestamps.clear();
            state.cache_mgmt.background_refresh.clear();
            state.cache_mgmt.preloads_in_progress.clear();
            state.cache_mgmt.preloads_total = 0;
            state.plex_session_id = None;
            state.artwork.clear_grid_art();
            state.artwork.grid_pending.clear();
            state.waveform = Default::default();
            state.search.results = None;
            clear_playlist_track_cache(state);

            // Stop playback and flush track cache
            audio.stop();
            state.playback.request_id = audio.playback_id();
            audio.track_cache.flush();
            state.playback.status = PlayStatus::Stopped;

            // Clear all server-related config (keep app settings like theme/playback)
            config.plex = crate::config::PlexConfig::default();
            config.libraries.default_library = None;
            config.libraries.selected_server = None;
            config.general.default_library = None;
            save_config_in_background(event_tx, config, "save configuration after logout");

            // Send the user straight to the sign-in form. Without
            // this they're stranded on whatever view they triggered
            // logout from (typically a Settings popup that's now
            // showing "Not signed in") with no obvious way back.
            state.view = View::Auth;
            // Keep the login form inert until token deletion has completed;
            // otherwise a very fast new sign-in can save a token just before
            // the logout worker deletes it.
            state.auth_state.step = crate::app::state::AuthStep::Authenticating;
            state.auth_state.error_message = None;

            state.set_status("Signing out...".to_string());
            let event_tx = event_tx.clone();
            tokio::task::spawn_blocking(move || {
                let result = PlexAuth::with_account_storage_epoch(
                    logout_storage_epoch,
                    || {
                        let mut errors = Vec::new();
                        if let Some(username) = marker_username {
                            if let Err(error) = PlexAuth::save_account_marker(&username) {
                                errors.push(format!("save account marker: {error}"));
                            }
                        }
                        if let Err(error) = PlexAuth::delete_token() {
                            errors.push(format!("delete auth token: {error}"));
                        }
                        if errors.is_empty() {
                            Ok(())
                        } else {
                            Err(errors.join("; "))
                        }
                    }
                ).unwrap_or(Ok(()));
                let _ = event_tx.blocking_send(Event::Effect(
                    SettingsAction::LogoutStorageFinished(result).into(),
                ));
            });
        }
        SettingsAction::LogoutStorageFinished(result) => {
            state.auth_state.step = crate::app::state::AuthStep::Login;
            match result {
                Ok(()) => state.set_status("Signed out.".to_string()),
                Err(error) => state.set_error(format!(
                    "Signed out locally, but credential cleanup failed: {error}"
                )),
            }
        }
        SettingsAction::PersistenceFailed { operation, error } => {
            state.set_error(format!("Failed to {operation}: {error}"));
        }
        SettingsAction::AuthSignIn => {
            use crate::app::state::AuthStep;
            // Authenticate with username/password entered in auth screen login form
            let username = state.auth_state.username_input.clone();

            if username.is_empty() || state.auth_state.password_input.is_empty() {
                state.auth_state.error_message = Some("Please enter username and password".to_string());
            } else {
                // Move rather than clone the secret. The authentication task is
                // its sole owner and zeroizes it immediately after the request.
                let password = Zeroizing::new(state.auth_state.password_input.take());
                state.auth_state.step = AuthStep::Authenticating;
                state.auth_state.error_message = None;
                let event_tx = event_tx.clone();

                tokio::spawn(async move {
                    let auth = match PlexAuth::new() {
                        Ok(auth) => auth,
                        Err(error) => {
                            let _ = event_tx.send(AuthEvent::AuthLoginFailed(
                                format!("Cannot initialize HTTP client: {}", error)
                            ).into()).await;
                            return;
                        }
                    };

                    let authentication = auth.authenticate_password(&username, &password).await;
                    match authentication {
                        Ok(token) => {
                            // Verify token and get user info
                            match auth.verify_token(&token).await {
                                Ok(user) => {
                                    // Get client_identifier BEFORE saving (save_token consumes it)
                                    let client_id = auth.client_identifier().to_string();

                                    // Get servers
                                    let servers = auth.get_servers(&token).await.unwrap_or_default();

                                    // Credential serialization and atomic file replacement are
                                    // blocking. Complete them off the runtime before exposing the
                                    // authenticated session to the reducer.
                                    let save_token = token.clone();
                                    let save_user = user.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        auth.save_token(&save_token, Some(&save_user))
                                    }).await {
                                        Ok(Ok(())) => {}
                                        Ok(Err(error)) => tracing::warn!("Failed to save token: {}", error),
                                        Err(error) => tracing::warn!("Token-save worker failed: {}", error),
                                    }

                                    // Send servers ready event (will auto-select or show selection)
                                    let has_plex_pass = user.has_plex_pass();
                                    let _ = event_tx.send(AuthEvent::AuthServersReady {
                                        token: token.into(),
                                        username: user.username,
                                        servers,
                                        client_identifier: client_id,
                                        has_plex_pass,
                                    }.into()).await;
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AuthEvent::AuthLoginFailed(
                                        format!("Token verification failed: {}", e)
                                    ).into()).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(AuthEvent::AuthLoginFailed(
                                format!("Invalid username or password")
                            ).into()).await;
                            tracing::error!("Auth error: {}", e);
                        }
                    }
                });

                // Clear password from memory immediately
                state.auth_state.password_input.zeroize();
            }
        }
        SettingsAction::AuthSelectServer => {
            use crate::app::state::AuthStep;
            // Select server from the server selection list
            if let Some(server) = state.available_servers.get(state.auth_state.server_index) {
                // Get the token and client_identifier that were stored when servers were received
                let token = client.token().map(|s| s.to_string());
                let client_id = client.client_identifier().to_string();

                if let Some(token) = token {
                    state.auth_state.step = AuthStep::Connecting;
                    let username = state.settings_state.username_input.clone();
                    let servers = state.available_servers.clone();
                    let server_clone = server.clone();
                    let event_tx = event_tx.clone();

                    // Find working connection URL (tests connectivity)
                    let has_plex_pass = state.auth_state.has_plex_pass;
                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server_clone, &token, &client_id).await {
                            let server_identifier = Some(server_clone.client_identifier.clone());
                            let _ = event_tx.send(AuthEvent::AuthSuccess {
                                token: token.into(),
                                username,
                                server_url: url,
                                server_identifier,
                                servers,
                                client_identifier: client_id,
                                has_plex_pass,
                            }.into()).await;
                        } else {
                            let _ = event_tx.send(AuthEvent::AuthFailed(
                                format!("Could not connect to {} - all connection attempts failed", server_clone.name)
                            ).into()).await;
                        }
                    });
                } else {
                    state.auth_state.error_message = Some("Authentication token not found".to_string());
                    state.auth_state.step = AuthStep::Login;
                }
            }
        }
        SettingsAction::OpenSettings => {
            state.set_view(View::Settings);
            state.settings_state.section = SettingsSection::Account;
            state.settings_state.item_index = 0;
            state.settings_state.signing_in = false;

            // Auto-discover remote players if connected and list is empty
            if state.remote.players.is_empty() && !state.remote.discovering {
                follow_ups.push(SettingsAction::DiscoverPlayers.into());
            }

            // Refresh cache stats so the cache section shows live
            // numbers — same as the GUI's settings popup. Without
            // this the TUI Cache table falls back to whatever was
            // computed at AuthSuccess (often empty if the on-disk
            // cache file hadn't been written yet).
            follow_ups.push(SettingsAction::RefreshCacheStats.into());

            // Get username from live state first, then config. Opening a view
            // must not synchronously read the credential file.
            state.settings_state.username_input = match &state.connection {
                ConnectionState::Connected { username, .. }
                | ConnectionState::Degraded { username, .. } => username.clone(),
                _ => config.plex.username.clone()
                    .unwrap_or_default(),
            };

            // Password field no longer used - token-based auth only
            state.settings_state.password_input.zeroize();
            state.settings_state.editing_credential = None;

            // If servers list is empty but we're connected, trigger discovery
            if state.available_servers.is_empty() {
                if let Some(token) = client.token().map(str::to_owned) {
                    let stored = crate::plex::StoredAuth {
                        token: token.into(),
                        user_id: None,
                        username: Some(state.settings_state.username_input.clone()),
                        client_identifier: client.client_identifier().to_string(),
                        server_url: client.server_url().map(str::to_owned),
                        server_identifier: state.active_server_id.clone(),
                        server_name: state.active_server_name().map(str::to_owned),
                        has_plex_pass: matches!(
                            &state.connection,
                            ConnectionState::Connected { has_plex_pass: true, .. }
                                | ConnectionState::Degraded { has_plex_pass: true, .. }
                        ),
                    };
                    state.settings_state.discovering_servers = true;
                    let event_tx = LibraryEventSender::new(
                        event_tx.clone(),
                        state.library_generation,
                    );
                    tokio::spawn(async move {
                        let auth = match PlexAuth::from_stored_auth(&stored) {
                            Ok(auth) => auth,
                            Err(error) => {
                                let _ = event_tx.send(
                                    AuthEvent::ServerDiscoveryFailed(error.to_string()).into()
                                ).await;
                                return;
                            }
                        };
                        match auth.get_servers(&stored.token).await {
                            Ok(servers) => {
                                let _ = event_tx.send(AuthEvent::ServersDiscovered(servers).into()).await;
                            }
                            Err(e) => {
                                let _ = event_tx.send(AuthEvent::ServerDiscoveryFailed(e.to_string()).into()).await;
                            }
                        }
                    });
                }
            }
        }
        SettingsAction::SaveCredentials => {
            // Save username to config file (for display purposes only)
            // Authentication is handled via stored tokens, not passwords
            config.plex.username = if state.settings_state.username_input.is_empty() {
                None
            } else {
                Some(state.settings_state.username_input.clone())
            };
            save_config_in_background(event_tx, config, "save username");
            state.set_status("Username saved.".to_string());
        }
        SettingsAction::SettingsSelect => {
            match state.settings_state.section {
                SettingsSection::Account => {
                    if state.settings_state.signing_in {
                        // In sign-in mode: 0=username, 1=password, 2=sign in, 3+=servers
                        let server_index = state.settings_state.item_index.saturating_sub(3);
                        if let Some(server) = state.available_servers.get(server_index) {
                            let server_id = server.client_identifier.clone();
                            tracing::info!("Selected server: {}", server.name);
                            follow_ups.push(SettingsAction::SelectServer(server_id).into());
                        }
                    } else if state.connection.is_authenticated() {
                        use crate::app::state::{ConfirmDialog, ConfirmAction};
                        let lib_count = state.libraries.len();
                        let idx = state.settings_state.item_index;
                        if idx < lib_count {
                            // Activate selected library
                            if let Some(lib) = state.libraries.get(idx) {
                                let lib_key = lib.key.clone();
                                follow_ups.push(SettingsAction::SelectLibrary(lib_key).into());
                            }
                        } else {
                            match idx - lib_count {
                                0 => {
                                    state.popups.close_all();
                                    state.popups.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Library Cache".to_string(),
                                        message: "Clear all cached library data and reload from server?".to_string(),
                                        on_confirm: ConfirmAction::ClearLibraryCache,
                                        selected_yes: true,
                                    });
                                }
                                1 => {
                                    state.popups.close_all();
                                    state.popups.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Artwork Cache".to_string(),
                                        message: "Delete all cached album artwork from disk?".to_string(),
                                        on_confirm: ConfirmAction::ClearArtworkCache,
                                        selected_yes: true,
                                    });
                                }
                                2 => {
                                    state.popups.close_all();
                                    state.popups.confirm_dialog = Some(ConfirmDialog {
                                        title: "Clear Subfolder Cache".to_string(),
                                        message: "Clear all cached subfolder contents?".to_string(),
                                        on_confirm: ConfirmAction::ClearSubfolderCache,
                                        selected_yes: true,
                                    });
                                }
                                3 => {
                                    // Toggle crawl: start if not running, stop if running
                                    if state.subfolder_preload_active {
                                        follow_ups.push(SettingsAction::StopSubfolderCrawl.into());
                                    } else {
                                        follow_ups.push(SettingsAction::StartSubfolderCrawl.into());
                                    }
                                }
                                4 => follow_ups.push(SettingsAction::ToggleKeepSubfolderCache.into()),
                                5 => follow_ups.push(SettingsAction::Logout.into()),
                                _ => {}
                            }
                        }
                    } else {
                        // Not signed in: 0=Sign In
                        if state.settings_state.item_index == 0 {
                            state.settings_state.signing_in = true;
                            state.settings_state.item_index = 0;
                        }
                    }
                }
                SettingsSection::Textamp => {
                    let theme_count = crate::app::theme::ThemeName::all().len();
                    let artwork_count = crate::app::state::ArtworkMode::all().len();
                    let output_offset = theme_count + artwork_count;
                    let idx = state.settings_state.item_index;

                    if idx < theme_count {
                        // Apply selected theme
                        if let Some(theme_name) = crate::app::theme::ThemeName::all().get(idx) {
                            state.theme = *theme_name;
                            crate::ui::theme::set_theme(state.theme);
                            state.set_status(format!("Theme: {}", state.theme.display_name()));

                            config.ui.theme = state.theme.config_name().to_string();
                            save_config_in_background(event_tx, config, "save theme preference");
                        }
                    } else if idx >= theme_count && idx < output_offset {
                        // Select artwork mode
                        let mode_idx = idx - theme_count;
                        if let Some(&mode) = crate::app::state::ArtworkMode::all().get(mode_idx) {
                            state.artwork.mode = mode;
                            crate::ui::screens::now_playing::set_artwork_mode(mode);
                            crate::ui::artwork::set_grid_artwork_mode(mode);
                            crate::ui::set_bio_artwork_mode(mode);

                            match mode {
                                crate::app::state::ArtworkMode::Halfblocks => {
                                    let hb = ratatui_image::picker::ProtocolType::Halfblocks;
                                    crate::ui::screens::now_playing::set_artwork_protocol_type(hb);
                                    crate::ui::artwork::set_grid_protocol_type(hb);
                                    crate::ui::set_bio_artwork_protocol_type(hb);
                                }
                                crate::app::state::ArtworkMode::Auto => {
                                    crate::ui::screens::now_playing::restore_artwork_native_protocol();
                                    crate::ui::artwork::restore_grid_native_protocol();
                                    crate::ui::restore_bio_artwork_native_protocol();
                                }
                                crate::app::state::ArtworkMode::Braille => {
                                    // Braille doesn't use picker protocol
                                }
                            }

                            state.set_status(format!("Artwork: {}", mode.name()));

                            config.ui.artwork_mode = mode.name().to_string();
                            save_config_in_background(event_tx, config, "save artwork preference");
                        }
                    } else if idx == output_offset {
                        // Local output
                        follow_ups.push(SettingsAction::SetOutputTarget(crate::app::state::OutputTarget::Local).into());
                    } else if idx <= output_offset + state.remote.players.len() {
                        // Remote player
                        let player_idx = idx - output_offset - 1;
                        if let Some(player) = state.remote.players.get(player_idx) {
                            let uri = player.connections.iter().find(|c| c.local)
                                .or_else(|| player.connections.iter().find(|c| !c.relay))
                                .or(player.connections.first())
                                .map(|c| c.uri.clone());
                            tracing::info!(
                                "Selecting remote player: {} (id={}, product={}, uri={:?})",
                                player.name, player.client_identifier, player.product, uri
                            );
                            follow_ups.push(SettingsAction::SetOutputTarget(crate::app::state::OutputTarget::Remote {
                                player_id: player.client_identifier.clone(),
                                player_name: player.name.clone(),
                                player_uri: uri,
                            }).into());
                        }
                    } else if idx == output_offset + 1 + state.remote.players.len() {
                        // Refresh players
                        follow_ups.push(SettingsAction::DiscoverPlayers.into());
                    } else if idx == output_offset + 2 + state.remote.players.len() {
                        // Transcode: cycle through 0 → 128 → 192 → 256 → 320 → 0
                        let options = [0u32, 128, 192, 256, 320];
                        let current_pos = options.iter().position(|&v| v == state.transcode_kbps).unwrap_or(0);
                        let next = options[(current_pos + 1) % options.len()];
                        state.transcode_kbps = next;

                        // Flush pre-fetch cache since encoding changed
                        audio.track_cache.flush();

                        // Save to config
                        config.playback.transcode_kbps = next;
                        save_config_in_background(event_tx, config, "save transcode preference");

                        if next == 0 {
                            state.set_status("Streaming: original (direct play)".to_string());
                        } else {
                            state.set_status(format!("Streaming: transcode to {}kbps MP3", next));
                        }
                    } else {
                        // External-services toggles. Indices, in order
                        // of the rows shown in `render_textamp_content`:
                        //   ext_base + 0: Apple Music
                        //   ext_base + 1: Spotify
                        //   ext_base + 2: YouTube
                        let ext_base = output_offset + 3 + state.remote.players.len();
                        use crate::services::external_search::SearchTarget;
                        match idx.checked_sub(ext_base) {
                            Some(0) => follow_ups.push(SettingsAction::ToggleExternalSearchService(SearchTarget::AppleMusic).into()),
                            Some(1) => follow_ups.push(SettingsAction::ToggleExternalSearchService(SearchTarget::Spotify).into()),
                            Some(2) => follow_ups.push(SettingsAction::ToggleExternalSearchService(SearchTarget::YouTube).into()),
                            _ => {}
                        }
                    }
                }
                SettingsSection::Sections => {
                    let idx = state.settings_state.item_index;
                    let cats = crate::app::state::BrowseCategory::all();
                    if idx < cats.len() {
                        let section = cats[idx];
                        follow_ups.push(SettingsAction::ToggleSectionVisibility(section).into());
                    }
                }
                SettingsSection::Cache => {}
                SettingsSection::About => {}
            }
        }
        SettingsAction::SettingsSignIn => {
            // Authenticate with username/password entered in settings
            let username = state.settings_state.username_input.clone();

            if username.is_empty() || state.settings_state.password_input.is_empty() {
                state.set_error("Please enter username and password".to_string());
            } else if state.settings_state.discovering_servers {
                // Already signing in
            } else {
                let password = Zeroizing::new(state.settings_state.password_input.take());
                state.settings_state.discovering_servers = true;
                let event_tx = event_tx.clone();
                let server_url = config.plex.server_url.clone();

                tokio::spawn(async move {
                    let auth = match PlexAuth::new() {
                        Ok(auth) => auth,
                        Err(error) => {
                            let _ = event_tx.send(AuthEvent::AuthLoginFailed(
                                format!("Cannot initialize HTTP client: {}", error)
                            ).into()).await;
                            return;
                        }
                    };

                    let authentication = auth.authenticate_password(&username, &password).await;
                    match authentication {
                        Ok(token) => {
                            // Verify token and get user info
                            match auth.verify_token(&token).await {
                                Ok(user) => {
                                    // Get client_identifier BEFORE saving
                                    let client_id = auth.client_identifier().to_string();

                                    // Get servers
                                    let servers = auth.get_servers(&token).await.unwrap_or_default();

                                    let save_token = token.clone();
                                    let save_user = user.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        auth.save_token(&save_token, Some(&save_user))
                                    }).await {
                                        Ok(Ok(())) => {}
                                        Ok(Err(error)) => tracing::warn!("Failed to save token: {}", error),
                                        Err(error) => tracing::warn!("Token-save worker failed: {}", error),
                                    }

                                    // Multiple servers and no configured URL: show server selection
                                    if server_url.is_empty() && servers.len() > 1 {
                                        let has_plex_pass = user.has_plex_pass();
                                        let _ = event_tx.send(AuthEvent::AuthServersReady {
                                            token: token.into(),
                                            username: user.username,
                                            servers,
                                            client_identifier: client_id,
                                            has_plex_pass,
                                        }.into()).await;
                                        return;
                                    }

                                    // Single server or configured URL: connect directly
                                    let final_url = if !server_url.is_empty()
                                        && crate::plex::test_connection(
                                            &server_url,
                                            &token,
                                            &client_id,
                                        ).await.is_ok()
                                    {
                                        Some(server_url)
                                    } else {
                                        helpers::find_working_connection_from_servers(
                                            &servers,
                                            &token,
                                            &client_id,
                                        ).await
                                    };

                                    if let Some(url) = final_url {
                                        let has_plex_pass = user.has_plex_pass();
                                        let server_identifier = servers
                                            .iter()
                                            .find(|server| {
                                                server.connections.iter().any(|connection| connection.uri == url)
                                            })
                                            .map(|server| server.client_identifier.clone());
                                        let _ = event_tx.send(AuthEvent::AuthSuccess {
                                            token: token.into(),
                                            username: user.username,
                                            server_url: url,
                                            server_identifier,
                                            servers,
                                            client_identifier: client_id,
                                            has_plex_pass,
                                        }.into()).await;
                                    } else {
                                        // Preserve the pending credential context and let the
                                        // normal server-selection reducer show the choices.
                                        let has_plex_pass = user.has_plex_pass();
                                        let _ = event_tx.send(AuthEvent::AuthServersReady {
                                            token: token.into(),
                                            username: user.username,
                                            servers,
                                            client_identifier: client_id,
                                            has_plex_pass,
                                        }.into()).await;
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AuthEvent::AuthFailed(
                                        format!("Token verification failed: {}", e)
                                    ).into()).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(AuthEvent::AuthFailed(
                                format!("Authentication failed: {}", e)
                            ).into()).await;
                        }
                    }
                });

                // Clear password immediately from memory (don't store it)
                state.settings_state.password_input.zeroize();
            }
        }
        SettingsAction::SelectServer(server_id) => {
            // Complete a settings-screen sign-in. This must flow through
            // AuthSuccess rather than merely replacing PlexClient::server_url;
            // the latter would retain the prior account's state and connection
            // identity under the newly installed token.
            if let Some(server) = state.available_servers.iter().find(|s| s.client_identifier == server_id) {
                let token = client.token().map(|s| s.to_string());

                if let Some(token) = token {
                    let server_clone = server.clone();
                    let event_tx = event_tx.clone();
                    let client_id = client.client_identifier().to_string();
                    let username = state.settings_state.username_input.clone();
                    let servers = state.available_servers.clone();
                    let has_plex_pass = state.auth_state.has_plex_pass;

                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server_clone, &token, &client_id).await {
                            let _ = event_tx.send(AuthEvent::AuthSuccess {
                                token: token.into(),
                                username,
                                server_url: url,
                                server_identifier: Some(server_clone.client_identifier),
                                servers,
                                client_identifier: client_id,
                                has_plex_pass,
                            }.into()).await;
                        } else {
                            let _ = event_tx.send(AuthEvent::AuthFailed(format!(
                                "Could not connect to {} - all connections failed",
                                server_clone.name,
                            )).into()).await;
                        }
                    });

                    state.set_status(format!("Testing connections to {}...", server.name));
                } else {
                    state.set_error("No authentication token available".to_string());
                }
            }
        }
        SettingsAction::SelectLibrary(lib_key) => {
            // Switch to the selected library
            if state.active_library.as_ref() != Some(&lib_key) {
                state.advance_library_generation();
                reset_library_data(state);
                state.active_library = Some(lib_key.clone());
                state.keep_subfolder_cache = config.libraries.per_library
                    .get(lib_key.as_str())
                    .map(|s| s.keep_subfolder_cache)
                    .unwrap_or(false);

                // Clear all current data and UI state
                state.library.artists.clear();
                state.library.albums.clear();
                state.library.playlists.clear();
                state.library.album_genres.clear();
                state.library.artist_genres.clear();
                state.library.album_genres.clear();
                state.library.moods.clear();
                state.library.styles.clear();
                state.library.decades.clear();
                state.library.years.clear();
                state.library.collections.clear();
                state.library.countries.clear();
                state.library.labels.clear();
                state.library.formats.clear();
                state.library.studios.clear();
                state.stations.clear();
                state.library.all_tracks.clear();
                state.library.track_artists.clear();
                state.library.artist_aliases.clear();
                state.library.album_display_artist.clear();
                state.library.compilations.albums.clear();
                state.library.compilations.artist_keys.clear();
                state.library.compilations.track_artist_keys.clear();
                state.library.compilations.artist_map.clear();
                state.library.compilations.single_artist.clear();
                state.library.compilations.detected = false;
                state.library.compilations.detecting = false;
                state.library.compilations.detection_request_id = state.library.compilations.detection_request_id.wrapping_add(1);

                state.library.selected_artist_albums.clear();
                state.library.selected_album_tracks.clear();
                state.library.tag_albums.clear();
                state.folder_state = None;
                clear_folder_cache(state);
                state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                state.subfolder_preload_active = false;
                clear_playlist_track_cache(state);
                state.search.results = None;
                state.similar = crate::app::state::SimilarViewState::default();
                state.related = crate::app::state::RelatedViewState::default();
                state.list_filter.deactivate();
                state.list_state.reset();

                // Clear cache timestamps (old library's values must not leak to new library)
                state.cache_mgmt.category_timestamps.clear();
                state.cache_mgmt.dirty = false;

                // Clear Miller column navigation states
                state.artist_nav = crate::app::state::BrowseNavigationState::new();
                state.tag_nav = crate::app::state::BrowseNavigationState::new();
                state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                state.station_nav = crate::app::state::StationNavigationState::new();

                // Report playback stop to Plex before switching libraries
                if state.playback.status != PlayStatus::Stopped {
                    if let Some(track) = state.current_track().cloned() {
                        helpers::report_playback_stop_to_plex(
                            &track, state.playback.position_ms, false,
                            state.plex_session_id.clone(), client,
                        );
                    }
                }

                // Stop remote playback if active
                if let crate::app::state::OutputTarget::Remote { ref player_id, ref player_uri, .. } = state.remote.output_target {
                    let target_id = player_id.clone();
                    let p_uri = player_uri.clone();
                    let token = client.shared_token_or_empty();
                    let client_id = client.client_identifier().to_string();
                    let server_url = client.server_url().unwrap_or("").to_string();
                    let machine_id = state.active_server_id.clone()
                        .or_else(|| state.available_servers.first()
                            .map(|server| server.client_identifier.clone()))
                        .unwrap_or_default();
                    tokio::spawn(async move {
                        if let Ok(rc) = crate::plex::RemotePlayerClient::new(
                            token, client_id, target_id, server_url, machine_id, p_uri,
                        ) {
                            let _ = rc.stop().await;
                        }
                    });
                }

                // Stop playback, flush track cache, and clear queue (tracks belong to the old library)
                audio.stop();
                state.playback.request_id = audio.playback_id();
                audio.track_cache.flush();
                state.playback.status = PlayStatus::Stopped;
                state.playback.position_ms = 0;
                state.playback.duration_ms = 0;
                state.playback.playback_started_at = None;
                state.queue.tracks.clear();
                state.queue.index = None;
                state.queue.original.clear();
                state.radio.clear();
                state.playback_mode = PlaybackMode::Queue;
                state.adventure = crate::app::state::AdventureState::default();

                // Clear waveform and artwork (belong to the old library's track)
                state.waveform = crate::app::state::WaveformState::default();
                state.artwork.current_thumb = None;
                state.artwork.current_data = None;
                state.artwork.loading = false;
                state.artwork.pending_thumb = None;

                // Find library name for status message
                let lib_name = state.libraries.iter()
                    .find(|l| l.key == lib_key)
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| lib_key.clone());

                // Show loading indicator and load cache in background
                state.library_loading = true;

                let tx = event_tx.clone();
                let bg_lib_key = lib_key.clone();
                let cache_generation = state.library_generation;
                let cache_server_id = state.active_server_id.clone();
                tokio::task::spawn_blocking(move || {
                    let result = LibraryCache::new().and_then(|cache| {
                        cache.load_scoped(cache_server_id.as_deref(), &bg_lib_key)
                    });
                    match result {
                        Some(cached) => {
                            let event = PreloadEvent::LibraryCacheLoaded {
                                library_key: bg_lib_key,
                                cached: Box::new(cached),
                            };
                            let _ = tx.blocking_send(Event::for_library(cache_generation, event));
                        }
                        None => {
                            let event = PreloadEvent::LibraryCacheLoadFailed {
                                library_key: bg_lib_key,
                            };
                            let _ = tx.blocking_send(Event::for_library(cache_generation, event));
                        }
                    }
                });

                // Refresh from API in background
                helpers::preload_all_library_data(event_tx, &lib_key, &lib_name, client, state);

                state.set_status(format!("Switched to {}", lib_name));

                // Auto-save the default library
                follow_ups.push(SettingsAction::SaveSettings.into());
            }
        }
        SettingsAction::SelectLibraryOnServer(lib_key, server_id) => {
            // Switch to a library on a different server
            // First, find the server and connect to it
            if let Some(server) = state.available_servers.iter().find(|s| s.client_identifier == server_id).cloned() {
                let token = client.token().map(|s| s.to_string());

                if let Some(token) = token {
                    state.advance_library_generation();
                    reset_library_data(state);
                    // Clear all current data (same as SelectLibrary but more thorough)
                    state.library.artists.clear();
                    state.library.albums.clear();
                    state.library.playlists.clear();
                    state.library.album_genres.clear();
                    state.library.artist_genres.clear();
                    state.library.album_genres.clear();
                    state.library.moods.clear();
                    state.library.styles.clear();
                    state.library.decades.clear();
                    state.library.years.clear();
                    state.library.collections.clear();
                    state.library.countries.clear();
                    state.library.labels.clear();
                    state.library.formats.clear();
                    state.library.studios.clear();
                    state.stations.clear();
                    state.library.all_tracks.clear();
                    state.library.track_artists.clear();
                    state.library.artist_aliases.clear();
                    state.library.album_display_artist.clear();
                    state.library.compilations.albums.clear();
                    state.library.compilations.artist_keys.clear();
                    state.library.compilations.track_artist_keys.clear();
                    state.library.compilations.artist_map.clear();
                    state.library.compilations.single_artist.clear();
                    state.library.compilations.detected = false;
                    state.library.compilations.detecting = false;
                    state.library.compilations.detection_request_id = state.library.compilations.detection_request_id.wrapping_add(1);

                    state.library.selected_artist_albums.clear();
                    state.library.selected_album_tracks.clear();
                    state.library.tag_albums.clear();
                    state.folder_state = None;
                    clear_folder_cache(state);
                    state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    state.subfolder_preload_active = false;
                    clear_playlist_track_cache(state);
                    state.search.results = None;
                    state.similar = crate::app::state::SimilarViewState::default();
                    state.related = crate::app::state::RelatedViewState::default();
                    state.list_filter.deactivate();
                    state.list_state.reset();
                    state.cache_mgmt.category_timestamps.clear();
                    state.cache_mgmt.dirty = false;
                    state.artist_nav = crate::app::state::BrowseNavigationState::new();
                    state.tag_nav = crate::app::state::BrowseNavigationState::new();
                    state.playlist_nav = crate::app::state::BrowseNavigationState::new();
                    state.station_nav = crate::app::state::StationNavigationState::new();
                    state.artwork.current_thumb = None;
                    state.artwork.current_data = None;
                    state.artwork.pending_thumb = None;
                    state.artwork.loading = false;
                    state.artwork.grid_pending.clear();
                    state.artwork.clear_grid_art();

                    // Stop playback
                    if state.playback.status != PlayStatus::Stopped {
                        if let Some(track) = state.current_track().cloned() {
                            helpers::report_playback_stop_to_plex(
                                &track, state.playback.position_ms, false,
                                state.plex_session_id.clone(), client,
                            );
                        }
                    }
                    audio.stop();
                    state.playback.request_id = audio.playback_id();
                    audio.track_cache.flush();
                    state.playback.status = PlayStatus::Stopped;
                    state.playback.position_ms = 0;
                    state.playback.duration_ms = 0;
                    state.queue.tracks.clear();
                    state.queue.index = None;
                    state.queue.original.clear();
                    state.radio.clear();
                    state.playback_mode = PlaybackMode::Queue;
                    state.adventure = crate::app::state::AdventureState::default();

                    state.library_loading = true;
                    let server_name = server.name.clone();
                    state.set_status(format!("Connecting to {}...", server_name));

                    let client_id = client.client_identifier().to_string();
                    let event_tx = event_tx.clone();
                    let spawn_server_name = server_name.clone();
                    let library_generation = state.library_generation;

                    tokio::spawn(async move {
                        if let Some(url) = helpers::find_working_connection(&server, &token, &client_id).await {
                            let event = AuthEvent::ServerConnectionSucceeded {
                                server_name: spawn_server_name.clone(),
                                url: url.clone(),
                            };
                            let _ = event_tx
                                .send(Event::for_library(library_generation, event))
                                .await;

                            // Now load libraries from this server
                            let libraries = match crate::plex::PlexClient::new_with_url(
                                &url,
                                Some(&token),
                                &client_id,
                            ) {
                                Ok(new_client) => new_client.get_libraries().await,
                                Err(error) => Err(error),
                            };
                            let result = libraries.map_err(|error| {
                                AsyncError::from_api("Failed to load libraries", &error)
                            });
                            let event = DataEvent::LibrariesLoaded {
                                    server_url: Some(url),
                                    result,
                                };
                            let _ = event_tx
                                .send(Event::for_library(library_generation, event))
                                .await;
                        } else {
                            let event = AuthEvent::ServerConnectionFailed {
                                server_name: spawn_server_name,
                            };
                            let _ = event_tx
                                .send(Event::for_library(library_generation, event))
                                .await;
                        }
                    });

                    // Update server tracking
                    state.active_server_id = Some(server_id);
                    state.active_library = Some(lib_key.clone());
                    state.keep_subfolder_cache = config.libraries.per_library
                        .get(lib_key.as_str())
                        .map(|s| s.keep_subfolder_cache)
                        .unwrap_or(false);

                    // ServerConnectionSucceeded persists the verified URL.
                }
            } else {
                state.set_error("Server not found".to_string());
            }
        }
        SettingsAction::SaveSettings => {
            config.libraries.default_library = state.active_library.clone();
            save_config_in_background(event_tx, config, "save library selection");
        }
        SettingsAction::ClearLibraryCache => {
            state.set_status("Clearing library cache...".to_string());
            let tx = event_tx.clone();
            tokio::task::spawn_blocking(move || {
                let result = LibraryCache::new()
                    .ok_or_else(|| "Cache not available".to_string())
                    .and_then(|cache| cache.clear_all().map_err(|error| error.to_string()));
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::LibraryCacheCleared(result).into(),
                ));
            });
        }
        SettingsAction::LibraryCacheCleared(result) => match result {
            Ok(count) => {
                tracing::info!("Cleared {} library cache files", count);
                apply_library_cache_clear(count, event_tx, state, client);
            }
            Err(error) => state.set_error(format!("Failed to clear library cache: {error}")),
        },
        SettingsAction::ClearArtworkCache => {
            state.set_status("Clearing artwork cache...".to_string());
            let tx = event_tx.clone();
            tokio::task::spawn_blocking(move || {
                let removed = crate::plex::ArtworkCache::default().clear_all();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::ArtworkCacheCleared(removed).into(),
                ));
            });
        }
        SettingsAction::ArtworkCacheCleared(removed) => {
            tracing::info!("Cleared {} artwork cache files", removed);
            apply_artwork_cache_clear(removed, state);
        }
        SettingsAction::AllCachesCleared { library, artwork } => {
            match library {
                Ok(count) => apply_library_cache_clear(count, event_tx, state, client),
                Err(error) => {
                    state.set_error(format!("Failed to clear library cache: {error}"));
                }
            }
            apply_artwork_cache_clear(artwork, state);
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
            state.set_status("Caches cleared; reloading library...".to_string());
        }
        SettingsAction::ClearSubfolderCache => {
            let count = state.folder_contents_cache.len();
            clear_folder_cache(state);
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;
            state.cache_mgmt.dirty = true;

            tracing::info!("Cleared {} subfolder cache entries", count);
            state.set_status(format!("Cleared {} subfolder cache entries", count));
        }
        SettingsAction::RefreshCacheStats => {
            // Two-stage estimate. First, a synchronous in-memory
            // measurement so the Cache tab has numbers to show
            // immediately — even right after sign-in when the
            // on-disk cache file hasn't been written yet. Second, a
            // background disk read that overrides those numbers
            // with the precise on-disk figures when (and only when)
            // a real cache file exists.
            fn measure_slice<T>(items: &[T]) -> u64 {
                items.len().saturating_mul(std::mem::size_of::<T>()) as u64
            }
            fn measure_map<K, V>(items: &std::collections::HashMap<K, V>) -> u64 {
                items
                    .len()
                    .saturating_mul(
                        std::mem::size_of::<K>()
                            .saturating_add(std::mem::size_of::<V>())
                            .saturating_add(16),
                    ) as u64
            }
            let est_breakdown: Vec<(String, u64)> = vec![
                ("artists".into(),         measure_slice(&state.library.artists)),
                ("albums".into(),          measure_slice(&state.library.albums)),
                ("tracks".into(),          measure_slice(&state.library.all_tracks)),
                ("playlist tracks".into(), measure_map(&state.playlist_tracks_cache)),
                ("genres".into(),
                    measure_slice(&state.library.album_genres)
                    + measure_slice(&state.library.artist_genres)
                    + measure_slice(&state.library.moods)
                    + measure_slice(&state.library.styles)
                    + measure_slice(&state.library.decades)
                    + measure_slice(&state.library.years)
                    + measure_slice(&state.library.collections)
                    + measure_slice(&state.library.countries)
                    + measure_slice(&state.library.labels)
                    + measure_slice(&state.library.formats)
                    + measure_slice(&state.library.studios)),
                ("stations".into(),        measure_slice(&state.stations)),
                ("folders".into(),         measure_map(&state.folder_contents_cache)),
            ];
            let est_total: u64 = est_breakdown.iter().map(|(_, v)| *v).sum();
            state.library_cache_stats = Some((est_total, est_breakdown));

            let event_tx = event_tx.clone();
            let lib_key = state.active_library.clone();
            let server_id = state.active_server_id.clone();
            let library_generation = state.library_generation;
            tokio::task::spawn_blocking(move || {
                let cache = crate::plex::ArtworkCache::default();
                let (count, total_bytes) = cache.stats();
                let _ = event_tx.blocking_send(crate::app::event::ArtworkEvent::ArtworkCacheStats { count, total_bytes }.into());
                if let (Some(cache), Some(key)) = (crate::plex::LibraryCache::new(), lib_key) {
                    let breakdown = cache.library_breakdown_scoped(server_id.as_deref(), &key);
                    // Only override the synchronous in-memory
                    // estimate if a real cache file exists on disk.
                    // Posting `(0, [])` here would clobber the
                    // estimate with all-dashes immediately after
                    // sign-in.
                    if !breakdown.is_empty() {
                        let total_bytes = cache.library_size_scoped(server_id.as_deref(), &key);
                        let event = crate::app::event::CacheEvent::LibraryCacheStats {
                            total_bytes,
                            breakdown,
                        };
                        let _ = event_tx.blocking_send(Event::for_library(
                            library_generation,
                            event,
                        ));
                    }
                }
                let wf = crate::plex::WaveformCache::default();
                let (wf_count, wf_bytes) = wf.stats();
                let _ = event_tx.blocking_send(crate::app::event::CacheEvent::WaveformCacheStats { count: wf_count, total_bytes: wf_bytes }.into());
            });
        }
        SettingsAction::RefreshAllCache => {
            // One worker owns the whole disk phase. The reducer does not start
            // replacement preloads or refresh statistics until both clears
            // have actually completed.
            state.set_status("Clearing caches...".to_string());
            let tx = event_tx.clone();
            tokio::task::spawn_blocking(move || {
                let library = LibraryCache::new()
                    .ok_or_else(|| "Cache not available".to_string())
                    .and_then(|cache| cache.clear_all().map_err(|error| error.to_string()));
                let artwork = crate::plex::ArtworkCache::default().clear_all();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::AllCachesCleared { library, artwork }.into(),
                ));
            });
        }
        SettingsAction::StartSubfolderCrawl => {
            use crate::app::handlers::helpers::SubfolderPreloadResult;
            match helpers::maybe_start_subfolder_preload(event_tx, state, client) {
                SubfolderPreloadResult::Started => {
                    state.set_status("Subfolder crawl started".to_string());
                }
                SubfolderPreloadResult::AlreadyActive => {
                    state.set_status("Subfolder crawl already running".to_string());
                }
                SubfolderPreloadResult::AllCached { count } => {
                    state.set_status(format!("All {} folder listings already cached and fresh", count));
                }
                SubfolderPreloadResult::NoRootFolders => {
                    state.set_status("No root folders loaded yet".to_string());
                }
                SubfolderPreloadResult::NoSubfolders => {
                    state.set_status("No subfolders to crawl (root has only tracks)".to_string());
                }
                SubfolderPreloadResult::NoLibrary => {
                    state.set_status("No library selected".to_string());
                }
            }
        }
        SettingsAction::StopSubfolderCrawl => {
            state.subfolder_preload_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.subfolder_preload_active = false;
            state.set_status("Subfolder crawl stopped".to_string());
        }
        SettingsAction::ToggleKeepSubfolderCache => {
            if let Some(lib_key) = state.active_library.clone() {
                state.keep_subfolder_cache = !state.keep_subfolder_cache;
                let entry = config.libraries.per_library.entry(lib_key).or_default();
                entry.keep_subfolder_cache = state.keep_subfolder_cache;
                save_config_in_background(event_tx, config, "save subfolder cache preference");
                state.set_status(if state.keep_subfolder_cache {
                    "subfolder cache: keep indefinitely".to_string()
                } else {
                    "subfolder cache: purge after 32 days".to_string()
                });
            }
        }

        SettingsAction::DiscoverPlayers => {
            if let Some(token) = client.token().map(str::to_owned) {
                let (username, has_plex_pass) = match &state.connection {
                    ConnectionState::Connected { username, has_plex_pass }
                    | ConnectionState::Degraded { username, has_plex_pass, .. } => {
                        (Some(username.clone()), *has_plex_pass)
                    }
                    _ => (None, false),
                };
                let stored = crate::plex::StoredAuth {
                    token: token.into(),
                    user_id: None,
                    username,
                    client_identifier: client.client_identifier().to_string(),
                    server_url: client.server_url().map(str::to_owned),
                    server_identifier: state.active_server_id.clone(),
                    server_name: state.active_server_name().map(str::to_owned),
                    has_plex_pass,
                };
                state.remote.discovering = true;
                let event_tx = LibraryEventSender::new(
                    event_tx.clone(),
                    state.library_generation,
                );
                tokio::spawn(async move {
                    let auth = match PlexAuth::from_stored_auth(&stored) {
                        Ok(auth) => auth,
                        Err(error) => {
                            let _ = event_tx.send(
                                RemoteEvent::PlayerDiscoveryFailed(error.to_string()).into()
                            ).await;
                            return;
                        }
                    };
                    match auth.get_players(&stored.token).await {
                        Ok(players) => {
                            let _ = event_tx.send(RemoteEvent::PlayersDiscovered(players).into()).await;
                        }
                        Err(e) => {
                            let _ = event_tx.send(RemoteEvent::PlayerDiscoveryFailed(e.to_string()).into()).await;
                        }
                    }
                });
            } else {
                state.set_error("No authentication token available".to_string());
            }
        }
        SettingsAction::SetOutputTarget(target) => {
            use crate::app::state::OutputTarget;
            let was_playing = matches!(state.playback.status, PlayStatus::Playing | PlayStatus::Paused);

            match &target {
                OutputTarget::Local => {
                    // Switching back to local: stop remote playback if active
                    if let OutputTarget::Remote { player_id, player_uri, .. } = &state.remote.output_target {
                        let target_id = player_id.clone();
                        let p_uri = player_uri.clone();
                        let token = client.shared_token_or_empty();
                        let client_id = client.client_identifier().to_string();
                        let server_url = client.server_url().unwrap_or("").to_string();
                        let machine_id = state.active_server_id.clone()
                            .or_else(|| state.available_servers.first()
                                .map(|server| server.client_identifier.clone()))
                            .unwrap_or_default();
                        tokio::spawn(async move {
                            if let Ok(rc) = crate::plex::RemotePlayerClient::new(
                                token, client_id, target_id, server_url, machine_id, p_uri,
                            ) {
                                let _ = rc.stop().await;
                            }
                        });
                    }
                    state.remote.output_target = OutputTarget::Local;
                    state.remote.playback = crate::app::state::RemotePlaybackState::default();

                    if was_playing && state.current_track().is_some() {
                        // Transfer playback to local
                        helpers::play_current_track(event_tx, state, client, audio);
                        state.set_status("Output: Local".to_string());
                    } else {
                        state.playback.status = PlayStatus::Stopped;
                        state.playback.position_ms = 0;
                        state.set_status("Output: Local".to_string());
                    }
                }
                OutputTarget::Remote { player_name, .. } => {
                    let name = player_name.clone();
                    // Stop local audio
                    audio.stop();
                    state.playback.request_id = audio.playback_id();
                    state.remote.output_target = target;
                    state.remote.playback = crate::app::state::RemotePlaybackState::default();

                    if was_playing && state.current_track().is_some() {
                        // Transfer playback to remote
                        helpers::play_current_track(event_tx, state, client, audio);
                        state.set_status(format!("Output: {}", name));
                    } else {
                        state.playback.status = PlayStatus::Stopped;
                        state.playback.position_ms = 0;
                        state.set_status(format!("Output: {}", name));
                    }
                }
            }
        }

        SettingsAction::SetAdventureLength(length) => {
            state.adventure.requested_length = length.clamp(5, 100);
            state.popups.input_dialog = None;
            state.adventure.generating = true;
            state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
            let request_id = state.adventure_request_id;
            state.set_status("Adventure: generating sonic bridge...".to_string());

            // Generate the adventure
            if let (Some(start), Some(end)) = (state.adventure.start_track.clone(), state.adventure.end_track.clone()) {
                let requested_length = state.adventure.requested_length;
                let library_key = state.active_library.clone();
                let request_client = client.clone();
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    let result = crate::services::generate_adventure_for_library(
                        &request_client,
                        &start,
                        &end,
                        requested_length,
                        library_key.as_deref(),
                    )
                    .await
                    .map_err(|error| {
                        crate::app::action::AsyncError::from_api(
                            "Adventure generation failed",
                            &error,
                        )
                    });
                    let _ = tx
                        .send(Event::Effect(
                            SettingsAction::AdventureGenerated { request_id, result }.into(),
                        ))
                        .await;
                });
            } else {
                // Fully reset adventure state
                state.adventure = crate::app::state::AdventureState::default();
                state.set_error("Adventure: missing start or end track".to_string());
            }
        }
        SettingsAction::CancelAdventure => {
            state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
            state.adventure = crate::app::state::AdventureState::default();
            state.popups.input_dialog = None;
            state.clear_status();
        }
        SettingsAction::AdventureComplete(tracks) => {
            // This is handled inline in SetAdventureLength for simplicity
            state.adventure = crate::app::state::AdventureState::default();
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            state.queue.tracks = tracks;
            state.queue.index = Some(0);
            state.queue.original.clear();
            state.queue.sort_mode = QueueSortMode::QueueOrder;
            state.playback_mode = PlaybackMode::Queue;
            state.set_view(View::Queue);
            helpers::play_current_track(event_tx, state, client, audio);
        }
        SettingsAction::AdventureError(msg) => {
            state.adventure.generating = false;
            state.set_error(format!("Adventure failed: {}", msg));
        }
        SettingsAction::AdventureGenerated { request_id, result } => {
            if state.adventure_request_id != request_id || !state.adventure.generating {
                return Ok(vec![]);
            }
            match result {
                Ok(tracks) if tracks.len() > 2 => {
                    state.connection.mark_healthy();
                    state.adventure = crate::app::state::AdventureState::default();
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    state.queue.tracks = tracks;
                    state.queue.index = Some(0);
                    state.queue.original.clear();
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                    state.playback_mode = PlaybackMode::Queue;
                    state.set_view(View::Queue);
                    helpers::play_current_track(event_tx, state, client, audio);
                    state.set_status(format!(
                        "Adventure: {} tracks ready!",
                        state.queue.tracks.len()
                    ));
                }
                Ok(_) => {
                    state.adventure = crate::app::state::AdventureState::default();
                    state.set_error("Adventure: no similar tracks found for these songs. Try different tracks with sonic analysis data.".to_string());
                }
                Err(error) => {
                    state.adventure = crate::app::state::AdventureState::default();
                    if error.connection_error {
                        state.connection.mark_degraded(error.message.clone());
                    }
                    state.set_error(error.message);
                }
            }
        }
        SettingsAction::ToggleExternalSearchService(target) => {
            use crate::services::external_search::SearchTarget;
            match target {
                SearchTarget::AppleMusic => {
                    config.ui.enable_apple_music_search = !config.ui.enable_apple_music_search;
                    state.external_search.apple_music = config.ui.enable_apple_music_search;
                }
                SearchTarget::Spotify => {
                    config.ui.enable_spotify_search = !config.ui.enable_spotify_search;
                    state.external_search.spotify = config.ui.enable_spotify_search;
                }
                SearchTarget::YouTube => {
                    config.ui.enable_youtube_search = !config.ui.enable_youtube_search;
                    state.external_search.youtube = config.ui.enable_youtube_search;
                }
            }
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::ToggleTallMode => {
            state.tall_mode = !state.tall_mode;
            config.ui.tall_mode = state.tall_mode;
            state.set_status(format!(
                "tall mode: {}",
                if state.tall_mode { "on" } else { "off" }
            ));
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::ToggleMillerLayout => {
            state.miller_layout = state.miller_layout.toggled();
            // TUI ribbon scroll state is reset on every layout toggle
            // so a freshly-toggled mode starts at column 0 — a stale
            // offset would dangle out of bounds.
            state.miller_scroll_col = 0;
            state.miller_scroll_manual = false;
            state.miller_h_drag_grab = None;
            config.ui.miller_layout = state.miller_layout;
            state.set_status(format!("miller layout: {}", state.miller_layout.name()));
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::ToggleSectionVisibility(section) => {
            if let Some(pos) = state.hidden_sections.iter().position(|s| s == &section) {
                state.hidden_sections.remove(pos);
            } else {
                state.hidden_sections.push(section);
            }
            // If the active category was just hidden, fall back to the
            // first visible section.
            if state.hidden_sections.contains(&state.browse_category) {
                let fallback = crate::app::state::BrowseCategory::all().iter()
                    .find(|c| !state.hidden_sections.contains(c))
                    .copied()
                    .unwrap_or(crate::app::state::BrowseCategory::Library);
                state.set_browse_category(fallback, false);
            }
            config.ui.hidden_sections = state.hidden_sections.clone();
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::SavePlaylistView { library_key, playlist_key, view } => {
            config.set_playlist_view(&library_key, &playlist_key, view);
            // Mirror the change onto AppState so event handlers
            // (which don't get &Config) see the latest value.
            let lib = state.playlist_views
                .entry(library_key.clone())
                .or_default();
            if view.is_default() {
                lib.remove(&playlist_key);
                if lib.is_empty() {
                    state.playlist_views.remove(&library_key);
                }
            } else {
                lib.insert(playlist_key.clone(), view);
            }
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::PrunePlaylistViews { library_key, live_playlist_keys } => {
            // Snapshot the keys we're about to drop so the saver
            // only fires when something actually changed (no point
            // rewriting the config every PlaylistsLoaded).
            let before = config.ui.library_view_settings.get(&library_key)
                .map(|l| l.playlists.len())
                .unwrap_or(0);
            config.prune_stale_playlist_views(&library_key, &live_playlist_keys);
            let after = config.ui.library_view_settings.get(&library_key)
                .map(|l| l.playlists.len())
                .unwrap_or(0);
            // Mirror the prune onto AppState.
            if let Some(lib) = state.playlist_views.get_mut(&library_key) {
                lib.retain(|k, _| live_playlist_keys.contains(k));
                if lib.is_empty() {
                    state.playlist_views.remove(&library_key);
                }
            }
            if before != after {
                follow_ups.push(SettingsAction::SaveSettings.into());
            }
        }
        SettingsAction::ArtistRadioComplete(tracks) => {
            if tracks.is_empty() {
                state.set_error("Artist radio: no tracks returned".to_string());
                return Ok(vec![]);
            }
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            let count = tracks.len();
            state.queue.tracks = tracks;
            state.queue.index = Some(0);
            state.queue.selected.clear();
            state.queue.original.clear();
            state.queue.sort_mode = QueueSortMode::QueueOrder;
            state.playback_mode = PlaybackMode::Queue;
            state.list_state.queue_index = 0;
            state.set_view(View::Queue);
            state.set_status(format!("Artist radio: {} tracks", count));
            helpers::play_current_track(event_tx, state, client, audio);
        }
    }
    Ok(follow_ups)
}
