use super::*;
use futures::{StreamExt, stream};

pub fn play(state: &mut AppState, audio: &mut AudioPlayer, tx: &mpsc::Sender<Event>) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let Some(mut track) = state.current_track().cloned() else {
        return;
    };
    let result = (|| -> anyhow::Result<()> {
        if track.origin == TrackOrigin::Unavailable {
            let song_id = session.id(&track.rating_key)?;
            track.origin = TrackOrigin::Navidrome {
                source_id: session.source.id.clone(),
                song_id,
            };
            if let Some(current) = state.current_track_mut() {
                current.origin = track.origin.clone();
            }
        }
        let TrackOrigin::Navidrome { source_id, song_id } = &track.origin else {
            anyhow::bail!("Track is not from the active Navidrome account");
        };
        anyhow::ensure!(
            source_id == &session.source.id,
            "Track belongs to a different account"
        );
        state.sources.media = crate::app::sources::MediaPreparation::Idle;
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        state.playback.position_ms = 0;
        state.playback.duration_ms = track.duration_ms();
        state.playback.scrobble = crate::app::state::ScrobbleState::Pending;
        state.playback.status = PlayStatus::Buffering;
        state.waveform = crate::app::state::WaveformState {
            track_key: Some(track.rating_key.clone()),
            ..Default::default()
        };
        state.spectrogram = crate::app::state::SpectrogramState {
            track_key: Some(track.rating_key.clone()),
            ..Default::default()
        };
        let url = session.client.stream_url(song_id, state.transcode_kbps)?;
        audio.play_url_with_headers(
            url.as_str(),
            Default::default(),
            None,
            helpers::audio_event_adapter(tx),
            session.client.http(),
        )?;
        state.playback.request_id = audio.playback_id();
        artwork(state, tx, &session);
        let session = session.clone();
        let id = song_id.clone();
        spawn(state, tx, "now-playing", async move {
            if let Err(error) = session
                .client
                .call("scrobble", &[("id", id), ("submission", "false".into())])
                .await
            {
                tracing::warn!("Navidrome now-playing report: {error}");
            }
            Ok(vec![])
        });
        Ok(())
    })();
    if let Err(error) = result {
        audio.stop();
        state.playback.request_id = audio.playback_id();
        state.playback.status = PlayStatus::Stopped;
        state.set_error(format!("Playback: {error:#}"));
    }
}

pub fn scrobble(state: &mut AppState, tx: &mpsc::Sender<Event>, track: &Track) {
    if state.downloads.is_offline() {
        return;
    }
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let TrackOrigin::Navidrome { source_id, song_id } = &track.origin else {
        return;
    };
    if source_id != &session.source.id {
        return;
    }
    let id = song_id.clone();
    // Do not replace a previous track's pending submission on fast navigation.
    let tx = tx.clone();
    let generation = state.library_generation;
    crate::app::tasks::spawn(async move {
        if let Err(error) = session
            .client
            .call("scrobble", &[("id", id), ("submission", "true".into())])
            .await
        {
            let _ = tx
                .send(Event::for_library(
                    generation,
                    Event::Effect(
                        SystemAction::SetStatus(format!("Play history was not saved: {error}"))
                            .into(),
                    ),
                ))
                .await;
        }
    });
}

