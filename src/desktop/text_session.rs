//! Composition of the existing terminal renderer with the live application actor.
use crate::{
    app::{self, Action, AppState, Event},
    config::Config,
    text_mode::{
        self, PresentedFrame,
        backend::{Renderer, Viewport},
    },
};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct TextSession {
    renderer: Option<Renderer>,
    frame: Option<Arc<PresentedFrame>>,
    viewport: Option<Viewport>,
    serial: u64,
}
impl TextSession {
    pub fn continuous(&self, state: &AppState) -> bool {
        state.playback.status == app::PlayStatus::Playing
            || self.renderer.as_ref().is_some_and(Renderer::animating)
    }
    pub fn frame(&self) -> Option<Arc<PresentedFrame>> {
        self.frame.clone()
    }
    pub fn viewport(
        &mut self,
        identity: app::windowed_presentation::Identity,
        viewport: Viewport,
        state: &mut AppState,
        config: &mut Config,
        queue_open: bool,
    ) -> Vec<Action> {
        if self.frame.as_ref().is_some_and(|frame| {
            frame.identity == identity && frame.viewport.revision > viewport.revision
        }) {
            return vec![];
        }
        let metrics = viewport.metrics;
        let preparing = state.windowed_presentation.pending().filter(|request| {
            request.identity == identity && request.destination == app::launch::WindowedMode::Text
        });
        if preparing.is_none()
            && !(state.windowed_presentation.accepts(identity)
                && state.windowed_presentation.active() == Some(app::launch::WindowedMode::Text))
        {
            return vec![];
        }
        let old_view = state.view;
        let old_frontend = state.desktop_frontend;
        let old_popups = state.popups.clone();
        let old_previous_view = state.previous_view;
        let old_settings_category = state.settings_state.category_index();
        let old_settings_item = state.settings_state.item_index;
        let old_settings_focus = state.settings_state.focus;
        let old_queue_visible = state.tui_queue_visible;
        let result = (|| -> anyhow::Result<()> {
            if self.renderer.is_none() {
                self.renderer = Some(Renderer::new(metrics)?);
            }
            state.desktop_frontend = false;
            if preparing.is_some() {
                state.view = state
                    .windowed_presentation
                    .project_view(app::launch::WindowedMode::Text, state.view);
                if let Some(draft) = state.staged_settings.clone() {
                    draft.restore(state);
                }
                state.tui_queue_visible = queue_open;
                if state.track_pane_open {
                    state.miller_reveal_right = true;
                }
            }
            let Some(renderer) = self.renderer.as_mut() else {
                return Ok(());
            };
            if self
                .viewport
                .is_none_or(|previous| previous.metrics != metrics)
            {
                state.seek_drag = None;
                state.volume_drag = false;
            }
            if preparing.is_some() {
                renderer.settle_motion();
            }
            renderer.set_geometry(viewport.geometry);
            renderer.resize(metrics)?;
            renderer.set_appearance(viewport.appearance);
            let frame = renderer.draw(state)?;
            self.publish(
                state,
                identity,
                viewport,
                frame,
                app::terminal_text::document(state),
            );
            self.viewport = Some(viewport);
            Ok(())
        })();
        match result {
            Ok(()) => {
                if let Some(preparation) = preparing
                    && state.windowed_presentation.commit(preparation)
                {
                    state.staged_settings = None;
                    config.ui.last_windowed_mode = app::launch::WindowedMode::Text;
                    return vec![app::action::SettingsAction::SaveSettings.into()];
                }
            }
            Err(error) => {
                state.view = old_view;
                state.desktop_frontend = old_frontend;
                state.popups = old_popups;
                state.previous_view = old_previous_view;
                state.settings_state.select_category(old_settings_category);
                state.settings_state.item_index = old_settings_item;
                state.settings_state.focus = old_settings_focus;
                state.tui_queue_visible = old_queue_visible;
                if let Some(preparation) = preparing {
                    state.windowed_presentation.fail(preparation);
                    self.suspend();
                }
                state.set_status(format!("Could not open Text Mode: {error}"));
            }
        }
        vec![]
    }
    pub fn render(&mut self, state: &mut AppState) -> anyhow::Result<()> {
        if state.windowed_presentation.active() != Some(app::launch::WindowedMode::Text) {
            return Ok(());
        }
        let Some(identity) = state.windowed_presentation.identity() else {
            return Ok(());
        };
        let (Some(renderer), Some(viewport)) = (self.renderer.as_mut(), self.viewport) else {
            return Ok(());
        };
        match state.view {
            app::View::NowPlaying => state
                .windowed_presentation
                .set_workspace(app::windowed_presentation::Workspace::NowPlaying),
            app::View::Browse => state
                .windowed_presentation
                .set_workspace(app::windowed_presentation::Workspace::Library),
            app::View::Queue => {
                state
                    .windowed_presentation
                    .set_workspace(match state.tui_queue_workspace {
                        app::state::TuiWorkspace::NowPlaying => {
                            app::windowed_presentation::Workspace::NowPlaying
                        }
                        _ => app::windowed_presentation::Workspace::Library,
                    })
            }
            _ => {}
        }
        let frame = renderer.draw(state)?;
        let anchors = app::view_continuity::capture_terminal(state);
        if !anchors.columns.is_empty() {
            state.browse_viewport = Some(anchors);
        }
        let editor = app::terminal_text::document(state);
        if self.frame.as_ref().is_some_and(|previous| {
            frame.dirty_rows.is_empty()
                && state.hit_regions.graphical_visualizer.is_none()
                && previous.frame.cursor == frame.cursor
                && previous.editor == editor
                && previous.queue_menus == state.hit_regions.queue_menus
                && previous.graphical_visualizer == state.hit_regions.graphical_visualizer
                && previous.transport_controls.as_slice()
                    == state
                        .hit_regions
                        .transport
                        .as_ref()
                        .map(|region| region.controls.as_slice())
                        .unwrap_or_default()
                && previous.muted == state.playback.muted
                && previous.playing == (state.playback.status == app::PlayStatus::Playing)
                && previous.position_ms == state.playback.position_ms
                && previous.duration_ms == state.playback.duration_ms
                && previous.input_region == state.hit_regions.text_input
                && previous.frame.images.len() == frame.images.len()
                && previous
                    .frame
                    .images
                    .iter()
                    .zip(&frame.images)
                    .all(|(a, b)| a.same_placement(b))
        }) {
            return Ok(());
        }
        self.publish(state, identity, viewport, frame, editor);
        Ok(())
    }
    fn publish(
        &mut self,
        state: &AppState,
        identity: app::windowed_presentation::Identity,
        viewport: Viewport,
        frame: text_mode::vt::Frame,
        editor: Option<app::terminal_text::Document>,
    ) {
        self.serial = self.serial.wrapping_add(1);
        self.frame = Some(Arc::new(PresentedFrame {
            identity,
            serial: self.serial,
            viewport,
            frame,
            breadcrumbs: super::browser::terminal_breadcrumbs(state),
            editor,
            queue_menus: state.hit_regions.queue_menus.clone(),
            graphical_visualizer: state.hit_regions.graphical_visualizer,
            transport_controls: state
                .hit_regions
                .transport
                .as_ref()
                .map(|region| region.controls.clone())
                .unwrap_or_default(),
            muted: state.playback.muted,
            playing: state.playback.status == app::PlayStatus::Playing,
            position_ms: state.playback.position_ms,
            duration_ms: state.playback.duration_ms,
            input_region: state.hit_regions.text_input.clone(),
        }));
    }
    pub fn input(
        &mut self,
        identity: app::windowed_presentation::Identity,
        viewport_revision: Option<u64>,
        input: text_mode::Input,
        state: &mut AppState,
        config: &Config,
        events: &mpsc::Sender<Event>,
    ) -> Vec<Action> {
        if viewport_revision.is_some_and(|revision| {
            self.viewport
                .is_none_or(|viewport| viewport.revision != revision)
        }) || !state.windowed_presentation.accepts(identity)
            || state.windowed_presentation.active() != Some(app::launch::WindowedMode::Text)
        {
            return vec![];
        }
        match input {
            text_mode::Input::Edit(edit) => app::terminal_text::edit(edit, state),
            text_mode::Input::Key(key) => {
                app::runtime::reduce(Event::Key(key), state, config, events)
            }
            text_mode::Input::Mouse(mouse) => {
                app::runtime::reduce(Event::Mouse(mouse), state, config, events)
            }
            text_mode::Input::Text(text) | text_mode::Input::Paste(text) => {
                app::runtime::reduce(Event::Paste(text), state, config, events)
            }
            text_mode::Input::Focus(focused) => {
                if !focused {
                    state.tui_shift_held = false;
                    state.seek_drag = None;
                    state.volume_drag = false;
                }
                vec![]
            }
        }
    }
    pub fn suspend(&mut self) {
        self.renderer = None;
        self.frame = None;
        self.viewport = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{launch::WindowedMode, windowed_presentation::Presentation};
    #[test]
    fn help_and_full_screen_search_remain_visible_when_entering_text() {
        for view in [app::View::Help, app::View::Search] {
            let mut state = AppState::new();
            state.view = view;
            state.windowed_presentation = Presentation::windowed();
            state.search.query = "unfinished search".into();
            let request = state
                .windowed_presentation
                .request(WindowedMode::Text)
                .unwrap();
            let mut text = TextSession::default();
            text.viewport(
                request.identity,
                viewport(1),
                &mut state,
                &mut Config::default(),
                false,
            );
            assert_eq!(state.view, view);
            assert_eq!(state.search.query, "unfinished search");
            let rendered: String = text
                .frame()
                .unwrap()
                .frame
                .cells
                .iter()
                .flat_map(|row| row.iter())
                .map(|cell| cell.text.as_str())
                .collect();
            match view {
                app::View::Help => assert!(state.hit_regions.help.is_some()),
                app::View::Search => assert!(rendered.contains("unfinished search")),
                _ => unreachable!(),
            }
        }
    }
    #[test]
    fn open_track_details_and_artwork_survive_presentation_switches() {
        use app::state::{BrowseColumn, BrowseItem};
        let track = crate::library::models::Track {
            rating_key: "track".into(),
            title: "Visible detail destination".into(),
            ..Default::default()
        };
        let mut state = AppState::new();
        state.windowed_presentation = Presentation::windowed();
        state.category_column_focused = false;
        state.track_pane_open = true;
        let mut column = BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        );
        column.artwork_visible = true;
        state.artist_nav.columns = vec![column];
        state
            .artwork
            .grid_cache
            .insert("cached-cover".into(), vec![1, 2, 3]);
        let mut config = Config::default();
        let mut text = TextSession::default();
        let request = state
            .windowed_presentation
            .request(WindowedMode::Text)
            .unwrap();
        text.viewport(
            request.identity,
            viewport(1),
            &mut state,
            &mut config,
            false,
        );
        assert!(state.track_pane_open);
        assert!(state.hit_regions.track_pane.is_some());
        let frame = text.frame().unwrap();
        let rendered = frame
            .frame
            .cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Track Details"), "{rendered}");
        assert_eq!(
            app::details::project(&state).unwrap().title,
            "Visible detail destination"
        );
        assert!(state.artist_nav.columns[0].artwork_visible);
        assert_eq!(state.artwork.grid_cache["cached-cover"], vec![1, 2, 3]);
        state.track_pane_open = false;
        let returning = state
            .windowed_presentation
            .request(WindowedMode::Gui)
            .unwrap();
        assert!(state.windowed_presentation.commit(returning));
        assert!(app::details::project(&state).is_none());
    }
    #[test]
    fn settings_restore_over_the_same_now_playing_workspace() {
        use app::windowed_presentation::Workspace;
        let mut state = AppState::new();
        state.windowed_presentation = Presentation::windowed();
        state
            .windowed_presentation
            .set_workspace(Workspace::NowPlaying);
        state.desktop_player_visible = false;
        let mut draft_state = AppState::new();
        draft_state.view = app::View::Settings;
        draft_state.settings_state.select_category(2);
        state.staged_settings = app::settings_transfer::Draft::capture(&draft_state);
        let request = state
            .windowed_presentation
            .request(WindowedMode::Text)
            .unwrap();
        TextSession::default().viewport(
            request.identity,
            viewport(1),
            &mut state,
            &mut Config::default(),
            false,
        );
        assert_eq!(state.view, app::View::Settings);
        assert_eq!(state.previous_view, Some(app::View::NowPlaying));
        assert_eq!(state.settings_state.category_index(), 2);
        assert_eq!(
            state.windowed_presentation.workspace(),
            Some(Workspace::NowPlaying)
        );
    }
    fn viewport(revision: u64) -> Viewport {
        Viewport {
            metrics: text_mode::backend::Metrics {
                columns: 110,
                rows: 35,
                cell_width: 18,
                cell_height: 40,
            },
            scale_factor: 2.,
            revision,
            geometry: Default::default(),
            appearance: text_mode::backend::Appearance {
                system: None,
                foreground: [240; 3],
                background: [20; 3],
            },
        }
    }
    #[test]
    fn switching_retains_filtered_navigation_and_rejects_inactive_or_resized_input() {
        let mut state = AppState::new();
        state.desktop_frontend = true;
        state.windowed_presentation = Presentation::windowed();
        state.artist_nav.reset(
            "Artists",
            vec![app::state::BrowseItem::Artist {
                key: "artist".into(),
                title: "Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        );
        state
            .artist_nav
            .drill_column(app::state::BrowseColumn::new("Albums", vec![]), false);
        state.list_filter.active = true;
        state.list_filter.query = "retained filter".into();
        state.list_filter.column = 1;
        state.palette.open = true;
        state.palette.query = "unfinished".into();
        state.palette.cursor = 10;
        state.playback.status = app::PlayStatus::Paused;
        state.playback.position_ms = 12345;
        state.playback.volume = 0.3;
        state.tui_sidebar_visible = false;
        let mut config = Config::default();
        let mut text = TextSession::default();
        let request = state
            .windowed_presentation
            .request(WindowedMode::Text)
            .unwrap();
        text.viewport(request.identity, viewport(1), &mut state, &mut config, true);
        assert_eq!(state.artist_nav.columns.len(), 2);
        assert_eq!(state.list_filter.query, "retained filter");
        assert_eq!(state.palette.query, "unfinished");
        assert_eq!(state.playback.position_ms, 12345);
        assert_eq!(state.playback.status, app::PlayStatus::Paused);
        assert!(!state.tui_sidebar_visible);
        assert!(state.tui_queue_visible);
        let (events, _) = mpsc::channel(8);
        text.viewport(request.identity, viewport(2), &mut state, &mut config, true);
        text.input(
            request.identity,
            Some(1),
            text_mode::Input::Paste("stale".into()),
            &mut state,
            &config,
            &events,
        );
        assert_eq!(state.palette.query, "unfinished");
        text.input(
            request.identity,
            None,
            text_mode::Input::Paste(" draft".into()),
            &mut state,
            &config,
            &events,
        );
        assert_eq!(state.palette.query, "unfinished draft");
        let next = state
            .windowed_presentation
            .request(WindowedMode::Gui)
            .unwrap();
        assert!(state.windowed_presentation.commit(next));
        text.suspend();
        text.input(
            request.identity,
            None,
            text_mode::Input::Paste("stale".into()),
            &mut state,
            &config,
            &events,
        );
        assert_eq!(state.palette.query, "unfinished draft");
        assert_eq!(state.list_filter.query, "retained filter");
        assert_eq!(state.playback.volume, 0.3);
    }
}
