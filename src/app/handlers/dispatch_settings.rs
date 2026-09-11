//! Settings persistence, cache management and shared adventure completion.

use crate::app::action::SettingsAction;
use crate::app::state::{PlaybackMode, QueueSortMode, SettingsSection, View};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::config::Config;

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

use super::helpers;

static CONFIG_SAVE_REVISION: AtomicU64 = AtomicU64::new(1);
static LAST_CONFIG_SAVE_REVISION: AtomicU64 = AtomicU64::new(0);
static CONFIG_SAVE_LOCK: Mutex<()> = Mutex::new(());

/// Serialize atomic config-file replacements on a blocking worker. A monotonic
/// revision prevents an older worker that starts late from overwriting a newer
/// in-memory settings snapshot.
pub(crate) fn save_config_in_background(
    event_tx: &mpsc::Sender<Event>,
    config: &Config,
    operation: &'static str,
) {
    let revision = CONFIG_SAVE_REVISION.fetch_add(1, Ordering::Relaxed);
    let snapshot = config.clone();
    let event_tx = event_tx.clone();
    crate::app::tasks::spawn_blocking(move || {
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
    crate::app::tasks::spawn_blocking(move || drop(value));
}

fn apply_library_cache_clear(count: usize, event_tx: &mpsc::Sender<Event>, state: &mut AppState) {
    {
        state.sources.nav_tasks.remove("audiomuse");
        state
            .sources
            .nav_tasks
            .remove(crate::app::sources::audiomuse::SYNC_SLOT);
        drop_in_background(state.sources.audiomuse.snapshot.take());
        state.sources.audiomuse.refresh_failed = false;
        state.cache_mgmt.next_refresh_check = None;
        if matches!(
            state.sources.nav_collection,
            Some(crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(_))
        ) {
            state.set_browse_category(crate::app::state::BrowseCategory::Library, true);
        }
        // Keep the visible catalog if reload fails. Clear invalidates old disk writes.
        state.library_cache_stats = Some((0, vec![]));
        if state.sources.active.navidrome().is_some() {
            crate::app::sources::navidrome::reload(state, event_tx);
        } else if state.sources.active.folder().is_some() {
            let action = crate::app::action::FolderAction::RefreshSubfolder(String::new());
            // Folder listings need no player; dispatch the refresh via the loop.
            let tx = event_tx.clone();
            crate::app::tasks::spawn(async move {
                let _ = tx.send(Event::Effect(action.into())).await;
            });
        }
        state.set_status(
            if matches!(
                state.sources.active,
                crate::app::sources::ActiveSource::None
            ) {
                format!("Cleared {count} cache files")
            } else {
                format!("Cleared {count} cache files; refreshing")
            },
        );
    }
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

/// Dispatch settings/cache/adventure actions. Returns follow-up actions.
pub async fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: SettingsAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        SettingsAction::PersistenceFailed { operation, error } => {
            state.set_error(format!("Failed to {operation}: {error}"));
        }

        SettingsAction::OpenSettings => {
            follow_ups.push(crate::app::action::SearchAction::ManageLibraries.into());
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }
        SettingsAction::SettingsSelect => match state.settings_state.section {
            SettingsSection::Textamp => {
                use crate::app::state::TextampSetting;
                let Some(item) = state
                    .textamp_settings()
                    .get(state.settings_state.item_index)
                    .copied()
                else {
                    return Ok(vec![]);
                };
                match item {
                    TextampSetting::Theme(name) => {
                        state.theme = name;
                        state.set_status(format!("Theme: {}", name.display_name()));
                        config.ui.theme = name.config_name().to_string();
                        save_config_in_background(event_tx, config, "save theme preference");
                    }
                    TextampSetting::Artwork(mode) => {
                        state.artwork.mode = mode;
                        state.set_status(format!("Artwork: {}", mode.name()));
                        config.ui.artwork_mode = mode.name().to_string();
                        save_config_in_background(event_tx, config, "save artwork preference");
                    }

                    TextampSetting::Transcode => {
                        let options = [0u32, 128, 192, 256, 320];
                        let current = options
                            .iter()
                            .position(|&v| v == state.transcode_kbps)
                            .unwrap_or(0);
                        let next = options[(current + 1) % options.len()];
                        state.transcode_kbps = next;
                        config.playback.transcode_kbps = next;
                        save_config_in_background(event_tx, config, "save transcode preference");
                        state.set_status(if next == 0 {
                            "Streaming: original (direct play)".into()
                        } else {
                            format!("Streaming: transcode to {next}kbps")
                        });
                    }
                    TextampSetting::Sidebar(section) => {
                        follow_ups.push(SettingsAction::ToggleSectionVisibility(section).into());
                    }
                    TextampSetting::ExternalSearch(target) => {
                        follow_ups.push(SettingsAction::ToggleExternalSearchService(target).into())
                    }
                }
            }
            SettingsSection::Libraries => {
                follow_ups.push(crate::app::action::SearchAction::ManageLibraries.into())
            }
            SettingsSection::About => {}
        },

        SettingsAction::SaveSettings => {
            save_config_in_background(event_tx, config, "save library selection");
        }
        SettingsAction::RescanSourceCache(choice) => {
            crate::app::sources::cache::start(choice, true, state, event_tx);
        }
        SettingsAction::SourceCacheScanned {
            choice,
            request,
            manual,
            result,
        } => {
            if !crate::app::sources::cache::completed(&choice, request, &result, state) {
                return Ok(vec![]);
            }
            if result.is_ok() && manual && crate::app::sources::cache::active(&choice, state) {
                state.sources.listing = None;
                state.sources.list_id = state.sources.list_id.wrapping_add(1);
                if state.sources.active.navidrome().is_some() {
                    state
                        .sources
                        .nav_tasks
                        .remove(crate::app::sources::audiomuse::SYNC_SLOT);
                    let analysis_failed = result.as_ref().is_ok_and(|scan| scan.warning.is_some());
                    if !analysis_failed {
                        state.sources.audiomuse.snapshot = None;
                    }
                    state.sources.audiomuse.refresh_failed = analysis_failed;
                    state.cache_mgmt.category_timestamps.clear();
                    state.cache_mgmt.failures.clear();
                    crate::app::sources::navidrome::open(state, event_tx);
                } else {
                    follow_ups.push(crate::app::action::FolderAction::LoadFolderRoot.into());
                }
            }
            if let Ok(crate::app::sources::cache::ScanResult {
                warning: Some(warning),
                ..
            }) = &result
            {
                if manual {
                    state.set_error(format!("Catalog refreshed; {warning}"));
                }
            }
            if let Err(error) = result {
                if manual {
                    state.set_error(format!("Re-scan {}: {error}", choice.label()));
                } else {
                    tracing::warn!("Folder tree scan: {error}");
                }
            }
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }
        SettingsAction::ClearSourceCache(choice) => {
            crate::app::sources::cache::clear(choice, state, event_tx);
        }
        SettingsAction::SourceCacheCleared {
            choice,
            request,
            result,
        } => {
            if !crate::app::sources::cache::cleared(&choice, request, &result, state) {
                return Ok(vec![]);
            }
            match result {
                Ok(count) => state.set_status(format!("Cleared {count} cache files")),
                Err(error) => state.set_error(format!("Clear {} cache: {error}", choice.label())),
            }
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }
        SettingsAction::CacheSizes { request, entries } => {
            if request == state.settings_state.cache_request {
                state.settings_state.cache_task = None;
                state.settings_state.cache_entries = entries;
            }
        }
        SettingsAction::ClearLibraryCache => {
            state.set_status("Clearing library cache...".to_string());
            let tx = event_tx.clone();
            crate::app::tasks::spawn_blocking(move || {
                let result = crate::library::cache::clear_all().map_err(|e| e.to_string());
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::LibraryCacheCleared(result).into(),
                ));
            });
        }
        SettingsAction::LibraryCacheCleared(result) => match result {
            Ok(count) => {
                tracing::info!("Cleared {} library cache files", count);
                apply_library_cache_clear(count, event_tx, state);
                follow_ups.push(SettingsAction::RefreshCacheStats.into());
            }
            Err(error) => state.set_error(format!("Failed to clear library cache: {error}")),
        },
        SettingsAction::ClearArtworkCache => {
            state.set_status("Clearing artwork cache...".to_string());
            let tx = event_tx.clone();
            crate::app::tasks::spawn_blocking(move || {
                let removed = crate::media::ArtworkCache::default().clear_all();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::ArtworkCacheCleared(removed).into(),
                ));
            });
        }
        SettingsAction::ArtworkCacheCleared(removed) => {
            tracing::info!("Cleared {} artwork cache files", removed);
            apply_artwork_cache_clear(removed, state);
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }
        SettingsAction::AllCachesCleared { library, artwork } => {
            match library {
                Ok(count) => apply_library_cache_clear(count, event_tx, state),
                Err(error) => {
                    state.set_error(format!("Failed to clear library cache: {error}"));
                }
            }
            apply_artwork_cache_clear(artwork, state);
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }

        SettingsAction::RefreshCacheStats => {
            state.settings_state.cache_request = state.settings_state.cache_request.wrapping_add(1);
            let request = state.settings_state.cache_request;
            let choices = crate::app::sources::library_choices(state);
            let event_tx = event_tx.clone();
            let task = crate::app::tasks::spawn_blocking(move || {
                let entries = choices
                    .into_iter()
                    .map(|choice| {
                        let bytes = choice
                            .cache_store()
                            .and_then(|s| s.bytes())
                            .map_err(|e| e.to_string());
                        (choice, bytes)
                    })
                    .collect();
                let _ = event_tx.blocking_send(Event::Effect(
                    SettingsAction::CacheSizes { request, entries }.into(),
                ));
                let (count, total_bytes) = crate::media::ArtworkCache::default().stats();
                let _ = event_tx.blocking_send(
                    crate::app::event::ArtworkEvent::ArtworkCacheStats { count, total_bytes }
                        .into(),
                );
                let (count, total_bytes) = crate::media::WaveformCache::default().stats();
                let _ = event_tx.blocking_send(
                    crate::app::event::CacheEvent::WaveformCacheStats { count, total_bytes }.into(),
                );
            });
            state.settings_state.cache_task = Some(crate::app::tasks::TaskLease::new(&task));
        }
        SettingsAction::RefreshAllCache => {
            // One worker owns the whole disk phase. The reducer does not start
            // replacement preloads or refresh statistics until both clears
            // have actually completed.
            state.set_status("Clearing caches...".to_string());
            let tx = event_tx.clone();
            crate::app::tasks::spawn_blocking(move || {
                let library = crate::library::cache::clear_all().map_err(|e| e.to_string());
                let artwork = crate::media::ArtworkCache::default().clear_all();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::AllCachesCleared { library, artwork }.into(),
                ));
            });
        }

        SettingsAction::CancelAdventure => {
            state.sources.sonic_tasks.remove("adventure");
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
            state.set_playback_mode(PlaybackMode::Queue);
            state.set_view(View::Queue);
            helpers::play_current_track(event_tx, state, audio);
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
                    state.adventure = crate::app::state::AdventureState::default();
                    if state.playback_mode == PlaybackMode::Radio {
                        state.radio.clear();
                    }
                    state.queue.tracks = tracks;
                    state.queue.index = Some(0);
                    state.queue.original.clear();
                    state.queue.sort_mode = QueueSortMode::QueueOrder;
                    state.set_playback_mode(PlaybackMode::Queue);
                    state.set_view(View::Queue);
                    helpers::play_current_track(event_tx, state, audio);
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
            use crate::app::state::SidebarSection;
            match section {
                SidebarSection::Category(c) => {
                    if state.hidden_sections.contains(&c) {
                        state.hidden_sections.retain(|s| *s != c);
                    } else {
                        state.hidden_sections.push(c);
                    }
                }
                SidebarSection::Collection(c) => {
                    if state.hidden_collections.contains(&c) {
                        state.hidden_collections.retain(|s| *s != c);
                    } else {
                        state.hidden_collections.push(c);
                    }
                }
            }
            // If the active category was just hidden, fall back to the
            // first visible section.
            if state.hidden_sections.contains(&state.browse_category)
                || state
                    .sources
                    .nav_collection
                    .is_some_and(|c| state.hidden_collections.contains(&c))
            {
                let fallback = state
                    .sidebar_sections()
                    .into_iter()
                    .find_map(|s| match s {
                        SidebarSection::Category(c) if !s.hidden(state) => Some(c),
                        _ => None,
                    })
                    .unwrap_or(state.browse_category);
                state.set_browse_category(fallback, false);
            }
            state.category_column_index = state.row_index_for_category(state.browse_category);
            state.scroll.category = None;
            config.ui.hidden_sections = state.hidden_sections.clone();
            config.ui.hidden_collections = state.hidden_collections.clone();
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::SavePlaylistView {
            library_key,
            playlist_key,
            view,
        } => {
            config.set_playlist_view(&library_key, &playlist_key, view);
            // Mirror the change onto AppState so event handlers
            // (which don't get &Config) see the latest value.
            let lib = state.playlist_views.entry(library_key.clone()).or_default();
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
        SettingsAction::PrunePlaylistViews {
            library_key,
            live_playlist_keys,
        } => {
            // Snapshot the keys we're about to drop so the saver
            // only fires when something actually changed (no point
            // rewriting the config every PlaylistsLoaded).
            let before = config
                .ui
                .library_view_settings
                .get(&library_key)
                .map(|l| l.playlists.len())
                .unwrap_or(0);
            config.prune_stale_playlist_views(&library_key, &live_playlist_keys);
            let after = config
                .ui
                .library_view_settings
                .get(&library_key)
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
        SettingsAction::ArtistRadioComplete(outcome) => {
            if outcome.items.is_empty() {
                if let Some(error) = outcome.first_error {
                    state.set_error(error.message.clone());
                } else {
                    state.set_error(
                        "Artist radio: the selected artists have no playable tracks".to_string(),
                    );
                }
                return Ok(vec![]);
            }
            // Clear radio state if switching from radio mode
            if state.playback_mode == PlaybackMode::Radio {
                state.radio.clear();
            }
            let count = outcome.items.len();
            state.queue.tracks = outcome.items;
            state.queue.index = Some(0);
            state.queue.selected.clear();
            state.queue.original.clear();
            state.queue.sort_mode = QueueSortMode::QueueOrder;
            state.set_playback_mode(PlaybackMode::Queue);
            state.list_state.queue_index = 0;
            state.set_view(View::Queue);
            if outcome.failed > 0 {
                state.set_status(format!(
                    "Artist radio: {} tracks; {} of {} artist requests failed",
                    count, outcome.failed, outcome.attempted
                ));
            } else {
                state.set_status(format!("Artist radio: {} tracks", count));
            }
            helpers::play_current_track(event_tx, state, audio);
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(follow_ups)
}
