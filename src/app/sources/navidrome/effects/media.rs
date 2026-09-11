use super::*;
use futures::{stream, StreamExt};

pub fn play(state: &mut AppState, audio: &mut AudioPlayer, tx: &mpsc::Sender<Event>) {
    let Some(session) = state.sources.active.navidrome().cloned() else {
        return;
    };
    let Some(track) = state.current_track().cloned() else {
        return;
    };
    let result = (|| -> anyhow::Result<()> {
        let TrackOrigin::Navidrome { source_id, song_id } = &track.origin else {
            anyhow::bail!("Track is not from the active Navidrome account");
        };
        anyhow::ensure!(
            source_id == &session.source.id,
            "Track belongs to a different account"
        );
        state.sources.preparing = None;
        state.sources.prepared = None;
        state.playback.preparation_id = state.playback.preparation_id.wrapping_add(1);
        state.playback.position_ms = 0;
        state.playback.duration_ms = track.duration_ms();
        state.playback.scrobble_reported = false;
        state.playback.status = PlayStatus::Buffering;
        state.waveform = crate::app::state::WaveformState {
            track_key: Some(track.rating_key.clone()),
            ..Default::default()
        };
        state.spectrogram = crate::app::state::SpectrogramState {
            track_key: Some(track.rating_key.clone()),
            ..Default::default()
        };
        let mut params = vec![("id", song_id.clone())];
        if state.transcode_kbps > 0 {
            params.push(("maxBitRate", state.transcode_kbps.to_string()));
        } else {
            params.push(("format", "raw".into()));
        }
        let url = session.client.url("stream", &params)?;
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
        state.playback.status = PlayStatus::Stopped;
        state.set_error(format!("Playback: {error:#}"));
    }
}

pub fn scrobble(state: &mut AppState, tx: &mpsc::Sender<Event>, track: &Track) {
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
        .and_then(|t| t.best_thumb())
        .map(str::to_owned);
    let Some(thumb_path) = thumb else {
        state.artwork.current_thumb = None;
        state.artwork.current_data = None;
        return;
    };
    if state.artwork.current_thumb.as_ref() == Some(&thumb_path)
        || state.artwork.pending_thumb.as_ref() == Some(&thumb_path)
    {
        return;
    }
    state.artwork.loading = true;
    state.artwork.pending_thumb = Some(thumb_path.clone());
    state.artwork.current_data = None;
    let generation = state.artwork.grid_generation;
    let session = session.clone();
    spawn(state, tx, "artwork", async move {
        let result = cover(&session, &thumb_path).await;
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
async fn cover(session: &Session, key: &str) -> anyhow::Result<Vec<u8>> {
    let url = session.client.url(
        "getCoverArt",
        &[("id", session.id(key)?), ("size", "600".into())],
    )?;
    session.client.bytes(url, 8 * 1024 * 1024).await
}
pub fn art_batch(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    session: &Session,
    batch: Vec<(String, String)>,
) {
    if state.artwork.suppress_loads {
        return;
    }
    // One viewport batch at a time; abandoned batches must not leave pending marks.
    state.artwork.grid_pending.clear();
    let batch: Vec<_> = batch
        .into_iter()
        .filter(|(k, _)| !state.artwork.grid_cache.contains_key(k))
        .take(100)
        .collect();
    if batch.is_empty() {
        return;
    }
    state
        .artwork
        .grid_pending
        .extend(batch.iter().map(|(k, _)| k.clone()));
    let generation = state.artwork.grid_generation;
    let session = session.clone();
    spawn(state, tx, "art-grid", async move {
        let actions = stream::iter(batch)
            .map(|(key, thumb)| {
                let session = session.clone();
                async move {
                    event(match cover(&session, &thumb).await {
                        Ok(data) => ArtworkEvent::AlbumArtLoaded {
                            generation,
                            key,
                            data,
                        },
                        Err(_) => ArtworkEvent::AlbumArtFailed { generation, key },
                    })
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        Ok(actions)
    });
}