pub fn artwork(state: &mut AppState, tx: &mpsc::Sender<Event>, session: &Session) {
    let thumb = state
        .current_track()
        .and_then(|track| {
            track
                .thumb
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(track.parent_thumb.as_deref().filter(|s| !s.is_empty()))
                .or_else(|| {
                    track
                        .parent_rating_key
                        .as_ref()
                        .and_then(|key| {
                            state
                                .library
                                .albums
                                .iter()
                                .find(|album| &album.rating_key == key)
                        })
                        .and_then(|album| album.thumb.as_deref())
                        .filter(|s| !s.is_empty())
                })
                .or(track.grandparent_thumb.as_deref().filter(|s| !s.is_empty()))
        })
        .map(str::to_owned);
    let Some(thumb_path) = thumb else {
        state.artwork.fetch = crate::app::state::ArtworkFetch::Idle;
        state.artwork.current_thumb = None;
        state.artwork.current_data = None;
        return;
    };
    if state.artwork.current_thumb.as_ref() == Some(&thumb_path)
        || state.artwork.fetch.pending_thumb() == Some(thumb_path.as_str())
    {
        return;
    }
    state.artwork.fetch = crate::app::state::ArtworkFetch::Loading {
        thumb: thumb_path.clone(),
    };
    state.artwork.current_data = None;
    let generation = state.artwork.grid_generation;
    let library_key = state.active_library.clone().unwrap_or_else(|| {
        format!(
            "navidrome:{}:{}",
            session.source.id,
            session.client.folder.as_deref().unwrap_or("all")
        )
    });
    let session = session.clone();
    spawn(state, tx, "artwork", async move {
        let result = cover(&session, &thumb_path, &library_key).await;
        Ok(vec![event(match result {
            Ok(data) => ArtworkEvent::ArtworkLoaded {
                generation,
                thumb_path,
                data,
            },
            Err(error) => {
                tracing::warn!("Navidrome artwork: {error:#}");
                ArtworkEvent::ArtworkFailed {
                    generation,
                    thumb_path,
                }
            }
        })])
    });
}
pub(in crate::app::sources::navidrome) async fn cover(
    session: &Session,
    key: &str,
    library_key: &str,
) -> anyhow::Result<Vec<u8>> {
    let cache = crate::media::ArtworkCache::for_library(library_key);
    let cache_key = key.to_owned();
    let read = cache.clone();
    if let Some(data) =
        crate::app::tasks::spawn_blocking(move || read.load(&cache_key, 32 * 86400)).await?
    {
        return Ok(data);
    }
    let data = download_cover(session, key).await?;
    let cache_key = key.to_owned();
    let cached = data.clone();
    crate::app::tasks::spawn_blocking(move || {
        if !cache.save(&cache_key, &cached) {
            tracing::warn!("Artwork cache write failed or was invalidated");
        }
    });
    Ok(data)
}
pub(in crate::app::sources::navidrome) async fn download_cover(
    session: &Session,
    key: &str,
) -> anyhow::Result<Vec<u8>> {
    let url = session.client.url(
        "getCoverArt",
        &[("id", session.id(key)?), ("size", "600".into())],
    )?;
    session.client.bytes(url, 8 * 1024 * 1024).await
}
async fn mosaic_cover(session: &Session, spec: &str, library_key: &str) -> anyhow::Result<Vec<u8>> {
    static COMPOSITIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let cache = crate::media::ArtworkCache::for_library(library_key);
    let cache_key = format!("mosaic:{spec}");
    let read = cache.clone();
    let read_key = cache_key.clone();
    if let Some(data) =
        crate::app::tasks::spawn_blocking(move || read.load(&read_key, 32 * 86400)).await?
    {
        return Ok(data);
    }
    let thumbs: Vec<String> = serde_json::from_str(spec)?;
    let covers = stream::iter(thumbs.into_iter().take(9))
        .map(|thumb| async move { cover(session, &thumb, library_key).await.ok() })
        .buffered(3)
        .filter_map(|cover| async move { cover })
        .collect()
        .await;
    let permit = COMPOSITIONS.acquire().await?;
    crate::app::tasks::spawn_blocking(move || {
        let _permit = permit;
        let data = crate::media::mosaic::compose(covers)?;
        let _ = cache.save(&cache_key, &data);
        Ok(data)
    })
    .await?
}

