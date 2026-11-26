//! Window-owned workflow state, separate from the library and playback domain.
use super::surface::{self, Surface};
use crate::app::{self, state::*};
use std::collections::{BTreeMap, HashMap};

use crate::app::workflow::Context;

struct Session {
    base: String,
    model: Surface,
    context: Context,
    sort_anchor: Option<(BrowseCategory, usize, String)>,
}
#[derive(Default)]
pub(super) struct Sessions {
    generation: u64,
    next_instance: u64,
    entries: BTreeMap<String, Session>,
    tasks: HashMap<(&'static str, u64), String>,
    leases: HashMap<(&'static str, u64), app::tasks::TaskLease>,
}
impl Sessions {
    pub fn sync(&mut self, model: Option<&Surface>, state: &app::AppState, multi: bool) {
        if !multi {
            self.entries.clear();
            self.tasks.clear();
            self.leases.clear();
        }
        if self.generation != state.library_generation {
            for entry in self.entries.values_mut() {
                entry.model.stale = true;
                if let Some(reader) = entry.context.popups.artist_bio_mut()
                    && let Some(task) = reader.task.take()
                {
                    task.cancel();
                }
            }
            self.tasks.clear();
            self.leases.clear();
            self.generation = state.library_generation;
        }
        if multi && let Some(model) = model {
            let existing = if model.palette {
                self.entries
                    .iter()
                    .find(|(_, entry)| entry.model.palette)
                    .map(|(id, _)| id.clone())
            } else {
                self.resolve(&model.id)
            };
            let id = existing.unwrap_or_else(|| {
                self.next_instance = self.next_instance.wrapping_add(1);
                format!("{}:window:{}", model.id, self.next_instance)
            });
            let mut context = Context::capture(state);
            if let Some(previous) = self.entries.get(&id)
                && let (Some(old), Some(new)) = (
                    previous
                        .context
                        .popups
                        .artist_bio()
                        .and_then(|reader| reader.task.as_ref()),
                    context
                        .popups
                        .artist_bio()
                        .and_then(|reader| reader.task.as_ref()),
                )
                && old.same_task(new)
            {
                context.bio_request = previous.context.bio_request;
            }
            let mut visible = model.clone();
            visible.id = id.clone();
            self.entries.insert(
                id,
                Session {
                    base: model.id.clone(),
                    model: visible,
                    context,
                    sort_anchor: state.popups.sort().and_then(|popup| {
                        super::browser::columns(state)
                            .get(popup.column_idx)
                            .map(|column| {
                                (
                                    state.browse_category,
                                    popup.column_idx,
                                    column.identity.clone(),
                                )
                            })
                    }),
                },
            );
        }
    }
    pub fn resolve(&self, base: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|(id, session)| id.as_str() == base || session.base == base)
            .map(|(id, _)| id.clone())
    }
    fn is_active(&self, id: &str, state: &app::AppState) -> bool {
        self.entries.get(id).is_some_and(|entry| {
            surface::project(state).is_some_and(|model| model.id == entry.base)
        })
    }
    pub fn refresh_palettes(&mut self, state: &mut app::AppState) {
        let original = state.palette.clone();
        for entry in self
            .entries
            .values_mut()
            .filter(|entry| entry.model.palette)
        {
            state.palette = entry.context.palette.clone();
            state.palette.open = true;
            if let Some(mut model) = surface::project(state) {
                entry.base = model.id.clone();
                model.id = entry.model.id.clone();
                entry.model = model;
                entry.context = Context::capture(state);
            }
        }
        state.palette = original;
    }
    pub fn models(&self) -> Vec<Surface> {
        self.entries
            .values()
            .map(|entry| entry.model.clone())
            .collect()
    }
    pub fn dismiss_playlist_picker(&mut self, id: &str) {
        if self.entries.get(id).is_some_and(|entry| {
            entry.model.palette_path == [app::sources::navidrome::playlist_edit::PICKER]
        }) {
            self.entries.remove(id);
        }
    }
    pub fn palette_item(&self, id: &str, revision: u64, index: usize) -> Option<surface::Item> {
        self.entries
            .get(id)
            .filter(|entry| {
                entry.model.palette && !entry.model.stale && entry.model.revision == revision
            })
            .and_then(|entry| entry.model.rows.get(index))
            .cloned()
    }
    pub fn leave_for_browse(&self, state: &mut app::AppState) {
        state.palette.close();
        if state.popups.library_dialog().is_none() {
            state.popups.close_all();
        }
        state.view = View::Browse;
        state.previous_view = None;
    }
    pub fn activate(&self, id: &str, revision: Option<u64>, state: &mut app::AppState) -> bool {
        let Some(entry) = self.entries.get(id) else {
            return false;
        };
        if entry.model.stale
            || self.generation != state.library_generation
            || revision.is_some_and(|revision| entry.model.revision != revision)
        {
            return false;
        }
        if entry
            .sort_anchor
            .as_ref()
            .is_some_and(|(category, index, identity)| {
                state.browse_category != *category
                    || super::browser::columns(state)
                        .get(*index)
                        .is_none_or(|column| &column.identity != identity)
            })
        {
            return false;
        }
        let request_counter = state.artist_bio_request_id;
        entry.context.restore(state);
        // New requests use a monotonic counter even when an older reader is focused.
        state.artist_bio_request_id = request_counter.max(state.artist_bio_request_id);
        true
    }
    pub fn close(
        &mut self,
        id: &str,
        state: &mut app::AppState,
        config: &crate::config::Config,
    ) -> Vec<app::Action> {
        let active = self.is_active(id, state);
        let closed = self.entries.remove(id);
        if !active && state.palette.open {
            let palette = state.palette.clone();
            state.palette.open = false;
            if closed.as_ref().is_some_and(|closed| {
                surface::project(state).is_some_and(|model| model.id == closed.base)
            }) {
                self.leave_for_browse(state);
            }
            state.palette = palette;
        }
        for ((slot, request), owner) in &self.tasks {
            if owner == id
                && state
                    .sources
                    .nav_tasks
                    .get(slot)
                    .is_some_and(|(current, _)| current == request)
            {
                state.sources.nav_tasks.remove(slot);
                if *slot == "station-children" {
                    state.station_nav.loading = false;
                    state.station_navigation_generation =
                        state.station_navigation_generation.wrapping_add(1);
                }
            }
        }
        self.leases
            .retain(|key, _| self.tasks.get(key).is_none_or(|owner| owner != id));
        self.tasks.retain(|_, owner| owner != id);
        if active {
            surface::cancel(state, config)
        } else {
            vec![]
        }
    }
    fn bind_tasks(
        &mut self,
        state: &app::AppState,
        owner: Option<&str>,
        previous: &std::collections::HashSet<(&'static str, u64)>,
    ) {
        let Some(owner) = owner else { return };
        for (slot, (request, lease)) in &state.sources.nav_tasks {
            if !matches!(
                *slot,
                "lyrics" | "similar" | "related" | "adventure" | "station-children"
            ) || previous.contains(&(slot, *request))
            {
                continue;
            }
            let superseded: Vec<_> = self
                .tasks
                .iter()
                .filter(|((old_slot, old_request), old_owner)| {
                    old_slot == slot && *old_request != *request && old_owner.as_str() == owner
                })
                .map(|(key, _)| *key)
                .collect();
            for key in superseded {
                self.tasks.remove(&key);
                self.leases.remove(&key);
            }
            self.leases.insert((slot, *request), lease.clone());
            self.tasks
                .entry((slot, *request))
                .or_insert_with(|| owner.to_owned());
        }
    }
    fn completion_key(&self, action: &app::Action) -> Option<(&'static str, u64)> {
        use app::sources::{SourceAction, navidrome::NavAction};
        if let app::Action::Source(SourceAction::Navidrome(action)) = action
            && let NavAction::Completed {
                generation,
                slot,
                request_id,
                ..
            } = action.as_ref()
            && *generation == self.generation
        {
            return Some((*slot, *request_id));
        }
        None
    }
    fn completion_owner(&self, action: &app::Action) -> Option<String> {
        self.completion_key(action)
            .and_then(|key| self.tasks.get(&key).cloned())
    }
    pub fn event(
        &mut self,
        event: app::Event,
        state: &mut app::AppState,
        events: &tokio::sync::mpsc::Sender<app::Event>,
    ) -> Vec<app::Action> {
        let owner = match &event {
            app::Event::LibraryResult { generation, event }
                if *generation == state.library_generation =>
            {
                self.bio_owner(event)
            }
            _ => self.bio_owner(&event),
        };
        let Some(owner) = owner else {
            return app::dispatch::handle_core_event(event, state, events);
        };
        let previous = (!self.is_active(&owner, state)).then(|| Context::capture(state));
        let request_counter = state.artist_bio_request_id;
        if let Some(entry) = self.entries.get(&owner) {
            entry.context.restore(state);
        }
        let actions = app::dispatch::handle_core_event(event, state, events);
        let model =
            surface::project(state).filter(|model| model.palette || !super::workspace::owns(state));
        self.sync(model.as_ref(), state, true);
        if let Some(previous) = previous {
            previous.restore(state);
        }
        state.artist_bio_request_id = state.artist_bio_request_id.max(request_counter);
        actions
    }
    fn bio_owner(&self, event: &app::Event) -> Option<String> {
        if let app::Event::Ui(app::event::UiEvent::ArtistBioLoaded {
            generation,
            request_id,
            ..
        }) = event
        {
            self.entries
                .iter()
                .find(|(_, entry)| {
                    !entry.model.stale
                        && *generation == self.generation
                        && entry.context.popups.artist_bio().is_some()
                        && entry.context.bio_request == *request_id
                })
                .map(|(id, _)| id.clone())
        } else {
            None
        }
    }
    pub async fn dispatch(
        &mut self,
        action: app::Action,
        state: &mut app::AppState,
        audio: &mut crate::audio::AudioPlayer,
        config: &mut crate::config::Config,
        events: &tokio::sync::mpsc::Sender<app::Event>,
    ) -> anyhow::Result<()> {
        if !config.ui.desktop_separate_player {
            return app::dispatch::dispatch_action(action, state, audio, config, events).await;
        }
        let owner = self.completion_owner(&action);
        let previous = owner
            .as_ref()
            .filter(|owner| !self.is_active(owner, state))
            .map(|_| Context::capture(state));
        let completed = owner.as_ref().and_then(|_| self.completion_key(&action));
        let previous_slot = completed.and_then(|(slot, _)| state.sources.nav_tasks.remove(slot));
        if let Some(key) = completed
            && let Some(lease) = self.leases.get(&key)
        {
            state
                .sources
                .nav_tasks
                .insert(key.0, (key.1, lease.clone()));
        }
        if let Some(owner) = owner.as_ref() {
            self.activate(owner, None, state);
        }
        let before = state
            .sources
            .nav_tasks
            .iter()
            .map(|(slot, (request, _))| (*slot, *request))
            .collect();
        let result = app::dispatch::dispatch_action(action, state, audio, config, events).await;
        let model =
            surface::project(state).filter(|model| model.palette || !super::workspace::owns(state));
        self.sync(model.as_ref(), state, true);
        let owner = owner.or_else(|| model.as_ref().and_then(|model| self.resolve(&model.id)));
        self.bind_tasks(state, owner.as_deref(), &before);
        if let Some(key) = completed {
            self.tasks.remove(&key);
            self.leases.remove(&key);
            if let Some((request, lease)) = previous_slot
                && request != key.1
            {
                state.sources.nav_tasks.insert(key.0, (request, lease));
            }
        }
        if let Some(previous) = previous {
            previous.restore(state);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::AppState;
    fn save(sessions: &mut Sessions, state: &AppState) -> Surface {
        let model = surface::project(state).unwrap();
        sessions.sync(Some(&model), state, true);
        sessions
            .entries
            .get(&sessions.resolve(&model.id).unwrap())
            .unwrap()
            .model
            .clone()
    }
    fn text(state: &mut AppState, request: u64, title: &str) {
        state.palette.close();
        state.view = View::Browse;
        state.popups.set_text(Some(TextPopup {
            album_notes: false,
            lyrics_query: None,
            title: title.into(),
            text: "Loading…".into(),
            scroll: 0,
            request_id: request,
        }));
    }
    #[test]
    fn workflows_persist_independently_and_only_explicit_close_removes_one() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        text(&mut state, 1, "First");
        let first = save(&mut sessions, &state);
        text(&mut state, 2, "Second");
        let second = save(&mut sessions, &state);
        state.popups.close_all();
        state.view = View::Browse;
        sessions.sync(None, &state, true);
        assert_eq!(sessions.models().len(), 2);
        assert!(sessions.activate(&first.id, Some(first.revision), &mut state));
        assert_eq!(state.popups.text().unwrap().title, "First");
        sessions.close(&second.id, &mut state, &crate::config::Config::default());
        assert_eq!(state.popups.text().unwrap().title, "First");
        assert_eq!(sessions.models().len(), 1);
        assert!(state.queue.tracks.is_empty());
    }

    #[test]
    fn choosing_a_playlist_dismisses_only_the_picker_window() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        text(&mut state, 1, "Lyrics");
        let reader = save(&mut sessions, &state);
        app::command_palette::open_with_query(&mut state, "");
        state.palette.path = vec![app::sources::navidrome::playlist_edit::PICKER.into()];
        let picker = save(&mut sessions, &state);

        state.palette.close();
        sessions.dismiss_playlist_picker(&picker.id);
        sessions.refresh_palettes(&mut state);
        let remaining = sessions.models();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, reader.id);
    }

    #[test]
    fn closing_a_reader_under_the_palette_does_not_reopen_it_on_palette_dismissal() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        state.view = View::Help;
        let help = save(&mut sessions, &state);
        app::command_palette::open_with_query(&mut state, "");
        let palette = save(&mut sessions, &state);
        sessions.close(&help.id, &mut state, &crate::config::Config::default());
        save(&mut sessions, &state);
        sessions.close(&palette.id, &mut state, &crate::config::Config::default());
        assert!(surface::project(&state).is_none());
        assert!(sessions.models().is_empty());
    }
    #[test]
    fn closing_and_reopening_a_workflow_rejects_its_old_window_identity() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        text(&mut state, 1, "Lyrics");
        let first = save(&mut sessions, &state);
        sessions.close(&first.id, &mut state, &crate::config::Config::default());
        text(&mut state, 1, "Lyrics");
        let reopened = save(&mut sessions, &state);
        assert_ne!(first.id, reopened.id);
        assert!(!sessions.activate(&first.id, None, &mut state));
        assert!(sessions.activate(&reopened.id, None, &mut state));
    }
    #[test]
    fn palette_has_one_window_identity_when_the_station_context_changes() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        app::command_palette::open_with_query(&mut state, "");
        let first = save(&mut sessions, &state);
        state.station_navigation_generation += 1;
        let second = save(&mut sessions, &state);
        assert_eq!(first.id, second.id);
        assert_eq!(sessions.models().len(), 1);
    }
    #[test]
    fn palette_stays_available_after_navigation_and_source_changes_reject_old_controls() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        app::command_palette::open_with_query(&mut state, "volume");
        let palette = save(&mut sessions, &state);
        state.palette.close();
        state.view = View::Browse;
        sessions.sync(None, &state, true);
        assert!(sessions.activate(&palette.id, None, &mut state));
        assert!(state.palette.open);
        assert_eq!(state.palette.query, "volume");
        state.library_generation += 1;
        state.palette.close();
        sessions.sync(None, &state, true);
        assert_eq!(sessions.models().len(), 1);
        assert!(sessions.models()[0].stale);
        assert!(!sessions.activate(&palette.id, None, &mut state));
        sessions.sync(None, &state, false);
        assert!(sessions.models().is_empty());
    }
    #[test]
    fn biography_completion_updates_its_reader_without_replacing_search() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        state.artist_bio_request_id = 17;
        state.popups.set_artist_bio(Some(ArtistBioPopup {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
            document: Default::default(),
            scroll: 0,
            google_focused: false,
            image_index: 0,
            task: None,
        }));
        let reader = save(&mut sessions, &state);
        state.popups.close_all();
        state.popups.set_search_active(true);
        state.search.query = "another artist".into();
        let search = save(&mut sessions, &state);
        let (events, _) = tokio::sync::mpsc::channel(4);
        let actions = sessions.event(
            app::event::UiEvent::ArtistBioLoaded {
                generation: state.library_generation,
                request_id: 17,
                result: Ok(crate::services::biography::Biography {
                    text: "Biography completed".into(),
                    ..Default::default()
                }),
            }
            .into(),
            &mut state,
            &events,
        );
        assert!(actions.is_empty());
        assert!(sessions.is_active(&search.id, &state));
        assert!(
            sessions
                .models()
                .iter()
                .find(|model| model.id == reader.id)
                .unwrap()
                .text
                .contains("Biography completed")
        );
        assert_eq!(state.search.query, "another artist");
    }
    #[tokio::test]
    async fn focusing_a_pending_biography_keeps_its_request_identity_and_counter_monotonic() {
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        state.artist_bio_request_id = 17;
        let task = tokio::spawn(std::future::pending::<()>());
        state.popups.set_artist_bio(Some(ArtistBioPopup {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
            document: Default::default(),
            scroll: 0,
            google_focused: false,
            image_index: 0,
            task: Some(app::tasks::TaskLease::new(&task)),
        }));
        let reader = save(&mut sessions, &state);
        state.artist_bio_request_id = 30;
        assert!(sessions.activate(&reader.id, None, &mut state));
        save(&mut sessions, &state);
        let (events, _) = tokio::sync::mpsc::channel(4);
        sessions.event(
            app::event::UiEvent::ArtistBioLoaded {
                generation: state.library_generation,
                request_id: 17,
                result: Ok(crate::services::biography::Biography {
                    text: "Finished".into(),
                    ..Default::default()
                }),
            }
            .into(),
            &mut state,
            &events,
        );
        assert_eq!(state.popups.artist_bio().unwrap().document.text, "Finished");
        assert_eq!(state.artist_bio_request_id, 30);
        task.abort();
    }
    #[tokio::test]
    async fn concurrent_reader_requests_keep_their_leases_and_completion_identity() {
        use app::{sources::navidrome::NavAction, tasks::TaskLease};
        let mut state = AppState::new();
        let mut sessions = Sessions::default();
        let mut config = crate::config::Config::default();
        config.ui.desktop_separate_player = true;
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (events, _) = tokio::sync::mpsc::channel(4);
        text(&mut state, 1, "First");
        let first = save(&mut sessions, &state);
        let one = tokio::spawn(std::future::pending::<()>());
        state
            .sources
            .nav_tasks
            .insert("lyrics", (1, TaskLease::new(&one)));
        sessions.bind_tasks(&state, Some(&first.id), &Default::default());
        text(&mut state, 2, "Second");
        let second = save(&mut sessions, &state);
        let two = tokio::spawn(std::future::pending::<()>());
        state
            .sources
            .nav_tasks
            .insert("lyrics", (2, TaskLease::new(&two)));
        sessions.bind_tasks(&state, Some(&second.id), &Default::default());
        tokio::task::yield_now().await;
        assert!(!one.is_finished());
        let completion = |request_id, value: &str| NavAction::Completed {
            generation: 0,
            request_id,
            slot: "lyrics",
            result: Ok(vec![
                NavAction::Text {
                    request_id,
                    result: Ok(value.into()),
                }
                .into(),
            ]),
        };
        sessions
            .dispatch(
                completion(1, "First complete").into(),
                &mut state,
                &mut audio,
                &mut config,
                &events,
            )
            .await
            .unwrap();
        assert_eq!(state.popups.text().unwrap().request_id, 2);
        assert_eq!(state.sources.nav_tasks.get("lyrics").unwrap().0, 2);
        assert_eq!(
            sessions
                .models()
                .iter()
                .find(|model| model.id == first.id)
                .unwrap()
                .text,
            "First complete"
        );
        sessions
            .dispatch(
                completion(2, "Second complete").into(),
                &mut state,
                &mut audio,
                &mut config,
                &events,
            )
            .await
            .unwrap();
        assert_eq!(state.popups.text().unwrap().text, "Second complete");
        sessions
            .dispatch(
                completion(1, "Stale").into(),
                &mut state,
                &mut audio,
                &mut config,
                &events,
            )
            .await
            .unwrap();
        assert_eq!(state.popups.text().unwrap().text, "Second complete");
        one.abort();
        two.abort();
    }
}
