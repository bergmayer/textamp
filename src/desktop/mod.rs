//! macOS desktop frontend built with GPUI Kit.
mod about;
mod artwork;
mod browser;
mod catalog;
mod column_layout;
mod commands;
mod connection;
mod engine;
mod filter;
#[cfg(all(test, feature = "desktop-tests"))]
mod frame_tests;
#[cfg(target_os = "macos")]
mod icon;
mod keyboard;
mod menu_catalog;
mod meters;
mod motion;
mod navigation;
mod palette;
mod player;
mod player_devices;
mod player_model;
mod preferences;
#[cfg(all(test, feature = "desktop-tests"))]
mod responsiveness_tests;
use crate::app::queue_edit as queue;
#[cfg(target_os = "macos")]
mod native_presentation;
mod queue_icons;
mod radio;
mod settings;
mod sidebar;
mod surface;
mod surface_view;
#[cfg(target_os = "macos")]
mod text;
#[cfg(target_os = "macos")]
mod text_boxdrawing;
#[cfg(target_os = "macos")]
mod text_session;
mod theme;
mod theme_preview;
mod views;
mod workflows;
mod workspace;
use crate::app::{action::PlaybackAction, state::PaletteCommandKind};
use engine::{Command, Engine, Snapshot};
use gpui_kit::component::WindowExt;
use gpui_kit::component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    menu::ContextMenuExt,
    slider::{SliderEvent, SliderState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;

gpui_kit::actions!(
    desktop,
    [
        Quit,
        Palette,
        Library,
        NowPlaying,
        Visualizer,
        Settings,
        PlayPause,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        Activate,
        ActivateFollowing,
        Minimize,
        CloseWindow,
        ResetView,
        Hide,
        HideOthers,
        ShowAll,
        ToggleNativeFullscreen,
        CycleApplicationMode
    ]
);
fn native_menus(radio: &[String]) -> Vec<Menu> {
    let mut menus = menu_catalog::menus(&menu_catalog::catalog(None));
    commands::add_radio_menu(&mut menus, radio);
    menus
}
fn native_menus_for_snapshot(snapshot: &Snapshot) -> Vec<Menu> {
    let mut menus = if snapshot.menu_commands.is_empty() {
        native_menus(&snapshot.radio_commands)
    } else {
        menu_catalog::menus(&snapshot.menu_commands)
    };
    if snapshot.separate_player
        && snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Gui)
        && let Some(window) = menus.iter_mut().find(|menu| menu.name == "Window")
    {
        let index = window.items.iter().position(|item| matches!(item, MenuItem::Action { action, .. } if action.as_any().is::<NowPlaying>())).map_or(window.items.len(), |index| index+1);
        window
            .items
            .insert(index, MenuItem::action("Visualizer", Visualizer));
    }
    commands::check_view_options(&mut menus, &snapshot.view_options);
    menus
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct LargeAlbumCovers;
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct RadioCommand {
    name: String,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct LetterJump {
    letter: String,
    second: bool,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct NativeCommand(NativeCommandKind);

#[derive(Clone, Copy, Debug, PartialEq)]
enum NativeCommandKind {
    CloseColumn,
    ToggleFullscreen,
    OpenSearch,
    GotoQueue,
    GoToRandomAlbum,
}

impl NativeCommand {
    fn command(&self) -> PaletteCommandKind {
        match self.0 {
            NativeCommandKind::CloseColumn => PaletteCommandKind::CloseColumn,
            NativeCommandKind::ToggleFullscreen => PaletteCommandKind::ToggleFullscreen,
            NativeCommandKind::OpenSearch => PaletteCommandKind::OpenSearch,
            NativeCommandKind::GotoQueue => PaletteCommandKind::GotoQueue,
            NativeCommandKind::GoToRandomAlbum => PaletteCommandKind::GoToRandomAlbum,
        }
    }
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct ItemCommand {
    target: browser::Target,
    intent: browser::Intent,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct CoreShortcut {
    key: String,
    shift: bool,
    control: bool,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct TrackContextCommand {
    target: browser::Target,
    label: String,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct ArtistContextCommand {
    target: browser::Target,
    biography: bool,
}
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(no_json)]
struct DescriptionContextCommand {
    target: browser::Target,
}
#[derive(Clone)]
struct QueueDrag {
    target: browser::Target,
    label: String,
}
impl Render for QueueDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::label::Label::new(self.label.clone()).p_2()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowRole {
    Library,
    NowPlaying,
    Queue,
    Visualizer,
}
impl WindowRole {
    fn index(self) -> usize {
        match self {
            Self::Library => 0,
            Self::NowPlaying => 1,
            Self::Queue => 2,
            Self::Visualizer => 4,
        }
    }
}
/// Cascade new native windows within the usable display bounds.
fn cascade_bounds(preferred: Size<Pixels>, cx: &App) -> Bounds<Pixels> {
    let mut bounds = Bounds::centered(None, preferred, cx);
    let offset = px((cx.windows().len() % 8) as f32 * 28.);
    if let Some(display) = cx.primary_display() {
        let visible = display.visible_bounds();
        bounds.origin.x = (bounds.origin.x + offset)
            .min(visible.origin.x + visible.size.width - preferred.width)
            .max(visible.origin.x);
        bounds.origin.y = (bounds.origin.y + offset)
            .min(visible.origin.y + visible.size.height - preferred.height)
            .max(visible.origin.y);
    }
    bounds
}

const SETTINGS_WINDOW: usize = 3;
type SettingsWindow = (AnyWindowHandle, Entity<settings::ConnectionSettings>);
#[derive(Clone, Default)]
struct WindowRegistry {
    settings_editor: std::rc::Rc<std::cell::RefCell<Option<SettingsWindow>>>,
    documents: std::rc::Rc<std::cell::RefCell<[Option<AnyWindowHandle>; 5]>>,
    workflows: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, AnyWindowHandle>>>,
}
/// Foreground ownership survives document projections that omit other windows' content.
enum HandoffWindow {
    Workflow(String),
    Settings,
    Library,
}
impl HandoffWindow {
    fn from_snapshot(snapshot: &Snapshot) -> Option<Self> {
        if let Some(surface) = &snapshot.surface {
            Some(if snapshot.separate_player {
                Self::Workflow(surface.id.clone())
            } else {
                Self::Library
            })
        } else {
            snapshot.settings_progress.as_ref().map(|_| Self::Settings)
        }
    }
}
impl WindowRegistry {
    fn activate_handoff_window(&self, target: Option<HandoffWindow>, cx: &mut App) {
        let handle = match target {
            Some(HandoffWindow::Workflow(id)) => self.workflows.borrow().get(&id).copied(),
            Some(HandoffWindow::Settings) => self
                .settings_editor
                .borrow()
                .as_ref()
                .map(|(handle, _)| *handle),
            Some(HandoffWindow::Library) => self.borrow()[WindowRole::Library.index()],
            None => None,
        };
        if let Some(handle) = handle {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
    }
}
impl std::ops::Deref for WindowRegistry {
    type Target = std::cell::RefCell<[Option<AnyWindowHandle>; 5]>;
    fn deref(&self) -> &Self::Target {
        &self.documents
    }
}
/// Native window coordination continues when no document window is visible.
fn monitor_windows(
    engine: std::sync::Arc<Engine>,
    windows: WindowRegistry,
    desktop: Entity<Desktop>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let mut requests = (0, 0, 0);
        let mut queue_request = 0;
        let mut queue_multi = false;
        let mut surface_request = 0;
        let mut surface_id = None;
        let mut menus = None;
        let mut save_files = std::collections::HashSet::new();
        let mut presentation_identity = engine.snapshot.borrow().presentation.identity();
        let mut updates = engine.snapshot.clone();
        while updates.changed().await.is_ok() {
            cx.update(|cx| {
                if engine.snapshot.borrow().presentation.active()
                    == Some(crate::app::launch::WindowedMode::Text)
                {
                    return;
                }
                {
                    let snapshot = engine.snapshot.borrow();
                    let models: Vec<_> = snapshot
                        .surfaces
                        .iter()
                        .chain(snapshot.surface.iter())
                        .filter(|model| model.save_file.is_some())
                        .collect();
                    save_files.retain(|id| models.iter().any(|model| &model.id == id));
                    for model in models {
                        if save_files.insert(model.id.clone()) {
                            surface_view::prompt_export(model, engine.commands.clone(), cx);
                        }
                    }
                }
                let queue_snapshot = engine.snapshot.borrow().clone();
                let returning_from_text =
                    presentation_identity != queue_snapshot.presentation.identity();
                presentation_identity = queue_snapshot.presentation.identity();
                if queue_snapshot.queue_request != queue_request
                    || queue_snapshot.separate_player != queue_multi
                {
                    queue_request = queue_snapshot.queue_request;
                    queue_multi = queue_snapshot.separate_player;
                    if queue_snapshot.separate_player && queue_snapshot.queue_open {
                        let _ = open_desktop_window(
                            engine.clone(),
                            windows.clone(),
                            WindowRole::Queue,
                            cx,
                        );
                    }
                }
                let library_open = windows.borrow()[WindowRole::Library.index()]
                    .is_some_and(|handle| handle.update(cx, |_, _, _| ()).is_ok());
                if library_open {
                    if returning_from_text {
                        windows.activate_handoff_window(
                            HandoffWindow::from_snapshot(&queue_snapshot),
                            cx,
                        );
                    }
                    let snapshot = engine.snapshot.borrow();
                    requests = (
                        snapshot.player_window_request,
                        snapshot.settings_request,
                        snapshot.library_window_request,
                    );
                    surface_request = snapshot.surface_request;
                    surface_id = snapshot.surface.as_ref().map(|model| model.id.clone());
                    return;
                }
                let snapshot = engine.snapshot.borrow().clone();
                let menu_state = (
                    snapshot.separate_player,
                    snapshot.radio_commands.clone(),
                    snapshot.view_options.clone(),
                    menu_catalog::signature(&snapshot.menu_commands),
                );
                if menus.as_ref() != Some(&menu_state) {
                    cx.set_menus(native_menus_for_snapshot(&snapshot));
                    menus = Some(menu_state);
                }
                if !snapshot.separate_player {
                    let _ = open_desktop_window(
                        engine.clone(),
                        windows.clone(),
                        WindowRole::Library,
                        cx,
                    );
                    return;
                }
                let mut workflows = windows.workflows.borrow_mut();
                workflows.retain(|id, handle| {
                    if snapshot.surfaces.iter().any(|model| &model.id == id) {
                        true
                    } else {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                        false
                    }
                });
                for model in snapshot
                    .surfaces
                    .iter()
                    .filter(|model| model.save_file.is_none())
                {
                    if !workflows.contains_key(&model.id) {
                        match surface_view::open_native(
                            model.clone(),
                            engine.commands.clone(),
                            engine.snapshot.clone(),
                            cx,
                        ) {
                            Ok(handle) => {
                                workflows.insert(model.id.clone(), handle);
                            }
                            Err(error) => tracing::error!(%error,"Open workflow window"),
                        }
                    }
                }
                let active = snapshot.surface.as_ref().map(|model| model.id.clone());
                if (snapshot.surface_request != surface_request || active != surface_id)
                    && let Some(handle) = active.as_ref().and_then(|id| workflows.get(id))
                {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }
                surface_id = active;
                surface_request = snapshot.surface_request;
                drop(workflows);
                if snapshot.library_window_request != requests.2 {
                    let _ = open_desktop_window(
                        engine.clone(),
                        windows.clone(),
                        WindowRole::Library,
                        cx,
                    );
                }
                if snapshot.player_window_request != requests.0
                    && (!snapshot.split_player || snapshot.player.fullscreen)
                {
                    let _ = open_player_windows(engine.clone(), windows.clone(), cx);
                }
                if snapshot.settings_request != requests.1 {
                    desktop.update(cx, |desktop, cx| {
                        if desktop.snapshot.presentation.active()
                            == Some(crate::app::launch::WindowedMode::Text)
                        {
                            desktop.command(PaletteCommandKind::OpenSettings);
                        } else {
                            desktop.open_settings(cx);
                        }
                    });
                }
                requests = (
                    snapshot.player_window_request,
                    snapshot.settings_request,
                    snapshot.library_window_request,
                );
            });
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
fn open_player_windows(
    engine: std::sync::Arc<Engine>,
    windows: WindowRegistry,
    cx: &mut App,
) -> anyhow::Result<()> {
    open_desktop_window(engine.clone(), windows.clone(), WindowRole::Visualizer, cx)?;
    open_desktop_window(engine, windows, WindowRole::NowPlaying, cx)?;
    Ok(())
}

fn open_desktop_window(
    engine: std::sync::Arc<Engine>,
    windows: WindowRegistry,
    role: WindowRole,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<Desktop>)> {
    let existing = windows.borrow()[role.index()];
    if let Some(handle) = existing {
        // Existing windows retain their own selection, scroll and focus state.
        if let Ok(entity) = handle.update(cx, |root, window, cx| {
            window.activate_window();
            root.downcast::<gpui_kit::base::Root>()
                .expect("desktop root")
                .read(cx)
                .view()
                .clone()
                .downcast::<Desktop>()
                .expect("desktop window root")
        }) {
            return Ok((handle, entity));
        }
    }
    let registry = windows.clone();
    let commands = engine.commands.clone();
    let closing_engine = engine.clone();
    let (handle, entity) = open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(
                    match role {
                        WindowRole::Library => "Textamp — Library",
                        WindowRole::NowPlaying => "Textamp — Now Playing",
                        WindowRole::Queue => "Textamp — Play Queue",
                        WindowRole::Visualizer => "Textamp — Visualizer",
                    }
                    .into(),
                ),
                ..gpui_kit::component::TitleBar::title_bar_options()
            }),
            show: role == WindowRole::NowPlaying
                || !std::env::args().any(|arg| arg == "--settings")
                || !engine.snapshot.borrow().separate_player,
            window_min_size: Some(if role == WindowRole::Queue {
                size(px(320.), px(240.))
            } else {
                size(px(260.), px(140.))
            }),
            window_bounds: Some(WindowBounds::Windowed(cascade_bounds(
                if role == WindowRole::Queue {
                    size(px(360.), px(640.))
                } else if role == WindowRole::Visualizer {
                    size(px(960.), px(420.))
                } else if role == WindowRole::NowPlaying {
                    size(px(840.), px(550.))
                } else {
                    size(px(1180.), px(780.))
                },
                cx,
            ))),
            ..gpui_kit::component::TitleBar::window_options()
        },
        cx,
        |window, cx| {
            #[cfg(target_os = "macos")]
            if role == WindowRole::Library {
                native_presentation::register(window);
            }
            if role == WindowRole::NowPlaying {
                let _ = commands.try_send(Command::PlayerWindowVisible(true));
            }
            window.on_window_should_close(cx, move |_, cx| {
                if !closing_engine.snapshot.borrow().separate_player {
                    cx.quit();
                    return false;
                }
                if role == WindowRole::NowPlaying {
                    let _ = commands.try_send(Command::PlayerWindowVisible(false));
                } else if role == WindowRole::Queue {
                    let _ = commands.try_send(Command::QueueWindowClosed);
                }
                true
            });
            cx.new(|cx| Desktop::new(engine, role, registry, window, cx))
        },
    )?;
    windows.borrow_mut()[role.index()] = Some(handle);
    Ok((handle, entity))
}
/// Ownership of a Kit dialog and its acknowledgement by application state.
#[derive(Default)]
enum OverlayHost {
    #[default]
    Closed,
    Visible {
        id: String,
        window: AnyWindowHandle,
    },
    Dismissing {
        id: String,
    },
}
impl OverlayHost {
    fn id(&self) -> Option<&String> {
        match self {
            Self::Closed => None,
            Self::Visible { id, .. } | Self::Dismissing { id } => Some(id),
        }
    }
    fn begin_dismissal(&mut self) -> Option<AnyWindowHandle> {
        match std::mem::take(self) {
            Self::Visible { id, window } => {
                *self = Self::Dismissing { id };
                Some(window)
            }
            other => {
                *self = other;
                None
            }
        }
    }
    fn did_close(&mut self, closed_id: &str) {
        if self.id().is_some_and(|id| id == closed_id) {
            *self = Self::Dismissing {
                id: closed_id.into(),
            };
        }
    }
}

struct Desktop {
    presentation_feedback: Option<crate::app::windowed_presentation::Identity>,
    initial_window_ready: bool,
    suspended_dialogs: Option<gpui_kit::component::SuspendedDialogs>,
    #[cfg(target_os = "macos")]
    text_surface: Option<(
        crate::app::windowed_presentation::Identity,
        Entity<text::TextSurface>,
    )>,
    filter_input: Entity<gpui_kit::component::input::InputState>,
    description_input: Entity<gpui_kit::component::input::InputState>,
    syncing_filter: bool,
    filter_expected: String,
    pending_centered_rows: std::collections::HashMap<String, usize>,
    initial_column_height: Pixels,
    engine: std::sync::Arc<Engine>,
    role: WindowRole,
    presentation: WindowRole,
    library_size: Size<Pixels>,
    arrow_repeat: navigation::ArrowRepeat,
    rapid_navigation: bool,
    sidebar_visible: bool,
    sidebar_width: Pixels,
    jump_viewports:
        std::collections::VecDeque<(u64, std::collections::HashMap<String, Point<Pixels>>)>,
    sidebar_scroll: ScrollHandle,
    details_scroll: ScrollHandle,
    windows: WindowRegistry,
    snapshot: Snapshot,
    applied_theme: Option<crate::app::theme::ThemeName>,
    focus: FocusHandle,
    seek_slider: Entity<SliderState>,
    volume_slider: Entity<SliderState>,
    scrub: Option<u64>,
    volume_dragging: bool,
    seek_expected: f32,
    volume_expected: f32,
    visualizer_bounds: Option<Bounds<Pixels>>,
    sheet_size: Size<Pixels>,
    queue_width: Pixels,
    queue_progress: f32,
    sheet_header_height: Pixels,
    transport_artwork_x: f32,
    column_motion_until: Option<std::time::Instant>,
    drawn_presentation: Option<crate::app::windowed_presentation::Identity>,
    column_layout: Option<column_layout::Layout>,
    column_reveal: Option<usize>,
    column_viewport_width: Pixels,
    pane_targets: std::collections::HashMap<motion::PaneKey, motion::PaneMotion>,
    pane_presented: std::collections::HashMap<motion::PaneKey, Bounds<Pixels>>,
    waveform_scrub: Option<(u64, f32)>,
    queue_drop_after: Option<(browser::Target, bool)>,
    seek_focus: FocusHandle,
    volume_focus: FocusHandle,
    scroll_handles: std::collections::HashMap<String, UniformListScrollHandle>,
    column_scroll: ScrollHandle,
    visualizer_mode_scroll: ScrollHandle,
    layout_epoch: u64,
    scroll_pin: Option<String>,
    queue_focused: bool,
    scroll_activity: std::collections::HashMap<String, (std::time::Instant, Pixels)>,
    letter_shortcuts: Option<(String, std::time::Instant)>,
    _subscriptions: Vec<Subscription>,
    radio_browser: Option<Entity<surface_view::SurfaceView>>,
    workspace_view: Option<Entity<surface_view::SurfaceView>>,
    player_devices: Option<Entity<player_devices::PlayerDevices>>,
    overlay: OverlayHost,
    transport_art: Option<std::sync::Arc<Image>>,
    inspector_art: Option<std::sync::Arc<Image>>,
    image_cache: std::collections::HashMap<String, std::sync::Arc<Image>>,
    visible_rows: std::collections::HashMap<String, (std::ops::Range<usize>, usize, u64)>,
    _refresh: Task<()>,
}
impl Desktop {
    fn new(
        engine: std::sync::Arc<Engine>,
        role: WindowRole,
        windows: WindowRegistry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_theme = engine.snapshot.borrow().theme;
        theme::apply(initial_theme, Some(window), cx);
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let filter_input = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx).placeholder("Filter column…")
        });
        let description_input = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx).placeholder("Describe music")
        });
        let seek_slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.0001));
        let volume_slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.01));
        let mut subscriptions = vec![
            cx.subscribe(&seek_slider, |this, _, event: &SliderEvent, cx| {
                match event {
                    SliderEvent::Change(value) => {
                        this.seek_expected = value.end();
                        if this.scrub.is_none() {
                            this.scrub = Some(this.snapshot.playback_id);
                        }
                    }
                    SliderEvent::Release(value) => {
                        if let Some(playback_id) = this.scrub.take() {
                            let _ = this.engine.commands.try_send(Command::Seek {
                                playback_id,
                                fraction: value.end(),
                            });
                        }
                    }
                }
                cx.notify();
            }),
            cx.subscribe(&volume_slider, |this, _, event: &SliderEvent, cx| {
                match event {
                    SliderEvent::Change(value) => {
                        this.volume_expected = value.end();
                        this.volume_dragging = true;
                        let _ = this.engine.commands.try_send(Command::Action(
                            PlaybackAction::SetVolume(value.end()).into(),
                        ));
                    }
                    SliderEvent::Release(_) => this.volume_dragging = false,
                }
                cx.notify();
            }),
        ];
        // Kit 0.9 updates SliderState without emitting SliderEvent for accessibility
        // increments. Observe those changes as input; expected values exclude our syncs.
        subscriptions.extend([
            cx.observe(&seek_slider, |this, slider, cx| {
                let value = slider.read(cx).value().end();
                if (value - this.seek_expected).abs() > 0.00001 {
                    this.seek_expected = value;
                    if this.scrub.is_none() && this.snapshot.seekable {
                        let _ = this.engine.commands.try_send(Command::Seek {
                            playback_id: this.snapshot.playback_id,
                            fraction: value,
                        });
                    }
                }
            }),
            cx.observe(&volume_slider, |this, slider, cx| {
                let value = slider.read(cx).value().end();
                if (value - this.volume_expected).abs() > 0.00001 {
                    this.volume_expected = value;
                    let _ = this
                        .engine
                        .commands
                        .try_send(Command::Action(PlaybackAction::SetVolume(value).into()));
                }
            }),
        ]);
        let mut updates = engine.snapshot.clone();
        let refresh = cx.spawn_in(window, async move |entity, cx| {
            while updates.changed().await.is_ok() {
                let snapshot = updates.borrow_and_update().clone();
                if cx
                    .update(|window, cx| {
                        entity.update(cx, |this, cx| this.apply_snapshot(snapshot, window, cx))
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        subscriptions.push(
            cx.observe_in(&filter_input, window, move |this, input, _, cx| {
                if this.syncing_filter {
                    return;
                }
                let query = input.read(cx).value().to_string();
                if query == this.filter_expected {
                    return;
                }
                this.filter_expected = query.clone();
                let column = this.snapshot.filter.column;
                if let Some(c) = this.snapshot.columns.get(column) {
                    let _ = this.engine.commands.try_send(Command::ColumnFilter {
                        generation: this.snapshot.generation,
                        column,
                        identity: c.identity.clone(),
                        query,
                    });
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &filter_input,
            window,
            |this, _, event: &gpui_kit::component::input::InputEvent, window, cx| {
                if matches!(
                    event,
                    gpui_kit::component::input::InputEvent::PressEnter { .. }
                ) {
                    let _ = this.engine.commands.try_send(Command::Navigate {
                        column_delta: 1,
                        row_delta: 0,
                    });
                    this.focus.focus(window, cx);
                }
            },
        ));
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.snapshot.theme == crate::app::theme::ThemeName::System {
                theme::apply(this.snapshot.theme, Some(window), cx);
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            this.rapid_navigation = false;
            if window.is_window_active()
                && this.snapshot.presentation.active()
                    == Some(crate::app::launch::WindowedMode::Gui)
                && this.snapshot.presentation.pending().is_none()
                && matches!(this.role, WindowRole::Library | WindowRole::NowPlaying)
            {
                let role = if this.snapshot.separate_player {
                    this.role
                } else {
                    this.presentation
                };
                let workspace = if role == WindowRole::NowPlaying {
                    crate::app::windowed_presentation::Workspace::NowPlaying
                } else {
                    crate::app::windowed_presentation::Workspace::Library
                };
                let _ = this
                    .engine
                    .commands
                    .try_send(Command::DisplayedWorkspace(workspace));
            }
            cx.notify();
            if this.snapshot.theme == crate::app::theme::ThemeName::System {
                theme::apply(this.snapshot.theme, Some(window), cx);
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe(&description_input, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe_in(
            &description_input,
            window,
            |this, _, event: &gpui_kit::component::input::InputEvent, window, cx| {
                if matches!(
                    event,
                    gpui_kit::component::input::InputEvent::PressEnter { .. }
                ) {
                    this.submit_description(window, cx);
                }
                cx.notify();
            },
        ));
        Self {
            presentation_feedback: None,
            initial_window_ready: false,
            suspended_dialogs: None,
            #[cfg(target_os = "macos")]
            text_surface: None,
            description_input,
            filter_input,
            syncing_filter: false,
            filter_expected: String::new(),
            pending_centered_rows: Default::default(),
            initial_column_height: window.viewport_size().height,
            inspector_art: None,
            transport_art: None,
            image_cache: Default::default(),
            visible_rows: Default::default(),
            radio_browser: None,
            workspace_view: None,
            player_devices: None,
            overlay: OverlayHost::Closed,
            engine: engine.clone(),
            role,
            presentation: role,
            library_size: size(px(1180.), px(780.)),
            arrow_repeat: navigation::ArrowRepeat::default(),
            rapid_navigation: false,
            sidebar_visible: true,
            sidebar_width: px(motion::SIDEBAR_WIDTH),
            jump_viewports: Default::default(),
            sidebar_scroll: ScrollHandle::new(),
            details_scroll: ScrollHandle::new(),
            windows,
            snapshot: Snapshot {
                theme: initial_theme,
                separate_player: engine.snapshot.borrow().separate_player,
                settings_request: engine.snapshot.borrow().settings_request,
                player_window_request: engine.snapshot.borrow().player_window_request,
                library_window_request: engine.snapshot.borrow().library_window_request,
                ..Default::default()
            },
            applied_theme: Some(initial_theme),
            focus,
            seek_slider,
            volume_slider,
            scrub: None,
            volume_dragging: false,
            seek_expected: 0.,
            volume_expected: 0.,
            visualizer_bounds: None,
            sheet_size: size(
                window.viewport_size().width,
                (window.viewport_size().height
                    - gpui_kit::component::TITLE_BAR_HEIGHT
                    - px(crate::ui::layout::FOOTER_HEIGHT))
                .max(px(114.)),
            ),
            queue_width: px(motion::SIDEBAR_WIDTH),
            queue_progress: 0.,
            sheet_header_height: px(crate::ui::layout::player_header(
                f32::from(
                    window.viewport_size().height
                        - gpui_kit::component::TITLE_BAR_HEIGHT
                        - px(crate::ui::layout::FOOTER_HEIGHT),
                ),
                f32::from(window.viewport_size().width),
            )),
            transport_artwork_x: 172.,
            column_motion_until: None,
            drawn_presentation: None,
            column_layout: None,
            column_reveal: None,
            column_viewport_width: px(0.),
            pane_targets: Default::default(),
            pane_presented: Default::default(),
            waveform_scrub: None,
            queue_drop_after: None,
            seek_focus: cx.focus_handle().tab_stop(true),
            volume_focus: cx.focus_handle().tab_stop(true),
            scroll_handles: Default::default(),
            column_scroll: ScrollHandle::new(),
            visualizer_mode_scroll: ScrollHandle::new(),
            layout_epoch: 0,
            scroll_pin: None,
            queue_focused: false,
            scroll_activity: Default::default(),
            letter_shortcuts: None,
            _subscriptions: subscriptions,
            _refresh: refresh,
        }
    }
    fn apply_snapshot(
        &mut self,
        mut snapshot: Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_graphical_frame = !self.initial_window_ready
            && self.role == WindowRole::Library
            && snapshot.presentation.pending().is_none()
            && snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Gui);
        if first_graphical_frame && let Some(identity) = snapshot.presentation.identity() {
            self.initial_window_ready = true;
            let commands = self.engine.commands.clone();
            window.on_next_frame(move |_, _| {
                let _ = commands.try_send(Command::WindowedReady {
                    identity,
                    mode: crate::app::launch::WindowedMode::Gui,
                });
            });
        }
        if let Some(visible) = snapshot.sidebar_visible {
            self.sidebar_visible = visible;
        }
        let returning_from_text = self.snapshot.presentation.active()
            == Some(crate::app::launch::WindowedMode::Text)
            && snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Gui);
        let handoff_window = returning_from_text
            .then(|| HandoffWindow::from_snapshot(&snapshot))
            .flatten();
        if let Some(identity) = snapshot.presentation.identity() {
            self.engine.commands.observe(identity);
        }
        #[cfg(target_os = "macos")]
        if self.role == WindowRole::Library {
            native_presentation::register(window);
            use crate::app::launch::WindowedMode;
            if let Some(preparation) = snapshot.presentation.pending() {
                if native_presentation::modal_active() {
                    let _ = self
                        .engine
                        .commands
                        .try_send(Command::PresentationDeferred(preparation));
                } else {
                    match preparation.destination {
                        WindowedMode::Text => {
                            if self.presentation_feedback != Some(preparation.identity) {
                                let settings_editor = self.windows.settings_editor.borrow().clone();
                                if let Some((handle, editor)) = settings_editor {
                                    let staged = if handle == window.window_handle() {
                                        editor.update(cx, |editor, cx| editor.handoff(window, cx))
                                    } else {
                                        handle
                                            .update(cx, |_, window, cx| {
                                                editor.update(cx, |editor, cx| {
                                                    editor.handoff(window, cx)
                                                })
                                            })
                                            .and_then(|result| result)
                                    };
                                    if let Err(error) = staged {
                                        tracing::warn!(%error, "Keep the current presentation until Settings is preserved");
                                        return;
                                    }
                                }
                                if self
                                    .windows
                                    .settings_editor
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|(handle, _)| *handle == window.window_handle())
                                {
                                    window.close_dialog(cx);
                                    self.windows.settings_editor.borrow_mut().take();
                                }

                                if let Some(category) = snapshot.browse_category {
                                    let columns = snapshot
                                        .columns
                                        .iter()
                                        .enumerate()
                                        .filter_map(|(column, model)| {
                                            let offset = self
                                                .scroll_handles
                                                .get(&model.identity)
                                                .map_or(0., |handle| -f32::from(handle.offset().y));
                                            let position = offset / model.row_height();
                                            let display = position.floor().max(0.) as usize;
                                            let original = filter::row(&snapshot, column, display)?;
                                            Some(crate::app::view_continuity::ColumnAnchor {
                                                column,
                                                item_key: model.rows.get(original)?.key.clone(),
                                                fraction: position.fract(),
                                                pointer_pinned: self.scroll_pin.as_ref()
                                                    == Some(&model.identity),
                                            })
                                        })
                                        .collect();
                                    let _ = self.engine.commands.try_send(
                                        Command::PresentationFeedback(
                                            crate::app::view_continuity::BrowseAnchors {
                                                generation: snapshot.generation,
                                                category,
                                                columns,
                                            },
                                        ),
                                    );
                                }
                                self.presentation_feedback = Some(preparation.identity);
                            }
                            let surface = self.text_surface.get_or_insert_with(|| {
                                (
                                    preparation.identity,
                                    cx.new(|cx| {
                                        text::TextSurface::new(
                                            self.engine.commands.clone(),
                                            preparation.identity,
                                            window,
                                            cx,
                                        )
                                    }),
                                )
                            });
                            if surface.0 != preparation.identity {
                                *surface = (
                                    preparation.identity,
                                    cx.new(|cx| {
                                        text::TextSurface::new(
                                            self.engine.commands.clone(),
                                            preparation.identity,
                                            window,
                                            cx,
                                        )
                                    }),
                                );
                            }
                            let bounds = Bounds {
                                origin: point(px(0.), gpui_kit::component::TITLE_BAR_HEIGHT),
                                size: size(
                                    window.viewport_size().width,
                                    (window.viewport_size().height
                                        - gpui_kit::component::TITLE_BAR_HEIGHT)
                                        .max(px(20.)),
                                ),
                            };
                            surface.1.update(cx, |surface, cx| {
                                surface.geometry = crate::ui::layout::WindowGeometry {
                                    sidebar_width: f32::from(self.sidebar_width).round() as u16,
                                    queue_width: f32::from(self.queue_width).round() as u16,
                                    player_header: f32::from(self.sheet_header_height).round()
                                        as u16,
                                };
                                surface.prepare(bounds, window, cx);
                            });
                        }
                        WindowedMode::Gui => {
                            let _ = self
                                .engine
                                .commands
                                .try_send(Command::PresentationReady(preparation));
                        }
                    }
                }
            }
            if snapshot.presentation.active() == Some(WindowedMode::Text) {
                if self.snapshot.presentation.active() != Some(WindowedMode::Text) {
                    self.suspended_dialogs = Some(window.suspend_dialogs(cx));
                    native_presentation::suspend(true);
                }
                if let Some(frame) = snapshot.text_frame.clone() {
                    if self
                        .text_surface
                        .as_ref()
                        .is_none_or(|surface| surface.0 != frame.identity)
                    {
                        self.text_surface = Some((
                            frame.identity,
                            cx.new(|cx| {
                                text::TextSurface::new(
                                    self.engine.commands.clone(),
                                    frame.identity,
                                    window,
                                    cx,
                                )
                            }),
                        ));
                    }
                    if let Some((_, surface)) = &self.text_surface {
                        surface.update(cx, |surface, cx| {
                            surface.player = snapshot.player.clone();
                            surface.apply(frame, window, cx);
                        });
                        if self.snapshot.presentation.active() != Some(WindowedMode::Text) {
                            let surface = surface.clone();
                            window.defer(cx, move |window, cx| {
                                window.activate_window();
                                surface.update(cx, |surface, cx| surface.activate(window, cx));
                            });
                        }
                    }
                }
                if self.applied_theme != Some(snapshot.theme) {
                    theme::apply(snapshot.theme, Some(window), cx);
                    self.applied_theme = Some(snapshot.theme);
                }
                if snapshot.view_options != self.snapshot.view_options
                    || snapshot.disabled_queue_menu_items != self.snapshot.disabled_queue_menu_items
                {
                    cx.set_menus(native_menus_for_snapshot(&snapshot));
                }
                let changed = self.snapshot.text_frame.as_ref().map(|frame| frame.serial)
                    != snapshot.text_frame.as_ref().map(|frame| frame.serial)
                    || self.snapshot.presentation.identity() != snapshot.presentation.identity();
                self.snapshot = snapshot;
                if changed {
                    cx.notify();
                }
                return;
            }
            if self.snapshot.presentation.active() == Some(WindowedMode::Text) {
                native_presentation::suspend(false);
                self.text_surface = None;
                self.focus.focus(window, cx);
                if let Some(dialogs) = self.suspended_dialogs.take() {
                    window.resume_dialogs(dialogs, cx);
                }
            }
        }
        let restoring =
            snapshot.jump_restore.is_some() && snapshot.jump_restore != self.snapshot.jump_restore;
        if snapshot.queue_open != self.snapshot.queue_open {
            self.queue_focused = snapshot.queue_open;
            self.focus.focus(window, cx);
        }
        if snapshot.generation != self.snapshot.generation {
            self.description_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.jump_viewports.clear();
        }
        if !restoring
            && snapshot.jump_head != self.snapshot.jump_head
            && let Some(id) = snapshot.jump_head
        {
            self.jump_viewports.push_back((
                id,
                self.scroll_handles
                    .iter()
                    .map(|(key, handle)| (key.clone(), handle.offset()))
                    .collect(),
            ));
            while self.jump_viewports.len() > 8 {
                self.jump_viewports.pop_front();
            }
        }
        if self.role == WindowRole::Visualizer && !snapshot.separate_player {
            window.remove_window();
            return;
        }
        if snapshot.separate_player {
            let artwork_viewer = snapshot.player.viewer.is_artwork();
            match self.role {
                WindowRole::Library => snapshot.player.fullscreen = false,
                WindowRole::NowPlaying if !artwork_viewer => snapshot.player.fullscreen = false,
                WindowRole::Visualizer => {
                    snapshot.player.fullscreen &= !artwork_viewer;
                }
                _ => {}
            }
        }
        if self.role == WindowRole::Queue {
            snapshot.player.fullscreen = false;
            snapshot.player.viewer = crate::app::state::DesktopFullscreenView::Closed;
        }
        if self.role == WindowRole::Queue && (!snapshot.separate_player || !snapshot.queue_open) {
            window.remove_window();
            return;
        }
        if self.role == WindowRole::NowPlaying
            && (!snapshot.separate_player || (snapshot.split_player && !snapshot.player.fullscreen))
        {
            let _ = open_desktop_window(
                self.engine.clone(),
                self.windows.clone(),
                WindowRole::Library,
                cx,
            );
            window.remove_window();
            return;
        }
        if snapshot.split_player && !snapshot.player.fullscreen {
            self.presentation = WindowRole::Library;
        }
        let entering_multiple_windows = self.role == WindowRole::Library
            && snapshot.separate_player
            && snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Gui)
            && snapshot.presentation.pending().is_none()
            && (first_graphical_frame || returning_from_text || !self.snapshot.separate_player);
        if matches!(self.role, WindowRole::Library | WindowRole::NowPlaying)
            && (snapshot.player_window_request != self.snapshot.player_window_request
                || entering_multiple_windows)
            && (!snapshot.split_player || snapshot.player.fullscreen)
        {
            if !snapshot.separate_player {
                self.presentation = WindowRole::NowPlaying;
                window.activate_window();
            } else if self.role == WindowRole::NowPlaying {
                window.activate_window();
            } else {
                let _ = open_player_windows(self.engine.clone(), self.windows.clone(), cx);
            }
        }
        if snapshot.library_window_request != self.snapshot.library_window_request {
            if !snapshot.separate_player {
                self.presentation = WindowRole::Library;
                if player::is_mini(window.viewport_size()) {
                    window.resize(self.library_size);
                }
                window.activate_window();
            } else if self.role == WindowRole::Library {
                window.activate_window();
            } else {
                let _ = open_desktop_window(
                    self.engine.clone(),
                    self.windows.clone(),
                    WindowRole::Library,
                    cx,
                );
            }
        }
        if self.role == WindowRole::Library
            && self.snapshot.separate_player
            && !snapshot.separate_player
        {
            self.presentation = match snapshot.presentation.workspace() {
                Some(crate::app::windowed_presentation::Workspace::NowPlaying) => {
                    WindowRole::NowPlaying
                }
                _ => WindowRole::Library,
            };
        }
        snapshot.player.active = if snapshot.separate_player {
            self.role
        } else {
            self.presentation
        } != WindowRole::Library;
        if self.snapshot.workspace.is_some()
            && !self.snapshot.player.active
            && (snapshot.workspace.is_none() || snapshot.player.active)
        {
            self.focus.focus(window, cx);
        }
        if snapshot.player.active {
            snapshot.split_player = false;
            if snapshot.separate_player {
                snapshot.surface = None;
                snapshot.section = "Now Playing".into();
                snapshot.filter = Default::default();
                snapshot.columns.clear();
                snapshot.preview = None;
            }
        }
        if snapshot.generation != self.snapshot.generation {
            self.column_layout = None;
            self.pane_targets.clear();
            self.pane_presented.clear();
        }
        let shape = |snapshot: &Snapshot| {
            (
                snapshot
                    .columns
                    .iter()
                    .filter(|column| !column.hidden && !column.metadata)
                    .count(),
                snapshot.preview.is_some(),
            )
        };
        let columns_changed = snapshot
            .columns
            .iter()
            .filter(|column| !column.hidden && !column.metadata)
            .map(|column| &column.identity)
            .ne(self
                .snapshot
                .columns
                .iter()
                .filter(|column| !column.hidden && !column.metadata)
                .map(|column| &column.identity));
        if columns_changed || shape(&snapshot) != shape(&self.snapshot) {
            let (columns, inspector) = shape(&snapshot);
            self.column_reveal = (columns + usize::from(inspector)).checked_sub(1);
            self.column_motion_until = Some(
                std::time::Instant::now()
                    + motion::COLUMN_DURATION
                    + std::time::Duration::from_millis(80),
            );
        }
        let preview_changed = match (&snapshot.preview, &self.snapshot.preview) {
            (Some(next), Some(previous)) => {
                next.kind != previous.kind
                    || next.title != previous.title
                    || next.track.as_ref().map(|track| {
                        (
                            &track.rating_key,
                            &track.parent_rating_key,
                            &track.grandparent_rating_key,
                        )
                    }) != previous.track.as_ref().map(|track| {
                        (
                            &track.rating_key,
                            &track.parent_rating_key,
                            &track.grandparent_rating_key,
                        )
                    })
            }
            (Some(_), None) => true,
            _ => false,
        };
        if preview_changed {
            let (columns, _) = shape(&snapshot);
            self.column_reveal = Some(columns);
        }
        if returning_from_text {
            self.pane_targets.clear();
            self.pane_presented.clear();
        }
        if returning_from_text || snapshot.filter.active || self.snapshot.filter.active {
            self.column_motion_until = None;
        }
        if snapshot.sidebar_cursor != self.snapshot.sidebar_cursor
            && let Some(index) = snapshot.sidebar_cursor
            && let Some(position) =
                sidebar::visible_rows(&snapshot.sidebar, &snapshot.collapsed_sidebar_sections)
                    .iter()
                    .position(|row| *row == index)
        {
            self.sidebar_scroll.scroll_to_item(position);
        }
        let preview_track = |snapshot: &Snapshot| {
            snapshot
                .preview
                .as_ref()
                .and_then(|preview| preview.track.as_ref())
                .map(|track| track.rating_key.clone())
        };
        if preview_track(&snapshot) != preview_track(&self.snapshot) {
            self.details_scroll.set_offset(point(px(0.), px(0.)));
        }
        if snapshot.player.artwork != self.snapshot.player.artwork
            || snapshot.artwork_mode != self.snapshot.artwork_mode
        {
            self.transport_art = snapshot
                .player
                .artwork
                .as_ref()
                .and_then(|bytes| player::image_from_bytes(bytes, snapshot.artwork_mode));
        }
        if snapshot.generation != self.snapshot.generation
            || snapshot.filter.column != self.snapshot.filter.column
            || (!snapshot.filter.active && self.snapshot.filter.active)
        {
            self.syncing_filter = true;
            self.filter_expected.clear();
            self.filter_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.syncing_filter = false;
        }
        if snapshot.filter.active
            && (!self.snapshot.filter.active
                || snapshot.filter.column != self.snapshot.filter.column)
            && snapshot.filter.query.is_empty()
        {
            self.filter_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
        if snapshot.radio_commands != self.snapshot.radio_commands
            || snapshot.separate_player != self.snapshot.separate_player
            || snapshot.view_options != self.snapshot.view_options
            || menu_catalog::signature(&snapshot.menu_commands)
                != menu_catalog::signature(&self.snapshot.menu_commands)
        {
            cx.set_menus(native_menus_for_snapshot(&snapshot));
        }
        if self.role == WindowRole::Library
            && (snapshot.settings_request != self.snapshot.settings_request
                || self.settings_host_needs_update(&snapshot, cx))
        {
            self.open_settings(cx);
        }
        if self.applied_theme != Some(snapshot.theme) {
            self.applied_theme = Some(snapshot.theme);
            theme::apply(snapshot.theme, Some(window), cx);
            gpui_kit::component::Theme::set_scrollbar_mode(
                gpui_kit::component::scroll::ScrollbarMode::Always,
                cx,
            );
        }
        if !std::sync::Arc::ptr_eq(&snapshot.images, &self.snapshot.images)
            || snapshot.artwork_mode != self.snapshot.artwork_mode
        {
            self.image_cache
                .retain(|key, _| snapshot.images.contains_key(key));
            for (key, bytes) in snapshot.images.iter() {
                if (self
                    .snapshot
                    .images
                    .get(key)
                    .is_none_or(|old| !std::sync::Arc::ptr_eq(old, bytes))
                    || snapshot.artwork_mode != self.snapshot.artwork_mode)
                    && let Some(image) = player::image_from_bytes(bytes, snapshot.artwork_mode)
                {
                    self.image_cache.insert(key.clone(), image);
                }
            }
        }
        if snapshot
            .columns
            .iter()
            .zip(&self.snapshot.columns)
            .any(|(a, b)| {
                a.artwork_visible != b.artwork_visible || a.large_artwork != b.large_artwork
            })
        {
            self.visible_rows.clear();
        }
        if snapshot.artwork != self.snapshot.artwork
            || snapshot.artwork_mode != self.snapshot.artwork_mode
        {
            self.inspector_art = snapshot
                .artwork
                .as_ref()
                .and_then(|bytes| player::image_from_bytes(bytes, snapshot.artwork_mode));
        }
        // A gesture retains its starting playback identity through release.
        // If the track changes mid-drag, the engine rejects that old request.
        if self.scrub.is_none() {
            let value = if snapshot.duration_ms > 0 {
                snapshot.position_ms as f32 / snapshot.duration_ms as f32
            } else {
                0.
            };
            self.seek_expected = value;
            if (self.seek_slider.read(cx).value().end() - value).abs() > 0.00001 {
                self.seek_slider
                    .update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
        if !self.volume_dragging
            && (self.volume_slider.read(cx).value().end() - snapshot.volume).abs() > 0.0001
        {
            self.volume_expected = snapshot.volume;
            self.volume_slider
                .update(cx, |s, cx| s.set_value(snapshot.volume, window, cx));
        }
        self.scroll_handles.retain(|key, _| {
            snapshot.columns.iter().any(|c| &c.identity == key)
                || snapshot
                    .playback_queue
                    .as_ref()
                    .is_some_and(|c| &c.identity == key)
        });
        for column in &snapshot.columns {
            self.scroll_handles
                .entry(column.identity.clone())
                .or_default();
        }
        if snapshot.player.active
            && snapshot.player_navigation_serial != self.snapshot.player_navigation_serial
            && let Some(queue) = &snapshot.playback_queue
        {
            self.scroll_handles
                .entry(queue.identity.clone())
                .or_default()
                .scroll_to_item(queue.selected, ScrollStrategy::Nearest);
        }
        let jump_changed = snapshot.navigation_origin == browser::Origin::LibraryJump
            && (snapshot.navigation_serial != self.snapshot.navigation_serial
                || snapshot.columns.len() != self.snapshot.columns.len()
                || snapshot.columns.iter().zip(&self.snapshot.columns).any(
                    |(current, previous)| {
                        current.identity != previous.identity
                            || current.selected != previous.selected
                    },
                ));
        if (jump_changed || restoring) && self.role == WindowRole::Library {
            self.queue_focused = false;
            self.focus.focus(window, cx);
        }
        if snapshot.navigation_serial != self.snapshot.navigation_serial || restoring {
            self.pending_centered_rows.clear();
        }
        if restoring {
            self.scroll_pin = None;
            if let Some((_, offsets)) = self
                .jump_viewports
                .iter()
                .find(|(id, _)| Some(*id) == snapshot.jump_restore)
            {
                for (key, offset) in offsets {
                    self.scroll_handles
                        .entry(key.clone())
                        .or_default()
                        .set_offset(*offset);
                }
            }
        } else if jump_changed {
            self.scroll_pin = None;
            for (index, column) in snapshot
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| !column.hidden && !column.metadata)
            {
                let row = if column.album_info_selected {
                    0
                } else {
                    column.selected + filter::header_count(&snapshot, index)
                };
                self.pending_centered_rows
                    .insert(column.identity.clone(), row);
            }
        } else if self.snapshot.filter.active
            && !self.snapshot.filter.query.trim().is_empty()
            && (!snapshot.filter.active || snapshot.filter.query.trim().is_empty())
            && snapshot.generation == self.snapshot.generation
        {
            self.scroll_pin = None;
            for (index, column) in snapshot.columns.iter().enumerate() {
                if column.hidden || column.metadata {
                    continue;
                }
                let was_filtered = self
                    .snapshot
                    .columns
                    .iter()
                    .position(|previous| previous.identity == column.identity)
                    .is_some_and(|previous| filter::indices(&self.snapshot, previous).is_some());
                if was_filtered {
                    let row = if column.album_info_selected {
                        0
                    } else {
                        column.selected + filter::header_count(&snapshot, index)
                    };
                    self.pending_centered_rows
                        .insert(column.identity.clone(), row);
                }
            }
        } else if snapshot.navigation_serial != self.snapshot.navigation_serial
            && matches!(
                snapshot.navigation_origin,
                browser::Origin::Keyboard | browser::Origin::AlphabetJump
            )
            && !(snapshot.filter.active
                && !self.snapshot.filter.active
                && snapshot.filter.query.is_empty())
        {
            self.scroll_pin = None;
            if let Some(column) = snapshot.columns.get(snapshot.focused_column)
                && let Some(handle) = self.scroll_handles.get(&column.identity)
            {
                let selected = filter::indices(&snapshot, snapshot.focused_column)
                    .and_then(|indices| indices.iter().position(|row| *row == column.selected))
                    .unwrap_or(column.selected);
                let row = if column.album_info_selected {
                    0
                } else {
                    selected + filter::header_count(&snapshot, snapshot.focused_column)
                };
                if snapshot.navigation_origin == browser::Origin::AlphabetJump {
                    handle.scroll_to_item_strict(row, ScrollStrategy::Center);
                } else {
                    handle.scroll_to_item(row, ScrollStrategy::Nearest);
                }
            }
        }
        if returning_from_text
            && let Some(anchors) = snapshot
                .presentation_anchors
                .as_ref()
                .filter(|anchors| anchors.generation == snapshot.generation)
        {
            for anchor in &anchors.columns {
                let Some(column) = snapshot.columns.get(anchor.column) else {
                    continue;
                };
                let Some(original) = column
                    .rows
                    .iter()
                    .position(|row| row.key == anchor.item_key)
                else {
                    continue;
                };
                let display = filter::indices(&snapshot, anchor.column)
                    .and_then(|indices| indices.iter().position(|&index| index == original))
                    .unwrap_or(original)
                    + filter::header_count(&snapshot, anchor.column);
                let handle = self
                    .scroll_handles
                    .entry(column.identity.clone())
                    .or_default();
                handle.set_offset(point(
                    px(0.),
                    -px((display as f32 + anchor.fraction) * column.row_height()),
                ));
                let selected = filter::indices(&snapshot, anchor.column)
                    .and_then(|indices| indices.iter().position(|&index| index == column.selected))
                    .unwrap_or(column.selected)
                    + filter::header_count(&snapshot, anchor.column);
                handle.scroll_to_item(selected, ScrollStrategy::Nearest);
                if anchor.pointer_pinned {
                    self.scroll_pin = Some(column.identity.clone());
                }
            }
        }
        use gpui_kit::component::scroll::ScrollbarHandle;
        let now = std::time::Instant::now();
        for (identity, handle) in &self.scroll_handles {
            let offset = handle.offset().y;
            if let Some((last, previous)) =
                self.scroll_activity.insert(identity.clone(), (now, offset))
            {
                let seconds = now.duration_since(last).as_secs_f32();
                let distance = f32::from(offset - previous).abs();
                if distance > 60.
                    && seconds > 0.
                    && distance / seconds > 650.
                    && snapshot
                        .columns
                        .iter()
                        .any(|c| c.identity == *identity && c.alphabet_ascending.is_some())
                {
                    self.letter_shortcuts = Some((
                        identity.clone(),
                        now + std::time::Duration::from_millis(1400),
                    ));
                }
            }
        }
        self.scroll_activity
            .retain(|key, _| self.scroll_handles.contains_key(key));
        if self
            .letter_shortcuts
            .as_ref()
            .is_some_and(|(_, until)| now > *until)
        {
            self.letter_shortcuts = None;
        }
        let mut workflow_windows = self.windows.workflows.borrow_mut();
        if snapshot.separate_player && self.role == WindowRole::Library {
            if let Some(previous) = self.overlay.begin_dismissal() {
                let _ = previous.update(cx, |_, window, cx| window.close_dialog(cx));
                self.overlay = OverlayHost::Closed;
            }
            workflow_windows.retain(|id, handle| {
                if snapshot.surfaces.iter().any(|model| &model.id == id) {
                    true
                } else {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    false
                }
            });
            for model in snapshot
                .surfaces
                .iter()
                .filter(|model| model.save_file.is_none())
            {
                if !workflow_windows.contains_key(&model.id) {
                    match surface_view::open_native(
                        model.clone(),
                        self.engine.commands.clone(),
                        self.engine.snapshot.clone(),
                        cx,
                    ) {
                        Ok(handle) => {
                            workflow_windows.insert(model.id.clone(), handle);
                        }
                        Err(error) => tracing::error!(%error,"Open workflow window"),
                    }
                }
            }
            if (snapshot.surface_request != self.snapshot.surface_request
                || snapshot.surface.as_ref().map(|model| &model.id)
                    != self.snapshot.surface.as_ref().map(|model| &model.id))
                && let Some(handle) = snapshot
                    .surface
                    .as_ref()
                    .and_then(|model| workflow_windows.get(&model.id))
            {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
        } else if !snapshot.separate_player {
            for (_, handle) in workflow_windows.drain() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            let surface_id = snapshot
                .surface
                .as_ref()
                .filter(|model| model.save_file.is_none())
                .map(|model| model.id.clone());
            let reopen_requested =
                surface_id.is_some() && snapshot.surface_request != self.snapshot.surface_request;
            if surface_id.as_ref() != self.overlay.id() || reopen_requested {
                if let Some(previous) = self.overlay.begin_dismissal() {
                    if previous == window.window_handle() {
                        window.close_dialog(cx);
                    } else {
                        let _ = previous.update(cx, |_, window, cx| window.close_dialog(cx));
                    }
                }
                if let Some(model) = snapshot
                    .surface
                    .clone()
                    .filter(|model| model.save_file.is_none())
                {
                    let commands = self.engine.commands.clone();
                    let updates = self.engine.snapshot.clone();
                    self.overlay = OverlayHost::Visible {
                        id: model.id.clone(),
                        window: window.window_handle(),
                    };
                    surface_view::open_overlay(
                        cx.entity().downgrade(),
                        model,
                        commands,
                        updates,
                        window,
                        cx,
                    );
                }
                if surface_id.is_none() {
                    self.overlay = OverlayHost::Closed;
                }
            }
        }
        drop(workflow_windows);
        if returning_from_text {
            self.windows.activate_handoff_window(handoff_window, cx);
        }
        self.snapshot = snapshot;
        cx.notify();
    }
    fn jump_letter(&mut self, letter: char, second: bool) {
        let column = self.snapshot.focused_column;
        if let Some(c) = self.snapshot.columns.get(column) {
            let _ = self.engine.commands.try_send(Command::JumpLetter {
                generation: self.snapshot.generation,
                column,
                identity: c.identity.clone(),
                letter,
                second,
            });
        }
    }
    fn reset_view(&mut self, cx: &mut Context<Self>) {
        let _ = self.engine.commands.try_send(Command::Action(
            crate::app::action::NavigationAction::ResetDesktopPanels.into(),
        ));
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
        self.column_layout = None;
        self.column_scroll = ScrollHandle::new();
        self.sidebar_scroll = ScrollHandle::new();
        self.letter_shortcuts = None;
        cx.notify();
    }
    fn command(&self, command: PaletteCommandKind) {
        if let Err(error) = self.engine.commands.try_send(Command::Palette(command)) {
            tracing::error!(%error, "Desktop command rejected");
        }
    }
}
impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        if self.snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Text)
            && let Some((_, surface)) = &self.text_surface
        {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .bg(cx.theme().background)
                .child(
                    gpui_kit::component::TitleBar::new().child(
                        div()
                            .font_family("Menlo")
                            .text_size(px(14.))
                            .truncate()
                            .child(
                                self.snapshot
                                    .text_frame
                                    .as_ref()
                                    .map(|frame| frame.breadcrumbs.join(" › "))
                                    .unwrap_or_default(),
                            ),
                    ),
                )
                .child(div().flex_1().min_h_0().child(surface.clone()))
                .into_any_element();
        }
        self.sample_column_motion(window, cx);
        let mini = matches!(self.role, WindowRole::Library | WindowRole::NowPlaying)
            && self.snapshot.player.active
            && player::is_mini(window.viewport_size());
        if !player::is_mini(window.viewport_size()) {
            self.library_size = window.viewport_size();
        }
        let single = self.role != WindowRole::Queue
            && !self.snapshot.separate_player
            && !self.snapshot.split_player
            && !mini;
        let workspace = if self.role == WindowRole::Queue {
            self.queue_panel(cx)
        } else if self.snapshot.player.fullscreen {
            self.fullscreen_workspace(cx)
        } else if self.role == WindowRole::Visualizer {
            self.player_content(false, cx)
        } else if mini {
            self.mini_player(window, cx)
        } else if single {
            self.single_player_workspace(window, cx)
        } else {
            self.workspace(window, cx).into_any_element()
        };
        let mut transport =
            (self.role != WindowRole::Queue && !mini && !single && !self.snapshot.player.active)
                .then(|| self.transport(None, cx).into_any_element());
        let workspace =
            if self.role == WindowRole::Library && !mini && !self.snapshot.player.fullscreen {
                let workspace = div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(workspace))
                    .when_some(transport.take(), |view, bar| view.child(bar))
                    .into_any_element();
                self.queue_dock(workspace, window, cx)
            } else {
                workspace
            };
        let titlebar = if matches!(self.role, WindowRole::Queue | WindowRole::Visualizer) {
            gpui_kit::component::TitleBar::new()
                .child(if self.role == WindowRole::Visualizer {
                    "Visualizer"
                } else {
                    "Play Queue"
                })
                .into_any_element()
        } else if mini {
            gpui_kit::component::TitleBar::new()
                .child(
                    Button::new("mini-expand")
                        .ghost()
                        .small()
                        .icon(gpui_kit::component::Icon::new(
                            gpui_kit::assets::IconName::Maximize2,
                        ))
                        .tooltip("Show Full Now Playing")
                        .on_click(
                            cx.listener(|this, _, window, _| window.resize(this.library_size)),
                        ),
                )
                .into_any_element()
        } else {
            self.titlebar(cx).into_any_element()
        };
        self.drawn_presentation = self.snapshot.presentation.identity();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|_, _: &Minimize, window, cx| {
                window.minimize_window();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                if !this.snapshot.separate_player {
                    cx.quit();
                    cx.stop_propagation();
                    return;
                }
                if this.role == WindowRole::NowPlaying {
                    let _ = this
                        .engine
                        .commands
                        .try_send(Command::PlayerWindowVisible(false));
                } else if this.role == WindowRole::Queue {
                    let _ = this.engine.commands.try_send(Command::QueueWindowClosed);
                }
                window.remove_window();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &ResetView, _, cx| {
                this.reset_view(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, item: &ItemCommand, _, _| {
                let _ = this.engine.commands.try_send(Command::Select {
                    target: item.target.clone(),
                    mode: browser::Selection::Context,
                    origin: browser::Origin::Pointer,
                    intent: item.intent,
                });
            }))
            .on_action(cx.listener(|this, action: &NativeCommand, _, cx| {
                if action.0 == NativeCommandKind::CloseColumn {
                    this.command(PaletteCommandKind::CloseColumn);
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, key: &CoreShortcut, _, _| {
                use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
                let code = match key.key.as_str() {
                    "up" => KeyCode::Up,
                    "down" => KeyCode::Down,
                    "left" => KeyCode::Left,
                    "right" => KeyCode::Right,
                    "escape" => KeyCode::Esc,
                    "delete" => KeyCode::Delete,
                    "backspace" => KeyCode::Backspace,
                    "home" => KeyCode::Home,
                    "end" => KeyCode::End,
                    "pageup" => KeyCode::PageUp,
                    "pagedown" => KeyCode::PageDown,
                    "f1" => KeyCode::F(1),
                    "f2" => KeyCode::F(2),
                    "f3" => KeyCode::F(3),
                    "f4" => KeyCode::F(4),
                    "f5" => KeyCode::F(5),
                    "f6" => KeyCode::F(6),
                    "f7" => KeyCode::F(7),
                    _ => match key.key.chars().next() {
                        Some(c) => KeyCode::Char(c),
                        None => return,
                    },
                };
                if code == KeyCode::Esc && this.role == WindowRole::Queue {
                    let _ = this.engine.commands.try_send(Command::QueueWindowClosed);
                    return;
                }
                if code == KeyCode::Esc && this.queue_focused {
                    let _ = this.engine.commands.try_send(Command::ToggleQueue);
                    return;
                }
                if code == KeyCode::Esc && this.snapshot.player.active {
                    let _ = this.engine.commands.try_send(Command::ExitPlayer);
                    return;
                }
                let mut modifiers = KeyModifiers::NONE;
                if key.shift {
                    modifiers |= KeyModifiers::SHIFT;
                }
                if key.control {
                    modifiers |= KeyModifiers::CONTROL;
                }
                let _ = this.engine.commands.try_send(
                    if this.snapshot.player.active || this.queue_focused {
                        Command::PlayerKey(KeyEvent::new(code, modifiers))
                    } else {
                        Command::CoreKey(KeyEvent::new(code, modifiers))
                    },
                );
            }))
            .on_action(cx.listener(|this, command: &ArtistContextCommand, _, _| {
                let _ = this.engine.commands.try_send(Command::ArtistContext {
                    target: command.target.clone(),
                    biography: command.biography,
                });
            }))
            .on_action(
                cx.listener(|this, command: &DescriptionContextCommand, _, _| {
                    let _ = this.engine.commands.try_send(Command::GuardedActions {
                        generation: command.target.generation,
                        target: Some(command.target.clone()),
                        actions: vec![
                            crate::app::sources::audiomuse::Command::RemoveDescription {
                                query: command.target.key.clone(),
                            }
                            .into(),
                        ],
                    });
                }),
            )
            .on_action(cx.listener(|this, command: &TrackContextCommand, _, _| {
                let _ = this.engine.commands.try_send(Command::TrackContext {
                    target: command.target.clone(),
                    label: command.label.clone(),
                });
            }))
            .on_action(cx.listener(|this, _: &Activate, _, _| {
                if this.snapshot.player.active || this.queue_focused {
                    let _ = this.engine.commands.try_send(Command::PlayerKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Enter,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    return;
                }
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: 1,
                    row_delta: 0,
                });
            }))
            .on_action(cx.listener(|this, _: &ActivateFollowing, _, _| {
                if this.snapshot.player.active || this.queue_focused {
                    let _ = this.engine.commands.try_send(Command::PlayerKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Enter,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    return;
                }
                let _ = this
                    .engine
                    .commands
                    .try_send(Command::Activate { following: true });
            }))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.snapshot.details_focused
                    && !this.snapshot.player.active
                    && !this.queue_focused
                {
                    let offset = this.details_scroll.offset();
                    this.details_scroll
                        .set_offset(point(offset.x, (offset.y + px(36.)).min(px(0.))));
                    cx.notify();
                    return;
                }
                if this.snapshot.player.active || this.queue_focused {
                    let _ = this.engine.commands.try_send(Command::PlayerKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Up,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    return;
                }
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: 0,
                    row_delta: -1,
                });
            }))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.snapshot.details_focused
                    && !this.snapshot.player.active
                    && !this.queue_focused
                {
                    let offset = this.details_scroll.offset();
                    this.details_scroll
                        .set_offset(point(offset.x, (offset.y + px(-36.)).min(px(0.))));
                    cx.notify();
                    return;
                }
                if this.snapshot.player.active || this.queue_focused {
                    let _ = this.engine.commands.try_send(Command::PlayerKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Down,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    return;
                }
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: 0,
                    row_delta: 1,
                });
            }))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| {
                if this.queue_focused || this.role == WindowRole::Queue {
                    this.queue_focused = false;
                    cx.notify();
                    return;
                }
                if this.snapshot.player.active {
                    let count = crate::app::state::VisualizerTab::ALL.len();
                    let mode = (this.snapshot.player.mode.max(1) + count - 2) % count + 1;
                    let _ = this.engine.commands.try_send(Command::PlayerMode(mode));
                    return;
                }
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: -1,
                    row_delta: 0,
                });
            }))
            .on_action(cx.listener(|this, _: &MoveRight, _, _| {
                if this.queue_focused || this.role == WindowRole::Queue {
                    return;
                }
                if this.snapshot.player.active {
                    let mode =
                        this.snapshot.player.mode % crate::app::state::VisualizerTab::ALL.len() + 1;
                    let _ = this.engine.commands.try_send(Command::PlayerMode(mode));
                    return;
                }
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: 1,
                    row_delta: 0,
                });
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.snapshot.player.fullscreen
                    && !window.has_active_dialog(cx)
                    && event.keystroke.modifiers == Modifiers::default()
                {
                    let forward = match event.keystroke.key.as_str() {
                        "left" | "up" => Some(false),
                        "right" | "down" => Some(true),
                        _ => None,
                    };
                    if let Some(forward) = forward {
                        this.command(PaletteCommandKind::DesktopFullscreen(
                            this.snapshot.player.viewer.cycle(forward),
                        ));
                        cx.stop_propagation();
                        return;
                    }
                }

                if !this.focus.is_focused(window)
                    || window.has_active_dialog(cx)
                    || event.keystroke.modifiers != Modifiers::default()
                {
                    return;
                }
                let direction = match event.keystroke.key.as_str() {
                    "up" => -1,
                    "down" => 1,
                    _ => return,
                };
                if this.snapshot.player.active || this.queue_focused {
                    return;
                }
                if this.snapshot.details_focused {
                    let offset = this.details_scroll.offset();
                    this.details_scroll.set_offset(point(
                        offset.x,
                        (offset.y - px(direction as f32 * 36.)).min(px(0.)),
                    ));
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                let count = if this.snapshot.sidebar_cursor.is_some() {
                    this.snapshot.sidebar.len()
                } else {
                    this.snapshot
                        .columns
                        .get(this.snapshot.focused_column)
                        .map_or(0, |c| c.rows.len())
                };
                let steps =
                    this.arrow_repeat
                        .steps(direction, event.is_held, std::time::Instant::now());
                this.rapid_navigation = event.is_held && steps > 1 && count > 20;
                let row_delta = direction
                    * if count > 20 {
                        steps.min((count / 8).clamp(1, 15) as i8)
                    } else {
                        1
                    };
                let _ = this.engine.commands.try_send(Command::Navigate {
                    column_delta: 0,
                    row_delta,
                });
                window.prevent_default();
                cx.stop_propagation();
            }))
            .on_key_up(cx.listener(|this, _, _, cx| {
                this.arrow_repeat = Default::default();
                this.rapid_navigation = false;
                cx.notify();
            }))
            .track_focus(&self.focus)
            .key_context("Textamp")
            .on_action(keyboard::next_focus)
            .on_action(keyboard::previous_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.snapshot.player.fullscreen
                    && event.keystroke.modifiers == Modifiers::default()
                    && let Ok(number) = event.keystroke.key.parse::<usize>()
                    && (1..=7).contains(&number)
                {
                    let _ = this
                        .engine
                        .commands
                        .try_send(Command::PlayerMode(number - 1));
                    cx.stop_propagation();
                    return;
                }
                if !window.has_active_dialog(cx)
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    let text = event
                        .keystroke
                        .key_char
                        .as_deref()
                        .unwrap_or(&event.keystroke.key);
                    if this.snapshot.player.active
                        && !this.queue_focused
                        && this.role != WindowRole::Queue
                        && let Some(mode) = text
                            .chars()
                            .next()
                            .and_then(|c| c.to_digit(10))
                            .filter(|d| (1..=6).contains(d))
                    {
                        let _ = this
                            .engine
                            .commands
                            .try_send(Command::PlayerMode(mode as usize));
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action(cx.listener(|this, action: &LetterJump, _, _| {
                if let Some(letter) = action.letter.chars().next() {
                    this.jump_letter(letter, action.second);
                }
            }))
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Undo, _, _| {
                    this.command(PaletteCommandKind::UndoQueueEdit);
                }),
            )
            .on_action(cx.listener(|this, _: &Palette, window, cx| this.open_palette(window, cx)))
            .on_action(
                cx.listener(|this, _: &Library, _, _| {
                    this.command(PaletteCommandKind::GotoLibrary)
                }),
            )
            .on_action(cx.listener(|this, _: &NowPlaying, _, _| {
                this.command(PaletteCommandKind::GotoNowPlaying)
            }))
            .on_action(cx.listener(|this, _: &PlayPause, _, _| {
                let _ = this
                    .engine
                    .commands
                    .try_send(Command::Action(PlaybackAction::TogglePlayPause.into()));
            }))
            .relative()
            .text_size(px(13.))
            .child(titlebar)
            // Constrain every workspace to the space left by the fixed chrome.
            // Now Playing's full-height panels must not push transport offscreen.
            .child(
                div()
                    .id("main-workspace")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(workspace),
            )
            .when_some(transport, |root, transport| root.child(transport))
            .when(!mini && self.role != WindowRole::Queue, |root| {
                root.child(self.status_strip(cx))
            })
            .into_any_element()
    }
}
pub fn run() -> anyhow::Result<()> {
    run_requested(crate::app::launch::Arguments::parse(
        std::env::args().skip(1),
        true,
    )?)
}

