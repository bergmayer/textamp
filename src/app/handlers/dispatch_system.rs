//! System dispatch handlers: Quit, ShowError, ClearError, SetStatus, ClearStatus,
//! RefreshCategory, CycleTheme, LoadArtwork, LoadWaveform.

use crate::app::{Action, AppState, Event};
use crate::cache::LibraryCache;
use crate::api::PlexClient;
use crate::config::Config;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch system-level actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: Action,
    state: &mut AppState,
    client: &mut PlexClient,
) -> Result<Vec<Action>> {
    match action {
        Action::Quit => {
            // Report playback stop to Plex before quitting
            if state.playback.status != crate::app::state::PlayStatus::Stopped {
                if let Some(track) = state.current_track().cloned() {
                    helpers::report_playback_stop_to_plex(
                        &track, state.playback.position_ms, false,
                        state.plex_session_id.clone(), client,
                    );
                }
            }

            // Save cache to disk before quitting
            if let Some(lib_key) = &state.active_library {
                use crate::cache::CacheData;

                let mut cache_data = CacheData::new(lib_key);

                // Core library data
                cache_data.artists = state.artists.clone();
                cache_data.albums = state.albums.clone();
                cache_data.playlists = state.playlists.clone();

                // Folder data - extract root folder items only if they belong to this library
                if let Some(ref folder_state) = state.folder_state {
                    if folder_state.library_key == *lib_key {
                        if let Some(root_col) = folder_state.columns.first() {
                            cache_data.root_folders = root_col.unshuffled_items().to_vec();
                        }
                    } else {
                        tracing::debug!("Not saving folder_state on quit - belongs to different library (expected {}, got {})",
                            lib_key, folder_state.library_key);
                    }
                }
                // Save cached subfolder contents
                cache_data.folder_contents = state.folder_contents_cache.clone();

                // Genre/mood/style data
                cache_data.genres = state.genres.clone();
                cache_data.artist_genres = state.artist_genres.clone();
                cache_data.album_genres = state.album_genres.clone();
                cache_data.moods = state.moods.clone();
                cache_data.styles = state.styles.clone();

                // Stations
                cache_data.stations = state.stations.clone();

                // Recent content
                cache_data.recently_added_albums = state.recently_added_albums.clone();
                cache_data.recently_played_albums = state.recently_played_albums.clone();

                if let Some(cache) = LibraryCache::new() {
                    if cache.save(&cache_data) {
                        tracing::info!("Cache saved on quit");
                    }
                }
            }

            state.should_quit = true;
        }
        Action::ShowError(msg) => {
            state.set_error(msg);
        }
        Action::ClearError => {
            state.clear_error();
        }
        Action::SetStatus(msg) => {
            state.set_status(msg);
        }
        Action::ClearStatus => {
            state.clear_status();
        }
        Action::RefreshCategory(category) => {
            if let Some(lib_key) = &state.active_library {
                let lib_key = lib_key.clone();
                helpers::spawn_category_refresh(event_tx, category, &lib_key, state, client);
            }
        }
        Action::CycleTheme => {
            state.theme = state.theme.next();
            crate::ui::theme::set_theme(state.theme);
            state.set_status(format!("Theme: {}", state.theme.display_name()));

            // Persist theme to config
            config.ui.theme = state.theme.config_name().to_string();
            if let Err(e) = crate::config::save_config(config) {
                tracing::warn!("Failed to save theme preference: {}", e);
            }
        }
        Action::LoadArtwork => {
            // Get thumb path from current track (clone to avoid borrow)
            let thumb_path = state.current_track()
                .and_then(|t| t.best_thumb().map(|s| s.to_string()));

            if let Some(thumb_path) = thumb_path {
                // Check if we need to load new artwork
                if state.artwork_thumb.as_deref() != Some(&thumb_path) {
                    state.artwork_loading = true;
                    match client.fetch_artwork(&thumb_path, 300).await {
                        Ok(data) => {
                            state.artwork_thumb = Some(thumb_path);
                            state.artwork_data = Some(data);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load artwork: {}", e);
                            state.artwork_thumb = None;
                            state.artwork_data = None;
                        }
                    }
                    state.artwork_loading = false;
                }
            } else {
                // No artwork available or no current track
                state.artwork_thumb = None;
                state.artwork_data = None;
            }
        }
        Action::LoadWaveform => {
            // Only generate waveform if we have a track and don't already have data
            if let Some(track) = state.current_track().cloned() {
                let needs_generation = state.waveform.data.is_none()
                    && !state.waveform.generating
                    && state.waveform.track_key.as_ref() == Some(&track.rating_key);

                if needs_generation {
                    state.waveform.generating = true;
                    let track_key = track.rating_key.clone();
                    let duration_ms = track.duration_ms();
                    let event_tx = event_tx.clone();

                    if let Ok(stream_url) = client.get_stream_url(&track) {
                        let token = client.token().map(|s| s.to_string());

                        tokio::spawn(async move {
                            // Check cache first
                            let cache_dir = dirs::cache_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                .join("textamp")
                                .join("waveforms");
                            let cache = crate::services::WaveformCache::new(cache_dir);

                            if let Some(data) = cache.load(&track_key) {
                                // Cache hit
                                let _ = event_tx.send(Event::WaveformCacheHit {
                                    track_key,
                                    data,
                                }).await;
                                return;
                            }

                            // Cache miss - download and generate
                            let http_client = reqwest::Client::new();
                            let mut request = http_client.get(&stream_url);
                            if let Some(ref token) = token {
                                request = request.header("X-Plex-Token", token);
                            }

                            match request.send().await {
                                Ok(response) => {
                                    match response.bytes().await {
                                        Ok(audio_data) => {
                                            // Generate waveform
                                            match crate::services::generate_waveform(
                                                track_key.clone(),
                                                duration_ms,
                                                audio_data.to_vec(),
                                            ) {
                                                Ok(data) => {
                                                    // Save to cache
                                                    cache.save(&data);
                                                    let _ = event_tx.send(Event::WaveformGenerated {
                                                        track_key,
                                                        data,
                                                    }).await;
                                                }
                                                Err(e) => {
                                                    let _ = event_tx.send(Event::WaveformFailed {
                                                        track_key,
                                                        error: e.to_string(),
                                                    }).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(Event::WaveformFailed {
                                                track_key,
                                                error: format!("Download failed: {}", e),
                                            }).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(Event::WaveformFailed {
                                        track_key,
                                        error: format!("Request failed: {}", e),
                                    }).await;
                                }
                            }
                        });
                    }
                }
            }
        }
        _ => unreachable!("dispatch_system called with non-system action: {:?}", action),
    }
    Ok(vec![])
}
