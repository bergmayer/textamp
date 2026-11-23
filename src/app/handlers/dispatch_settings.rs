//! Settings persistence, cache management and shared adventure completion.

use crate::app::action::SettingsAction;
use crate::app::state::{PlaybackMode, QueueSortMode, SettingsSection, View};
use crate::app::{Action, AppState, Event};
use crate::audio::AudioPlayer;
use crate::config::Config;

use anyhow::Result;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
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
    state.artwork.current_thumb = None;
    state.artwork.fetch = crate::app::state::ArtworkFetch::Idle;
    state.artwork.cache_stats = Some((0, 0));
    state.set_status(format!("Cleared {} artwork cache files", count));
}

/// Dispatch settings/cache/adventure actions. Returns follow-up actions.
pub fn dispatch(
    event_tx: &mpsc::Sender<Event>,
    config: &mut Config,
    action: SettingsAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    match action {
        SettingsAction::RememberPlaylistDestination(key) => {
            if let Some(library) = state.active_library.clone() {
                state
                    .last_playlist_destinations
                    .insert(library.clone(), key.clone());
                if config
                    .ui
                    .last_playlist_destinations
                    .insert(library, key.clone())
                    .as_ref()
                    != Some(&key)
                {
                    follow_ups.push(SettingsAction::SaveSettings.into());
                }
            }
        }
        SettingsAction::PersistCommandHistory => {
            config.ui.command_history = state.palette.history.clone();
            config.ui.desktop_command_history.clear();
            save_config_in_background(event_tx, config, "save command history");
        }
        SettingsAction::SetLibraryRootVisible { root, visible } => {
            crate::app::sidebar::set_library_root_visible(state, root, visible);
            config.ui.desktop_hidden_library_roots = state.desktop_hidden_library_roots.clone();
            config.ui.hidden_sections = state.hidden_sections.clone();
            save_config_in_background(event_tx, config, "save sidebar visibility");
        }

        SettingsAction::ToggleSidebarGroup(title) => {
            if state.desktop_collapsed_sections.contains(&title) {
                state
                    .desktop_collapsed_sections
                    .retain(|item| item != &title);
            } else {
                state.desktop_collapsed_sections.push(title);
            }
            config.ui.desktop_collapsed_sections = state.desktop_collapsed_sections.clone();
            save_config_in_background(event_tx, config, "save sidebar disclosures");
        }

        SettingsAction::PersistenceFailed { operation, error } => {
            state.set_error(format!("Failed to {operation}: {error}"));
        }

        SettingsAction::SetSinglesTrackLimit(limit) => {
            if !(1..=crate::services::singles::MAX_LIMIT).contains(&limit) {
                state.set_error(format!(
                    "Singles track limit must be between 1 and {}",
                    crate::services::singles::MAX_LIMIT
                ));
                return Ok(vec![]);
            }
            state.singles_track_limit = limit;
            config.ui.singles_track_limit = limit;
            if state.desktop_library_root == crate::app::state::LibraryRoot::Singles
                && state.browse_category == crate::app::state::BrowseCategory::Library
            {
                crate::app::album_collections::refresh_singles(state);
            }
            save_config_in_background(event_tx, config, "save singles track limit");
        }
        SettingsAction::SetLargeArtwork(large) => {
            state.large_artwork = large;
            config.ui.desktop_large_album_covers = large;
            save_config_in_background(event_tx, config, "save artwork view options");
        }
        SettingsAction::SetShowCompilationOnlyArtists(show) => {
            config.ui.show_compilation_only_artists = show;
            state.set_compilation_artists_visible(show);
            save_config_in_background(event_tx, config, "save artist view options");
        }
        SettingsAction::SetRemixAllowSameArtist(allowed) => {
            if state.remix_allow_same_artist != allowed {
                state.sources.nav_tasks.remove("remix");
                state.remix_allow_same_artist = allowed;
                config.ui.remix_allow_same_artist = allowed;
                save_config_in_background(event_tx, config, "save queue tools preference");
                state.set_status("Queue tools preference updated".into());
            }
        }
        SettingsAction::OpenSettings => {
            follow_ups.push(crate::app::action::SearchAction::ManageLibraries.into());
            follow_ups.push(SettingsAction::RefreshCacheStats.into());
        }
        SettingsAction::SetTranscodeBitrate(bitrate) => {
            if crate::app::preferences::TRANSCODE_BITRATES.contains(&bitrate) {
                state.transcode_kbps = bitrate;
                config.playback.transcode_kbps = bitrate;
                save_config_in_background(event_tx, config, "save transcode preference");
                state.set_status(if bitrate == 0 {
                    "Streaming: original file, starting with the next track".into()
                } else {
                    format!("Streaming: up to {bitrate} kbps, starting with the next track")
                });
            }
        }
        SettingsAction::ActivatePreference(item) => {
            if state
                .textamp_settings_for(crate::app::preferences::page_for(item))
                .contains(&item)
            {
                follow_ups.extend(apply_preference(event_tx, state, config, item));
            }
        }
        SettingsAction::ReadActivityHistory => {
            let tx = event_tx.clone();
            crate::app::tasks::spawn_blocking(move || {
                let text = crate::util::activity_history_text();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::ActivityHistoryLoaded(text.into()).into(),
                ));
            });
        }
        SettingsAction::ActivityHistoryLoaded(text) => {
            state.settings_state.activity_history = Some(text)
        }
        SettingsAction::SettingsSelect => match state.settings_state.section {
            SettingsSection::Textamp => {
                let Some(item) = state
                    .textamp_settings()
                    .get(state.settings_state.item_index)
                    .copied()
                else {
                    return Ok(vec![]);
                };
                follow_ups.extend(apply_preference(event_tx, state, config, item));
            }
            SettingsSection::Libraries => {
                follow_ups.push(crate::app::action::SearchAction::ManageLibraries.into())
            }
            SettingsSection::About => {
                state.settings_state.about_taps += 1;
                if state.settings_state.about_taps == 4 {
                    state.settings_state.about_taps = 0;
                    follow_ups.push(SettingsAction::ReadActivityHistory.into());
                }
            }
        },

        SettingsAction::SaveSettings => {
            save_config_in_background(event_tx, config, "save library selection");
        }
        SettingsAction::SaveColumnArtwork {
            library_key,
            column_key,
            visible,
        } => {
            if let Some(column_key) = column_key {
                config.set_column_artwork(&library_key, &column_key, visible);
                if state.active_library.as_ref() == Some(&library_key) {
                    state.artwork.column_visibility.insert(column_key, visible);
                }
            }
            follow_ups.push(SettingsAction::SaveSettings.into());
        }
        SettingsAction::ManageLibrary(choice) => {
            follow_ups.extend(crate::app::sources::options::open(choice, state));
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
                && manual
            {
                state.set_error(format!("Catalog refreshed; {warning}"));
            }
            if let Err(error) = result {
                if manual {
                    state.set_error(format!("Re-scan {}: {error}", choice.label()));
                } else {
                    tracing::warn!("Library cache scan: {error}");
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
        SettingsAction::CacheSizes {
            request,
            entries,
            artwork,
        } => {
            if request == state.settings_state.cache_request {
                state.settings_state.cache_task = None;
                state.settings_state.cache_entries = entries;
                state.settings_state.artwork_cache_entries = artwork
                    .into_iter()
                    .map(|(key, count, bytes)| (key, (count, bytes)))
                    .collect();
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
        SettingsAction::ClearArtworkCache(choice) => {
            let Some(key) = choice.artwork_key() else {
                return Ok(vec![]);
            };
            if choice.active(state) {
                state.sources.nav_tasks.remove("artwork");
                state.sources.nav_tasks.remove("art-grid");
                state.artwork.clear_grid_art();
                state.artwork.grid_pending.clear();
                state.artwork.current_data = None;
                state.artwork.current_thumb = None;
                state.artwork.fetch = crate::app::state::ArtworkFetch::Idle;
            }
            crate::app::sources::cache::cancel(&choice, state);
            state.set_status(format!("Clearing artwork for {}...", choice.label()));
            let tx = event_tx.clone();
            crate::app::tasks::spawn_blocking(move || {
                let removed = crate::media::ArtworkCache::for_library(&key).clear_all();
                let _ = tx.blocking_send(Event::Effect(
                    SettingsAction::ArtworkCacheCleared { choice, removed }.into(),
                ));
            });
        }
        SettingsAction::ArtworkCacheCleared { choice, removed } => {
            tracing::info!("Cleared {} artwork cache files", removed);
            if choice.active(state) {
                apply_artwork_cache_clear(removed, state);
                if state.sources.active.navidrome().is_some() && state.current_track().is_some() {
                    follow_ups.push(crate::app::action::SystemAction::LoadArtwork.into());
                }
            } else {
                state.set_status(format!(
                    "Cleared {removed} artwork files for {}",
                    choice.label()
                ));
            }
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
                let artwork = choices
                    .iter()
                    .filter_map(|choice| {
                        choice.artwork_key().map(|key| {
                            let (count, bytes) =
                                crate::media::ArtworkCache::for_library(&key).stats();
                            (key, count, bytes)
                        })
                    })
                    .collect();
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
                    SettingsAction::CacheSizes {
                        request,
                        entries,
                        artwork,
                    }
                    .into(),
                ));
                let (count, total_bytes) = crate::media::WaveformCache::default().stats();
                let _ = event_tx.blocking_send(
                    crate::app::event::CacheEvent::WaveformCacheStats { count, total_bytes }.into(),
                );
            });
            state.settings_state.cache_task = Some(crate::app::tasks::TaskLease::new(&task));
        }

        SettingsAction::CancelAdventure => {
            state.sources.sonic_tasks.remove("adventure");
            state.adventure_request_id = state.adventure_request_id.wrapping_add(1);
            state.adventure = crate::app::state::AdventureState::default();
            state.popups.set_input_dialog(None);
            state.clear_status();
        }
        SettingsAction::AdventureComplete(tracks) => {
            let tracks = match crate::services::sonic_adventure::unique_artists(tracks) {
                Ok(tracks) => tracks,
                Err(error) => {
                    state.adventure = crate::app::state::AdventureState::default();
                    state.set_error(error.to_string());
                    return Ok(vec![]);
                }
            };
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

        SettingsAction::AdventureGenerated { request_id, result } => {
            if state.adventure_request_id != request_id || !state.adventure.generating {
                return Ok(vec![]);
            }
            let result = result.and_then(|tracks| {
                crate::services::sonic_adventure::unique_artists(tracks).map_err(|error| {
                    crate::app::action::AsyncError {
                        message: error.to_string(),
                    }
                })
            });
            if let Some(launcher) = state.popups.adventure_launcher_mut() {
                launcher.stage = match result {
                    Ok(tracks) if tracks.len() > 2 => {
                        crate::app::state::JourneyStage::Ready(tracks)
                    }
                    Ok(_) => crate::app::state::JourneyStage::Failed(
                        "No route found. Try tracks with more sonic analysis data.".into(),
                    ),
                    Err(error) => crate::app::state::JourneyStage::Failed(error.message),
                };
                launcher.loading = false;
                launcher.item_index = 0;
                state.adventure = Default::default();
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
        SettingsAction::ToggleSplitView => {
            state.desktop_split_view = !state.desktop_split_view;
            config.ui.desktop_split_view = state.desktop_split_view;
            state.set_status(format!(
                "split view: {}",
                if state.desktop_split_view {
                    "on"
                } else {
                    "off"
                }
            ));
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
            // Persist only if pruning changes the saved playlist preferences.
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

fn apply_preference(
    event_tx: &mpsc::Sender<Event>,
    state: &mut AppState,
    config: &mut Config,
    item: crate::app::state::TextampSetting,
) -> Vec<Action> {
    use crate::app::state::TextampSetting;
    let mut follow_ups = Vec::new();
    match item {
        TextampSetting::Presentation(mode) => {
            if !state
                .windowed_presentation
                .can_activate(mode.presentation())
            {
                return follow_ups;
            }
            if let Some(multiple) = mode.gui_multiple_windows() {
                state.gui_multiple_windows = multiple;
                let changed = config.ui.desktop_separate_player != multiple
                    || multiple && (state.desktop_split_view || config.ui.desktop_split_view);
                config.ui.desktop_separate_player = multiple;
                if multiple {
                    state.desktop_split_view = false;
                    config.ui.desktop_split_view = false;
                }
                if changed {
                    save_config_in_background(event_tx, config, "save window arrangement");
                }
            }
            follow_ups.push(
                crate::app::action::NavigationAction::SwitchPresentation(mode.presentation())
                    .into(),
            );
        }
        TextampSetting::LibraryRootVisible(root) => follow_ups.push(
            SettingsAction::SetLibraryRootVisible {
                root,
                visible: !crate::app::sidebar::library_root_visible(state, root),
            }
            .into(),
        ),
        TextampSetting::SinglesTrackLimit => {
            state
                .popups
                .set_input_dialog(Some(crate::app::state::InputDialog {
                    title: "Singles: maximum tracks (1–1000)".into(),
                    input: state.singles_track_limit.to_string().into(),
                    action_type: crate::app::state::InputDialogAction::SinglesTrackLimit,
                }));
        }
        TextampSetting::RemixAllowSameArtist => {
            follow_ups.push(
                SettingsAction::SetRemixAllowSameArtist(!state.remix_allow_same_artist).into(),
            );
        }
        TextampSetting::Theme(name) => {
            state.theme = name;
            state.set_status(format!("Theme: {}", name.display_name()));
            config.ui.theme = name.config_name().to_string();
            save_config_in_background(event_tx, config, "save theme preference");
        }
        TextampSetting::Artwork(mode) => {
            if let Some(key) = state.active_library.as_deref() {
                let presentation = state.artwork_presentation();
                match presentation {
                    crate::app::launch::WindowedMode::Gui => state.artwork.graphical_mode = mode,
                    crate::app::launch::WindowedMode::Text => state.artwork.mode = mode,
                }
                config.set_artwork_mode(key, presentation, mode);
                state.set_status(format!("Artwork rendering: {}", mode.name()));
                save_config_in_background(event_tx, config, "save artwork preference");
            }
        }
        TextampSetting::Transcode => {
            let options = crate::app::preferences::TRANSCODE_BITRATES;
            let current = options
                .iter()
                .position(|&v| v == state.transcode_kbps)
                .unwrap_or(0);
            let next = options[(current + 1) % options.len()];
            follow_ups.push(SettingsAction::SetTranscodeBitrate(next).into());
        }
        TextampSetting::Sidebar(section) => {
            follow_ups.push(SettingsAction::ToggleSectionVisibility(section).into());
        }
        TextampSetting::WebSearch(provider) => {
            state.web_search_provider = provider;
            config.ui.web_search_provider = provider;
            save_config_in_background(event_tx, config, "save browser search preference");
        }
        TextampSetting::ExternalSearch(target) => {
            follow_ups.push(SettingsAction::ToggleExternalSearchService(target).into())
        }
    }
    follow_ups
}