/// Run one native application session with an explicit or remembered presentation.
/// # Errors
/// Propagates profile-lock, backend and native-window initialization failures.
pub fn run_requested(arguments: crate::app::launch::Arguments) -> anyhow::Result<()> {
    let _lock = crate::util::ProcessLock::acquire()?;
    let _logging = crate::util::setup_logging(arguments.verbose);
    tracing::info!("Starting desktop engine");
    let engine = Engine::start_mode(arguments.no_audio, arguments.mode)?;
    let application = application()
        .with_assets(assets::AllAssets)
        .with_quit_mode(gpui::QuitMode::Explicit);
    let engine = std::sync::Arc::new(engine);
    let windows = WindowRegistry::default();
    let reopen_engine = engine.clone();
    let reopen_windows = windows.clone();
    application.on_reopen(move |cx| {
        let _ = open_desktop_window(
            reopen_engine.clone(),
            reopen_windows.clone(),
            WindowRole::Library,
            cx,
        );
    });
    application.run(move |cx| {
        tracing::info!("Initializing GPUI");
        init(cx);
        gpui_kit::component::Theme::set_scrollbar_mode(
            gpui_kit::component::scroll::ScrollbarMode::Always,
            cx,
        );
        theme::apply(engine.snapshot.borrow().theme, None, cx);
        cx.bind_keys([
            KeyBinding::new(
                "cmd-c",
                gpui_kit::component::input::Copy,
                Some("TextampText"),
            ),
            KeyBinding::new(
                "cmd-x",
                gpui_kit::component::input::Cut,
                Some("TextampText"),
            ),
            KeyBinding::new(
                "cmd-v",
                gpui_kit::component::input::Paste,
                Some("TextampText"),
            ),
            KeyBinding::new(
                "cmd-a",
                gpui_kit::component::input::SelectAll,
                Some("TextampText"),
            ),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, Some("Textamp && !Input")),
            KeyBinding::new(
                "cmd-z",
                gpui_kit::component::input::Undo,
                Some("Textamp && !Input"),
            ),
            KeyBinding::new(
                "escape",
                gpui_kit::base::actions::Cancel,
                Some("TextampSurface"),
            ),
            KeyBinding::new(
                "escape",
                surface_view::DismissPalette,
                Some("TextampPalette > Input"),
            ),
            KeyBinding::new(
                "escape",
                surface_view::DismissPalette,
                Some("TextampPalette > Command"),
            ),
            KeyBinding::new("cmd-w", settings::CloseSettings, Some("TextampSettings")),
            KeyBinding::new(
                "cmd-s",
                settings::SaveConnectionForm,
                Some("TextampSettings"),
            ),
            KeyBinding::new(
                "enter",
                Activate,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            KeyBinding::new(
                "cmd-enter",
                ActivateFollowing,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            KeyBinding::new("up", MoveUp, Some("Textamp && !TextampSlider && !Input")),
            KeyBinding::new(
                "down",
                MoveDown,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            KeyBinding::new(
                "left",
                MoveLeft,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            KeyBinding::new(
                "right",
                MoveRight,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            KeyBinding::new(
                "cmd-[",
                NativeCommand(NativeCommandKind::CloseColumn),
                Some("Textamp && !TextampSurface && !Input"),
            ),
            KeyBinding::new(":", Palette, Some("Textamp && !Input")),
            KeyBinding::new("space", PlayPause, Some("Textamp && !Input")),
        ]);
        keyboard::init(cx);
        let context = Some("Textamp && !TextampSlider && !TextampSurface && !Input");
        for (binding, letter, second) in [
            ("a", "a", false),
            ("shift-a", "a", true),
            ("b", "b", false),
            ("shift-b", "b", true),
            ("c", "c", false),
            ("shift-c", "c", true),
            ("d", "d", false),
            ("shift-d", "d", true),
            ("e", "e", false),
            ("shift-e", "e", true),
            ("f", "f", false),
            ("shift-f", "f", true),
            ("g", "g", false),
            ("shift-g", "g", true),
            ("h", "h", false),
            ("shift-h", "h", true),
            ("i", "i", false),
            ("shift-i", "i", true),
            ("j", "j", false),
            ("shift-j", "j", true),
            ("k", "k", false),
            ("shift-k", "k", true),
            ("l", "l", false),
            ("shift-l", "l", true),
            ("m", "m", false),
            ("shift-m", "m", true),
            ("n", "n", false),
            ("shift-n", "n", true),
            ("o", "o", false),
            ("shift-o", "o", true),
            ("p", "p", false),
            ("shift-p", "p", true),
            ("q", "q", false),
            ("shift-q", "q", true),
            ("r", "r", false),
            ("shift-r", "r", true),
            ("s", "s", false),
            ("shift-s", "s", true),
            ("t", "t", false),
            ("shift-t", "t", true),
            ("u", "u", false),
            ("shift-u", "u", true),
            ("v", "v", false),
            ("shift-v", "v", true),
            ("w", "w", false),
            ("shift-w", "w", true),
            ("x", "x", false),
            ("shift-x", "x", true),
            ("y", "y", false),
            ("shift-y", "y", true),
            ("z", "z", false),
            ("shift-z", "z", true),
            ("#", "#", false),
            ("%", "%", false),
        ] {
            cx.bind_keys([KeyBinding::new(
                binding,
                LetterJump {
                    letter: letter.into(),
                    second,
                },
                context,
            )]);
        }
        for (binding, key, shift, control) in [
            ("ctrl-l", "l", false, true),
            ("ctrl-g", "g", false, true),
            ("ctrl-o", "o", false, true),
            ("ctrl-n", "n", false, true),
            ("ctrl-f", "f", false, true),
            ("ctrl-p", "p", false, true),
            ("ctrl-e", "e", false, true),
            ("ctrl-shift-e", "e", true, true),
            ("ctrl-a", "a", false, true),
            ("ctrl-s", "s", false, true),
            ("ctrl-x", "x", false, true),
            ("ctrl-j", "j", false, true),
            ("ctrl-r", "r", false, true),
            ("ctrl-m", "m", false, true),
            ("ctrl-v", "v", false, true),
            (",", ",", false, false),
            ("[", "[", false, false),
            ("]", "]", false, false),
            ("^", "^", false, false),
            ("f2", "f2", false, false),
            ("f3", "f3", false, false),
            ("cmd-l", "l", false, true),
            ("cmd-g", "g", false, true),
            ("cmd-shift-o", "o", false, true),
            ("cmd-n", "n", false, true),
            ("cmd-e", "e", false, true),
            ("cmd-shift-e", "e", true, true),
            ("cmd-a", "a", false, true),
            ("cmd-s", "s", false, true),
            ("cmd-x", "x", false, true),
            ("cmd-j", "j", false, true),
            ("delete", "delete", false, false),
            ("backspace", "backspace", false, false),
            ("shift-up", "up", true, false),
            ("shift-down", "down", true, false),
            ("shift-left", "left", true, false),
            ("shift-right", "right", true, false),
            ("cmd-shift-up", "up", true, true),
            ("cmd-shift-down", "down", true, true),
            ("home", "home", false, false),
            ("end", "end", false, false),
            ("pageup", "pageup", false, false),
            ("pagedown", "pagedown", false, false),
            ("escape", "escape", false, false),
            ("/", "/", false, false),
            ("f1", "f1", false, false),
            ("f4", "f4", false, false),
            ("f5", "f5", false, false),
            ("f6", "f6", false, false),
            ("<", "<", false, false),
            (">", ">", false, false),
        ] {
            cx.bind_keys([KeyBinding::new(
                binding,
                CoreShortcut {
                    key: key.into(),
                    shift,
                    control,
                },
                context,
            )]);
        }
        cx.bind_keys([
            KeyBinding::new("cmd-f", NativeCommand(NativeCommandKind::OpenSearch), None),
            KeyBinding::new("ctrl-cmd-f", ToggleNativeFullscreen, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
        ]);
        menu_catalog::bind_shortcuts(cx);
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.on_action(|_: &Minimize, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, _| window.minimize_window());
                });
            }
        });
        cx.on_action(|_: &ToggleNativeFullscreen, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, _| window.toggle_fullscreen());
                });
            }
        });
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, _| window.remove_window());
                });
            }
        });
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(native_menus_for_snapshot(&engine.snapshot.borrow()));
        let settings_only = std::env::args().any(|arg| arg == "--settings");
        let (_, desktop) =
            open_desktop_window(engine.clone(), windows.clone(), WindowRole::Library, cx)
                .expect("open Textamp");
        monitor_windows(engine.clone(), windows, desktop.clone(), cx);

        if settings_only {
            desktop.update(cx, |desktop, cx| desktop.open_settings(cx));
        }
        let radio_commands = desktop.clone();
        let cover_engine = engine.clone();
        cx.on_action(move |_: &LargeAlbumCovers, _| {
            let _ = cover_engine
                .commands
                .try_send(Command::ToggleLargeAlbumCovers);
        });
        cx.on_action(move |action: &RadioCommand, cx| {
            radio_commands.update(cx, |desktop, _| {
                let _ = desktop
                    .engine
                    .commands
                    .try_send(Command::RadioByName(action.name.clone()));
            });
        });
        let visualizer_engine = engine.clone();
        let visualizer_windows = desktop.read(cx).windows.clone();
        cx.on_action(move |_: &Visualizer, cx| {
            if visualizer_engine.snapshot.borrow().separate_player
                && visualizer_engine.snapshot.borrow().presentation.active()
                    == Some(crate::app::launch::WindowedMode::Gui)
            {
                let _ = open_desktop_window(
                    visualizer_engine.clone(),
                    visualizer_windows.clone(),
                    WindowRole::Visualizer,
                    cx,
                );
            }
        });
        let cycle_commands = engine.commands.clone();
        cx.on_action(move |_: &CycleApplicationMode, _| {
            let _ =
                cycle_commands.try_send(Command::Palette(PaletteCommandKind::CycleApplicationMode));
        });
        let players_engine = engine.clone();
        cx.on_action(move |_: &player_devices::Players, _| {
            let generation = players_engine.snapshot.borrow().generation;
            let _ = players_engine.commands.try_send(Command::OpenLibraryRoot {
                generation,
                root: crate::app::state::LibraryRoot::Mp3Players,
            });
        });
        let catalog_engine = engine.clone();
        cx.on_action(move |action: &menu_catalog::Invoke, _cx| {
            let _ = catalog_engine
                .commands
                .try_send(Command::Catalog(action.key.clone()));
        });
        let native_engine = engine.clone();
        cx.on_action(move |native: &menu_catalog::Native, cx| {
            let snapshot = native_engine.snapshot.borrow().clone();
            let raw = if snapshot.menu_commands.is_empty() {
                native_menus(&snapshot.radio_commands)
            } else {
                menu_catalog::menus(&snapshot.menu_commands)
            };
            if !native.available(cx.try_global::<menu_catalog::Origin>()) {
                return;
            }
            let Some(action) = menu_catalog::native_action(&raw, native) else {
                return;
            };
            let _ = native_engine
                .commands
                .try_send(Command::RememberPalette(native.key()));
            let palette = snapshot
                .surface
                .as_ref()
                .filter(|surface| surface.palette)
                .or_else(|| snapshot.surfaces.iter().find(|surface| surface.palette));
            let origin = if palette.is_some() {
                cx.try_global::<menu_catalog::Origin>().cloned()
            } else {
                None
            };
            if let Some(palette) = palette {
                let _ = native_engine
                    .commands
                    .try_send(Command::CloseSurface(palette.id.clone()));
            }
            cx.defer(move |cx| {
                if let Some(origin) = origin {
                    let _ = origin.window.update(cx, |_, window, cx| {
                        if window.has_active_dialog(cx) {
                            window.close_dialog(cx);
                        }
                        window.activate_window();
                        if let Some(focus) = origin.focus {
                            focus.focus(window, cx);
                        }
                        window.dispatch_action(action, cx);
                    });
                } else {
                    cx.dispatch_action(action.as_ref());
                }
            });
        });
        let desktop_commands = desktop.clone();
        cx.on_action(move |action: &NativeCommand, cx| {
            desktop_commands.update(cx, |desktop, _| {
                desktop.command(action.command());
            });
        });
        let layout = desktop.clone();
        cx.on_action(move |_: &ResetView, cx| {
            layout.update(cx, |desktop, cx| desktop.reset_view(cx));
        });
        let navigation = desktop.clone();
        cx.on_action(move |_: &Library, cx| {
            let desktop = navigation.clone();
            cx.defer(move |cx| {
                desktop.update(cx, |desktop, cx| {
                    let _ = open_desktop_window(
                        desktop.engine.clone(),
                        desktop.windows.clone(),
                        WindowRole::Library,
                        cx,
                    );
                    desktop.command(PaletteCommandKind::GotoLibrary)
                });
            });
        });
        let navigation = desktop.clone();
        cx.on_action(move |_: &NowPlaying, cx| {
            let desktop = navigation.clone();
            cx.defer(move |cx| {
                desktop.update(cx, |desktop, cx| {
                    let _ = open_desktop_window(
                        desktop.engine.clone(),
                        desktop.windows.clone(),
                        if desktop.engine.snapshot.borrow().separate_player {
                            WindowRole::NowPlaying
                        } else {
                            WindowRole::Library
                        },
                        cx,
                    );
                    desktop.command(PaletteCommandKind::GotoNowPlaying)
                });
            });
        });
        let desktop_settings = desktop.clone();
        cx.on_action(move |_: &Settings, cx| {
            let desktop = desktop_settings.clone();
            cx.defer(move |cx| {
                desktop.update(cx, |desktop, cx| {
                    if desktop.snapshot.presentation.active()
                        == Some(crate::app::launch::WindowedMode::Text)
                    {
                        desktop.command(PaletteCommandKind::OpenSettings);
                    } else {
                        desktop.open_settings(cx);
                    }
                });
            });
        });
        let desktop_for_palette = desktop.clone();
        cx.on_action(move |_: &Palette, cx| {
            let desktop = desktop_for_palette.clone();
            let window = cx.active_window();
            cx.defer(move |cx| {
                if let Some(window) = window {
                    let _ = window.update(cx, |_, window, cx| {
                        let view = gpui_kit::base::Root::read(window, cx)
                            .view()
                            .clone()
                            .downcast::<Desktop>();
                        if let Ok(view) = view.clone() {
                            view.update(cx, |desktop, cx| desktop.open_palette(window, cx));
                        } else {
                            desktop.update(cx, |desktop, cx| desktop.open_palette(window, cx));
                        }
                    });
                } else {
                    desktop.update(cx, |desktop, _| {
                        let _ = desktop.engine.commands.try_send(Command::OpenPalette {
                            query: String::new(),
                            player: false,
                        });
                    });
                }
            });
        });
        tracing::info!("Desktop window created");
        cx.activate(true);
    });
    Ok(())
}
