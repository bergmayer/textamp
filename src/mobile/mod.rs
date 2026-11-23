//! Native phone adapter. The application session remains the sole mutable domain owner.
mod commands;
mod projection;

use crate::{app::runtime::Session, audio::AudioPlayer};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::mpsc;

/// Launch configuration supplied before the native bridge opens a profile.
#[derive(Default, Deserialize)]
pub struct Options {
    /// Use the silent backend for deterministic simulator interaction checks.
    #[serde(default)]
    pub silent: bool,
}

#[derive(Default)]
struct Updates {
    model: Option<Value>,
    player: Option<Value>,
    position: Option<u64>,
    lyrics: Option<Value>,
    error: Option<String>,
    browser_url: Option<String>,
    external_search: Option<crate::services::external_search::Destination>,
    assets: Arc<std::collections::HashMap<String, Arc<Vec<u8>>>>,
}

/// Thread-safe command port to one shared application runtime.
pub struct Client {
    sender: mpsc::UnboundedSender<commands::Command>,
    updates: Arc<Mutex<Updates>>,
    worker: Option<JoinHandle<()>>,
}

impl Client {
    /// Start a session without initializing desktop or terminal rendering.
    /// # Errors
    /// Returns thread creation errors. Runtime failures are delivered by `drain`.
    pub fn start(
        options: Options,
        output_session: impl Fn(bool) -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let updates = Arc::new(Mutex::new(Updates::default()));
        let output = updates.clone();
        let worker = std::thread::Builder::new()
            .name("textamp-mobile".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| {
                        let result =
                            runtime.block_on(run(options, output_session, receiver, &output));
                        runtime.shutdown_timeout(Duration::from_secs(3));
                        result
                    });
                if let Err(error) = result
                    && let Ok(mut updates) = output.lock()
                {
                    updates.error = Some(error.to_string());
                }
            })?;
        Ok(Self {
            sender,
            updates,
            worker: Some(worker),
        })
    }

    /// Decode and enqueue an input without blocking the main UI thread.
    /// # Errors
    /// Rejects invalid input or a closed session.
    pub fn send(&self, json: &str) -> anyhow::Result<()> {
        self.sender.send(serde_json::from_str(json)?)?;
        Ok(())
    }

    /// Take coalesced updates. An absent model leaves native scroll and navigation intact.
    pub fn drain(&self) -> Value {
        let (model, player, position, lyrics, error, browser_url, external_search) = {
            let Ok(mut updates) = self.updates.lock() else {
                return json!({"error": "The application session is unavailable"});
            };
            (
                updates.model.take(),
                updates.player.take(),
                updates.position.take(),
                updates.lyrics.take(),
                updates.error.take(),
                updates.browser_url.take(),
                updates.external_search.take(),
            )
        };
        // Move the existing JSON trees into the response; serializing them into
        // another Value here would allocate the full snapshot again.
        Value::Object(serde_json::Map::from_iter([
            ("model".into(), model.unwrap_or(Value::Null)),
            ("player".into(), player.unwrap_or(Value::Null)),
            ("position".into(), json!(position)),
            ("lyrics".into(), lyrics.unwrap_or(Value::Null)),
            ("error".into(), json!(error)),
            ("browserURL".into(), json!(browser_url)),
            ("externalSearch".into(), json!(external_search.map(|request| json!({
                "label": request.label,
                "url": request.native_url.as_ref().unwrap_or(&request.web_url),
                "webURL": request.web_url,
                "requiresApp": request.target == crate::services::external_search::SearchTarget::AppleMusic
            })))),
        ]))
    }

    /// Return already-cached artwork; network access stays in the shared source boundary.
    pub fn asset(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.updates.lock().ok()?.assets.get(id).cloned()
    }
}
/// Prepare ANSI cells from immutable artwork, without acquiring the live session.
pub fn character_art(bytes: &[u8], mode: &str) -> Option<Value> {
    let mode = crate::app::state::ArtworkMode::from_config(mode);
    let (width, height, cells) = crate::ui::artwork::character_cells(bytes, mode)?;
    Some(json!({"width": width, "height": height, "cells": cells}))
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.sender.send(commands::Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn run(
    options: Options,
    output_session: impl Fn(bool) -> anyhow::Result<()> + Send + 'static,
    mut inputs: mpsc::UnboundedReceiver<commands::Command>,
    output: &Arc<Mutex<Updates>>,
) -> anyhow::Result<()> {
    let mut first_library = Some(std::time::Instant::now());
    let _lock = crate::util::ProcessLock::acquire()?;
    let _logging = crate::util::setup_logging(false);
    let config = crate::config::load_config()?;
    let audio = if options.silent {
        AudioPlayer::new_without_audio()
    } else {
        AudioPlayer::with_deferred_output(output_session)
    };
    let mut session = Session::with_audio(config, audio);
    session.state.visualizers_available = false;
    session.state.playback_waveform_available = true;
    let context = session.task_context();
    context.run(async {
        session.initialize().await?;
        let mut adapter = commands::Adapter::default();
        let mut assets = projection::Assets::default();
        let mut clock = tokio::time::interval(Duration::from_millis(250));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut publish = tokio::time::interval(Duration::from_millis(50));
        publish.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;
        let mut last_position = None;
        loop {
            let mut actions = tokio::select! {
                input = inputs.recv() => {
                    let Some(input) = input else { break; };
                    if matches!(input, commands::Command::Shutdown) { break; }
                    dirty = true;
                    adapter.handle(input, &mut session.state, &session.config)
                }
                event = session.completions.recv() => {
                    let Some(event) = event else { break; };
                    dirty = true;
                    crate::app::runtime::reduce(event, &mut session.state, &session.config, &session.events)
                }
                _ = clock.tick() => {
                    let notices = session.state.notifications.clone();
                    let activity = crate::app::activity::message(&session.state).map(str::to_owned);
                    let mut actions = vec![];
                    for event in session.poll() {
                        if !matches!(event, crate::app::Event::Tick) { dirty = true; }
                        actions.extend(crate::app::runtime::reduce(event, &mut session.state, &session.config, &session.events));
                    }
                    let position = session.state.playback.position_ms;
                    if !dirty && last_position != Some(position) {
                        let lyrics = last_position.and_then(|previous|
                            crate::app::lyrics::playback_update(&session.state, previous)
                        );
                        let mut updates = output.lock().map_err(|_| anyhow::anyhow!("Mobile updates are unavailable"))?;
                        if let Some(lyrics) = lyrics { updates.lyrics = Some(json!(lyrics)); }
                        if let Some(player) = updates.player.as_mut() {
                            player["position"] = json!(position);
                        } else {
                            updates.position = Some(position);
                        }
                        last_position = Some(position);
                    }
                    if !actions.is_empty() || notices != session.state.notifications
                        || activity.as_deref() != crate::app::activity::message(&session.state) { dirty = true; }
                    actions
                }
                _ = publish.tick() => {
                    if dirty {
                        adapter.sync(&session.state);
                        if !projection::library_loading(&session.state) && let Some(started) = first_library.take() {
                            tracing::info!(elapsed_ms = started.elapsed().as_millis(), "Mobile library ready");
                        }
                        let model = projection::model(&session.state, &mut adapter, &mut assets);
                        let player = projection::player(&session.state, &mut assets);
                        if let Ok(mut updates) = output.lock() {
                            updates.model = Some(model);
                            updates.lyrics = None;
                            updates.player = Some(player);
                            updates.assets = assets.bytes.clone();
                            updates.position = None;
                            last_position = Some(session.state.playback.position_ms);
                        }
                        dirty = false;
                    }
                    vec![]
                }
            };
            actions.extend(adapter.take_home_action(&mut session.state));
            for action in actions {
                if let crate::app::Action::System(crate::app::action::SystemAction::OpenBrowser(url)) = action {
                    if let Ok(mut updates) = output.lock() { updates.browser_url = Some(url); }
                    continue;
                }
                if let crate::app::Action::System(crate::app::action::SystemAction::OpenExternalSearch { target, query }) = action {
                    match crate::app::external_search::request(&session.state, target, query) {
                        Ok(request) => {
                            if let Ok(mut updates) = output.lock() {
                                updates.external_search = Some(crate::services::external_search::search_destination(request.target, &request.query));
                            }
                        }
                        Err(error) => session.state.set_status(error.to_string()),
                    }
                    continue;
                }
                // Native navigation presents these requests as sheets and full-screen Settings.
                adapter.observe(&action, &mut session.state);
                if let Err(error) = session.dispatch(action).await && let Ok(mut updates) = output.lock() {
                    updates.error = Some(error.to_string());
                }
            }
        }
        if !session.shutdown().await { anyhow::bail!("Background work timed out at shutdown"); }
        Ok::<_, anyhow::Error>(())
    }).await
}