pub fn art_batch(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
    batch: Vec<(String, String)>,
) {
    let mosaics = crate::app::artwork_mosaic::requests(state);
    state
        .artwork
        .request_grid_art(batch.into_iter().chain(mosaics));
    if matches!(
        state.artwork.navigation,
        crate::app::state::ArtworkNavigation::Moving { .. }
    ) || state.sources.nav_tasks.contains_key("art-grid")
    {
        return;
    }
    state.artwork.grid_pending.clear();
    let batch: Vec<_> = state
        .artwork
        .grid_requested
        .drain(..)
        .filter(|(key, _)| !state.artwork.grid_cache.contains_key(key))
        .collect();
    if batch.is_empty() {
        return;
    }
    state
        .artwork
        .grid_pending
        .extend(batch.iter().map(|(k, _)| k.clone()));
    let generation = state.artwork.grid_generation;
    let library_key = state.active_library.clone().unwrap_or_else(|| {
        format!(
            "navidrome:{}:{}",
            session.source.id,
            session.client.folder.as_deref().unwrap_or("all")
        )
    });
    let session = session.clone();
    let progress = tx.clone();
    spawn_scoped(
        state,
        tx,
        "art-grid",
        move |library_generation, request_id| async move {
            let mut actions = stream::iter(batch)
                .map(|(key, thumb)| {
                    let session = session.clone();
                    let library_key = library_key.clone();
                    async move {
                        let result = if let Some(spec) = thumb.strip_prefix("mosaic:") {
                            mosaic_cover(&session, spec, &library_key).await
                        } else {
                            cover(&session, &thumb, &library_key).await
                        };
                        event(match result {
                            Ok(data) => ArtworkEvent::AlbumArtLoaded {
                                generation,
                                key,
                                data,
                            },
                            Err(_) => ArtworkEvent::AlbumArtFailed { generation, key },
                        })
                    }
                })
                .buffer_unordered(4);
            while let Some(action) = actions.next().await {
                if progress
                    .send(Event::Effect(
                        NavAction::Progress {
                            generation: library_generation,
                            request_id,
                            slot: "art-grid",
                            actions: vec![action],
                        }
                        .into(),
                    ))
                    .await
                    .is_err()
                {
                    return Ok(vec![]);
                }
            }
            Ok(vec![])
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn artwork_requested_during_a_batch_starts_after_its_owned_completion() {
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let mut state = AppState::new();
        state.sources.active = ActiveSource::Navidrome(Box::new(session.clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        spawn(&mut state, &tx, "art-grid", std::future::pending());
        let request_id = state.sources.nav_tasks["art-grid"].0;
        let generation = state.library_generation;
        art_batch(
            &mut state,
            &tx,
            &session,
            vec![("visible-album".into(), "cover".into())],
        );
        assert!(!state.artwork.grid_pending.contains("visible-album"));
        assert_eq!(state.artwork.grid_requested.len(), 1);
        super::super::super::dispatch(
            NavAction::Completed {
                generation,
                request_id,
                slot: "art-grid",
                result: Ok(vec![]),
            },
            &mut state,
            &mut AudioPlayer::new_without_audio(),
            &mut Config::default(),
            &tx,
        )
        .await
        .unwrap();
        assert!(state.artwork.grid_pending.contains("visible-album"));
        assert!(state.artwork.grid_requested.is_empty());
        assert_ne!(state.sources.nav_tasks["art-grid"].0, request_id);
    }

    #[test]
    fn artwork_backlog_is_bounded_deduplicated_and_cleared_with_its_cache() {
        let mut artwork = crate::app::state::ArtworkState::default();
        artwork.grid_pending.insert("pending".into());
        artwork.insert_grid_art("cached".into(), vec![1]);
        artwork.request_grid_art((0..130).map(|i| (i.to_string(), "cover".into())));
        artwork.request_grid_art([
            ("pending".into(), "cover".into()),
            ("cached".into(), "cover".into()),
            ("129".into(), "cover".into()),
        ]);
        assert_eq!(artwork.grid_requested.len(), 100);
        assert_eq!(artwork.grid_requested.front().unwrap().0, "30");
        assert_eq!(artwork.grid_requested.back().unwrap().0, "129");
        artwork.clear_grid_art();
        assert!(artwork.grid_requested.is_empty());
    }

    #[tokio::test]
    async fn cached_radio_tracks_recover_only_the_active_accounts_playback_identity() {
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        for (key, valid) in [
            (session.key("song"), true),
            ("navidrome:another:song".into(), false),
        ] {
            let mut state = AppState::new();
            state.sources.active = ActiveSource::Navidrome(Box::new(session.clone()));
            state.playback_mode = crate::app::state::PlaybackMode::Radio;
            state.radio.tracks = vec![Track {
                rating_key: key,
                ..Default::default()
            }];
            state.radio.track_index = Some(0);
            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            play(&mut state, &mut AudioPlayer::new_without_audio(), &tx);
            if valid {
                assert_eq!(
                    state.current_track().unwrap().origin,
                    TrackOrigin::Navidrome {
                        source_id: "fixture".into(),
                        song_id: "song".into()
                    }
                );
                assert!(
                    !state
                        .notifications
                        .last_error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("different account")
                );
            } else {
                assert_eq!(
                    state.current_track().unwrap().origin,
                    TrackOrigin::Unavailable
                );
                assert!(
                    state
                        .notifications
                        .last_error
                        .as_deref()
                        .unwrap()
                        .contains("different account")
                );
            }
        }
    }
}

/// Warm only the next waveform, after foreground playback has finished its transfer.
pub fn prefetch_waveform(state: &mut AppState, tx: &mpsc::Sender<Event>) {
    let next = state.next_track().cloned();
    let key = next.as_ref().map(|track| track.rating_key.clone());
    if key.as_deref() == state.sources.next_waveform.key() {
        return;
    }
    state.sources.nav_tasks.remove("next-waveform");
    state.sources.next_waveform = crate::app::sources::NextWaveform::Idle;
    let Some(track) = next else {
        return;
    };
    let saved = state
        .downloads
        .store
        .find(&track)
        .map(|(id, _)| state.downloads.store.root.join(id).join("audio"));
    if state.downloads.is_offline() && saved.is_none() {
        return;
    }
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let key = track.rating_key.clone();
    state.sources.next_waveform = crate::app::sources::NextWaveform::Loading(key.clone());
    let scope = state.media_cache_scope();
    spawn(state, tx, "next-waveform", async move {
        let result = async {
            let read_scope = scope.clone();
            let read_key = key.clone();
            if crate::app::tasks::spawn_blocking(move || {
                crate::media::WaveformCache::default().load_scoped(&read_scope, &read_key)
            })
            .await?
            .is_some()
            {
                return Ok(());
            }
            let bytes = if let Some(path) = saved {
                use tokio::io::AsyncReadExt;
                let file = tokio::fs::File::open(path).await?;
                let mut bytes = Vec::new();
                file.take(128 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                anyhow::ensure!(
                    bytes.len() <= 128 * 1024 * 1024,
                    "Audio exceeds analysis limit"
                );
                bytes
            } else {
                let id = session.id(&key)?;
                let url = session.client.stream_url(&id, 0)?;
                session.client.bytes(url, 128 * 1024 * 1024).await?
            };
            let analysis_key = key.clone();
            crate::app::tasks::analyze(move || -> anyhow::Result<()> {
                let waveform = crate::media::generate_waveform(
                    analysis_key,
                    track.duration_ms(),
                    std::sync::Arc::from(bytes),
                )?;
                anyhow::ensure!(
                    crate::media::WaveformCache::default().save_scoped(&scope, &waveform),
                    "Could not save next waveform"
                );
                Ok(())
            })
            .await??;
            Ok::<_, anyhow::Error>(())
        }
        .await
        .map_err(|error| format!("{error:#}"));
        Ok(vec![NavAction::WaveformPrefetched { key, result }.into()])
    });
}
