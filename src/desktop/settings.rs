//! Kit settings content hosted according to the application window mode.
use super::connection::Draft;
use super::*;
use crate::app::settings_transfer::Progress;
use crate::app::sources::{
    LibraryChoice,
    dialogs::{ServerForm, WebdavForm},
};
use crate::library::FolderLocation;
use gpui_kit::component::{
    Disableable, IndexPath, Sizable, TitleBar, WindowExt,
    alert::Alert,
    button::ButtonVariants,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants},
    input::{Input, InputState},
    list::{List, ListEvent, ListState},
};
use tokio::sync::watch;
use uuid::Uuid;

gpui_kit::actions!(desktop_settings, [CloseSettings, SaveConnectionForm]);
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Server,
    Webdav,
    Local,
}

#[derive(Clone, Copy)]
enum RestoreReason {
    Initial,
    Resume,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsHost {
    Dialog,
    Window,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestOwnership {
    Editor,
    Transferred,
    Closed,
}

pub(super) struct ConnectionSettings {
    registration: std::rc::Weak<std::cell::RefCell<Option<super::SettingsWindow>>>,
    request_ownership: RequestOwnership,
    host: SettingsHost,
    owner: Uuid,
    draft_id: String,
    kind: Kind,
    selected: Option<LibraryChoice>,
    fields: [Entity<InputState>; 8],
    analysis: bool,
    saved: bool,
    editor_open: bool,
    commands: super::engine::CommandPort,
    snapshot: Snapshot,
    phase: Option<Progress>,
    error: Option<String>,
    focus: FocusHandle,
    category_index: usize,
    category_navigation: bool,
    about_taps: u8,
    activity_open: bool,
    settings_revision: u64,
    restoring: Option<Task<()>>,
    theme_list: Entity<ListState<super::theme_preview::Choices>>,
    _theme_subscription: Subscription,
    _refresh: Task<()>,
}
impl Desktop {
    pub(super) fn settings_host_needs_update(&self, snapshot: &Snapshot, cx: &App) -> bool {
        self.windows
            .settings_editor
            .borrow()
            .as_ref()
            .is_some_and(|(_, editor)| {
                matches!(editor.read(cx).host, SettingsHost::Window) != snapshot.separate_player
            })
    }
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let registry = self.windows.clone();
        cx.defer(move |cx| open_settings(engine, registry, cx));
    }
}

fn open_settings(engine: std::sync::Arc<Engine>, registry: super::WindowRegistry, cx: &mut App) {
    let commands = engine.commands.clone();
    let updates = engine.snapshot.clone();
    let host = if updates.borrow().separate_player {
        SettingsHost::Window
    } else {
        SettingsHost::Dialog
    };
    let existing = registry.settings_editor.borrow().clone();
    if let Some((handle, editor)) = existing {
        let mut reuse = false;
        let _ = handle.update(cx, |_, window, cx| {
            let old_host = editor.read(cx).host;
            let available = editor.read(cx).request_ownership != RequestOwnership::Closed;
            if available && old_host == host {
                window.activate_window();
                reuse = true;
            } else if available {
                if let Err(error) = editor.update(cx, |editor, cx| editor.handoff(window, cx)) {
                    tracing::warn!(%error, "Keep Settings open while its draft cannot be queued");
                    reuse = true;
                    return;
                }
                match old_host {
                    SettingsHost::Dialog => window.close_dialog(cx),
                    SettingsHost::Window => window.remove_window(),
                }
            }
        });
        if reuse {
            return;
        }
        registry.settings_editor.borrow_mut().take();
        registry.borrow_mut()[super::SETTINGS_WINDOW] = None;
    }
    if !updates.borrow().separate_player {
        let handle = registry.borrow()[super::WindowRole::Library.index()];
        if let Some(handle) = handle {
            let _ = handle.update(cx, |_, window, cx| {
                let viewport = window.viewport_size();
                window.resize(super::surface_view::fit_window_size(
                    size(viewport.width.max(px(1000.)), viewport.height.max(px(850.))),
                    cx,
                ));
                let entity = cx.new(|cx| {
                    ConnectionSettings::new(
                        SettingsHost::Dialog,
                        std::rc::Rc::downgrade(&registry.settings_editor),
                        commands.clone(),
                        updates.clone(),
                        window,
                        cx,
                    )
                });
                *registry.settings_editor.borrow_mut() = Some((handle, entity.clone()));
                let first = std::cell::Cell::new(true);
                window.open_dialog(cx, move |dialog, window, cx| {
                    if first.replace(false) {
                        let focus = entity.read(cx).focus.clone();
                        focus.focus(window, cx);
                    }
                    let editor = entity.downgrade();
                    dialog
                        .title("Settings")
                        .w((window.viewport_size().width - px(32.)).min(px(900.)))
                        .margin_top(px(32.))
                        .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                        .on_close(move |_, _, cx| {
                            let _ = editor.update(cx, |editor, cx| editor.finish_close(cx));
                        })
                        .child(
                            div()
                                .h((window.viewport_size().height - px(150.))
                                    .min(px(720.))
                                    .max(px(180.)))
                                .child(entity.clone()),
                        )
                });
            });
        }
        return;
    }
    match open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                ..TitleBar::title_bar_options()
            }),
            window_bounds: Some(WindowBounds::Windowed(super::cascade_bounds(
                super::surface_view::fit_window_size(size(px(920.), px(820.)), cx),
                cx,
            ))),
            window_min_size: Some(size(px(680.), px(420.))),
            ..TitleBar::window_options()
        },
        cx,
        |window, cx| {
            let entity = cx.new(|cx| {
                ConnectionSettings::new(
                    SettingsHost::Window,
                    std::rc::Rc::downgrade(&registry.settings_editor),
                    commands,
                    updates,
                    window,
                    cx,
                )
            });
            let editor = entity.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                let _ = editor.update(cx, |editor, cx| editor.finish_close(cx));
                true
            });
            entity
        },
    ) {
        Ok((window, entity)) => {
            registry.borrow_mut()[super::SETTINGS_WINDOW] = Some(window);
            *registry.settings_editor.borrow_mut() = Some((window, entity));
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        Err(error) => tracing::error!(%error,"Open Settings"),
    }
}
impl ConnectionSettings {
    pub(super) fn handoff(&mut self, window: &mut Window, cx: &App) -> anyhow::Result<()> {
        let draft = self.stage(window, cx);
        self.commands
            .try_send(Command::StageSettings(draft))
            .map_err(|error| anyhow::anyhow!("Could not preserve Settings: {error}"))?;
        self.request_ownership = RequestOwnership::Transferred;
        Ok(())
    }
    fn new(
        host: SettingsHost,
        registration: std::rc::Weak<std::cell::RefCell<Option<super::SettingsWindow>>>,
        commands: super::engine::CommandPort,
        updates: watch::Receiver<Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        if host == SettingsHost::Window {
            focus.focus(window, cx);
        }
        let fields = std::array::from_fn(|i| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(i == 2 || i == 6)
                    .placeholder(match i {
                        0 => "https://music.example.com",
                        1 | 5 => "Username",
                        2 | 6 => "Password",
                        3 => "Library name (optional)",
                        4 => "https://analysis.example.com",
                        _ => "Navidrome",
                    })
            })
        });
        fields[7].update(cx, |field, cx| field.set_value("Navidrome", window, cx));
        let snapshot = updates.borrow().clone();
        let theme_list = cx.new(|cx| ListState::new(super::theme_preview::Choices, window, cx));
        theme_list.update(cx, |list, cx| {
            list.set_selected_index(
                crate::app::theme::ThemeName::all()
                    .iter()
                    .position(|name| *name == snapshot.theme)
                    .map(IndexPath::new),
                window,
                cx,
            )
        });
        let theme_subscription = cx.subscribe_in(&theme_list, window, |this, _, event, _, cx| {
            this.category_navigation = false;
            if let ListEvent::Confirm(index) = event
                && let Some(name) = crate::app::theme::ThemeName::all().get(index.row)
            {
                let _ = this.commands.try_send(Command::Preference {
                    generation: this.snapshot.generation,
                    setting: crate::app::state::TextampSetting::Theme(*name),
                });
            }
            cx.notify();
        });

        let _ = commands.try_send(Command::Action(
            crate::app::action::SettingsAction::RefreshCacheStats.into(),
        ));
        let mut updates = updates;
        let refresh = cx.spawn_in(window, async move |entity, cx| {
          while updates.changed().await.is_ok() {
            let snapshot = updates.borrow_and_update().clone();
            if cx.update(|window, cx| {
                    entity.update(cx, |this, cx| {
                            let previous=this.snapshot.presentation.active();
                            this.snapshot = snapshot;
                            if let Some(identity)=this.snapshot.presentation.identity() {this.commands.observe(identity);}

                            if previous==Some(crate::app::launch::WindowedMode::Text) && this.snapshot.presentation.active()==Some(crate::app::launch::WindowedMode::Gui) {
                                this.request_restore(RestoreReason::Resume, window, cx);
                            }

                            if let Some(status) = this
                                .snapshot
                                .connection
                                .as_ref()
                                .filter(|s| s.owner == this.owner)
                            {
                                let saved = status.phase == Progress::Saved
                                    && this.phase != Some(Progress::Saved);
                                this.phase = Some(status.phase.clone());
                                if saved {
                                    this.saved = true;
                                    if this.kind == Kind::Local && this.selected.is_none() {
                                        let path = this.fields[0].read(cx).value();
                                        this.selected = this.snapshot.libraries.iter().find(|choice| {
                                            matches!(choice, LibraryChoice::Folder(source) if matches!(&source.location,
                                                FolderLocation::Local { path: saved } if saved.to_string_lossy() == path.as_str()))
                                        }).cloned();
                                    }
                                    for i in [2, 6] {
                                        this.fields[i].update(cx, |field, cx| {
                                            field.set_value("", window, cx)
                                        });
                                    }
                                }
                            }
                            if this.phase==Some(Progress::Checking) {
                                match &this.snapshot.settings_progress {
                                    Some(crate::app::settings_transfer::Progress::Saved)=>{this.phase=Some(Progress::Saved);this.saved=true;},
                                    Some(crate::app::settings_transfer::Progress::Failed(error))=>this.phase=Some(Progress::Failed(error.clone())),
                                    _=>{},
                                }
                            }
                            if this.snapshot.presentation.active()!=Some(crate::app::launch::WindowedMode::Text) {cx.notify();}
                    })
                })
                .is_err()
            {
                break;
            }
          }
        });
        let mut editor = Self {
            registration,
            request_ownership: RequestOwnership::Editor,
            host,
            owner: Uuid::new_v4(),
            draft_id: Uuid::new_v4().to_string(),
            kind: Kind::Server,
            selected: None,
            fields,
            analysis: false,
            saved: false,
            editor_open: false,
            commands,
            snapshot,
            phase: None,
            error: None,
            focus,
            category_index: 0,
            category_navigation: true,
            about_taps: 0,
            activity_open: false,
            settings_revision: 0,
            restoring: None,
            theme_list,
            _theme_subscription: theme_subscription,
            _refresh: refresh,
        };
        editor.request_restore(RestoreReason::Initial, window, cx);
        editor
    }

    fn request_restore(
        &mut self,
        reason: RestoreReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (reply, receiver) = tokio::sync::oneshot::channel();
        if let Err(error) = self.commands.try_send(Command::ReadSettings(reply)) {
            self.error = Some(format!("Could not restore Settings: {error}"));
            cx.notify();
            return;
        }
        self.restoring = Some(cx.spawn_in(window, async move |entity, cx| {
            let Ok(draft) = receiver.await else {
                return;
            };
            let _ = cx.update(|window, cx| {
                entity.update(cx, |this, cx| {
                    if this.request_ownership != RequestOwnership::Editor {
                        return;
                    }
                    if let Some(draft) = draft {
                        this.restore_draft(draft, window, cx);
                    } else if matches!(reason, RestoreReason::Resume) {
                        this.close(window, cx);
                    }
                })
            });
        }));
    }

    fn choose(
        &mut self,
        choice: Option<LibraryChoice>,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self
            .commands
            .try_send(Command::CancelConnection(self.owner));
        self.category_index = 0;
        self.category_navigation = false;
        self.settings_revision += 1;
        self.owner = Uuid::new_v4();
        self.editor_open = true;
        self.selected = choice.clone();
        self.kind = kind;
        self.saved = false;
        self.phase = None;
        self.error = None;
        self.analysis = false;
        let mut values: [String; 8] = Default::default();
        self.draft_id = Uuid::new_v4().to_string();
        let analysis = choice
            .as_ref()
            .and_then(|c| c.artwork_key())
            .and_then(|key| self.snapshot.analysis_connections.get(&key))
            .cloned();
        match choice {
            Some(LibraryChoice::Navidrome { source, .. }) => {
                self.draft_id = source.id;
                values[0] = source.url;
                values[1] = source.username;
                values[3] = source.name;
            }
            Some(LibraryChoice::Folder(source)) => {
                self.draft_id = source.id;
                values[3] = source.name;
                match source.location {
                    FolderLocation::Local { path } => {
                        values[0] = path.to_string_lossy().into_owned()
                    }
                    FolderLocation::Webdav { url, username, .. } => {
                        values[0] = url;
                        values[1] = username.unwrap_or_default();
                    }
                }
            }
            _ => {}
        }
        values[7] = "Navidrome".into();
        if let Some(connection) = analysis {
            values[4] = connection.url;
            values[5] = connection.username;
            values[7] = connection.server;
            self.analysis = true;
        }
        for (index, (field, value)) in self.fields.iter().zip(values).enumerate() {
            field.update(cx, |field, cx| {
                field.set_value(value, window, cx);
                if index == 0 {
                    field.set_placeholder(
                        if kind == Kind::Local {
                            "/Users/you/Music"
                        } else {
                            "https://music.example.com"
                        },
                        window,
                        cx,
                    );
                }
            });
        }
        self.resize_editor(window, cx);
        self.fields[0].read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn resize_editor(&self, window: &mut Window, cx: &App) {
        if self.host == SettingsHost::Dialog {
            return;
        }
        let height = if !self.editor_open {
            788.
        } else if self.analysis {
            600.
        } else if self.kind == Kind::Local {
            420.
        } else {
            440.
        };
        let width = window.viewport_size().width.max(px(760.));
        window.resize(super::surface_view::fit_window_size(
            size(width, px(height + 32.)),
            cx,
        ));
    }
    fn finish_close(&mut self, cx: &mut Context<Self>) {
        if self.request_ownership == RequestOwnership::Closed {
            return;
        }
        let _ = self
            .commands
            .try_send(Command::CancelConnection(self.owner));
        let _ = self.commands.try_send(Command::SettingsClosed);
        self.request_ownership = RequestOwnership::Closed;
        if let Some(registration) = self.registration.upgrade() {
            let belongs_to_editor = registration
                .borrow()
                .as_ref()
                .is_some_and(|(_, editor)| editor.entity_id() == cx.entity().entity_id());
            if belongs_to_editor {
                registration.borrow_mut().take();
            }
        }
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_close(cx);
        match self.host {
            SettingsHost::Dialog => window.close_dialog(cx),
            SettingsHost::Window => window.remove_window(),
        }
    }
    fn cancel_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self
            .commands
            .try_send(Command::CancelConnection(self.owner));
        self.owner = Uuid::new_v4();
        self.editor_open = false;
        self.category_navigation = true;
        self.error = None;
        self.phase = None;
        for index in [2, 6] {
            self.fields[index].update(cx, |field, cx| field.set_value("", window, cx));
        }
        self.resize_editor(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn connection_draft(&self, cx: &App) -> Draft {
        let values: [String; 8] =
            std::array::from_fn(|i| self.fields[i].read(cx).value().to_string());
        match self.kind {
            Kind::Server => {
                let source = match &self.selected {
                    Some(LibraryChoice::Navidrome { source, .. }) => Some(source),
                    _ => None,
                };
                let mut form = ServerForm::new(source);
                form.source.id = self.draft_id.clone();
                for (field, value) in form.fields.iter_mut().zip(values) {
                    field.cursor = value.len();
                    field.value = value.into();
                }
                Draft::Server {
                    form: Box::new(form),
                    library: self.selected.clone(),
                }
            }
            Kind::Webdav => {
                let source = match &self.selected {
                    Some(LibraryChoice::Folder(source)) => Some(source),
                    _ => None,
                };
                let mut form = WebdavForm::new(source);
                form.source_id = self.draft_id.clone();
                form.editing = self.selected.is_some() || self.saved;
                for (i, field) in form.fields.iter_mut().enumerate() {
                    field.cursor = values[i].len();
                    field.value = values[i].clone().into();
                }
                Draft::Webdav(form)
            }
            Kind::Local => Draft::Local {
                path: values[0].clone(),
                name: values[3].clone(),
                existing_id: self.selected.as_ref().and_then(|choice| match choice {
                    LibraryChoice::Folder(source) => Some(source.id.clone()),
                    _ => None,
                }),
            },
        }
    }
    pub(super) fn stage(&self, window: &Window, cx: &App) -> crate::app::settings_transfer::Draft {
        use crate::app::sources::dialogs::Dialog;
        let focused = self
            .fields
            .iter()
            .position(|field| field.read(cx).focus_handle(cx).is_focused(window))
            .unwrap_or(0);
        let dialog = self.editor_open.then(|| match self.connection_draft(cx) {
            Draft::Server { mut form, .. } => {
                form.busy = self.phase == Some(Progress::Checking);
                form.focus = focused;
                if let Some(field) = form.fields.get_mut(focused) {
                    field.cursor = self.fields[focused]
                        .read(cx)
                        .cursor()
                        .min(field.value.len());
                    field.selected = self.fields[focused].read(cx).selected_range().len()
                        == field.value.len()
                        && !field.value.is_empty();
                }

                Dialog::Navidrome(*form)
            }
            Draft::Webdav(mut form) => {
                form.focus = focused.min(3);
                Dialog::Webdav(form)
            }
            Draft::Local {
                path,
                name,
                existing_id,
            } => {
                let mut form = crate::app::sources::dialogs::LocalForm {
                    existing_id,
                    focus: usize::from(focused == 3),
                    ..Default::default()
                };
                for (field, value) in form.fields.iter_mut().zip([path, name]) {
                    field.cursor = value.len();
                    field.value = value.into();
                }
                Dialog::Local(form)
            }
        });
        let category = self.category_index;
        crate::app::settings_transfer::Draft {
            progress: self.phase.clone().unwrap_or_default(),
            category,
            item: if self.category_index == 1 && !self.category_navigation {
                self.theme_list
                    .read(cx)
                    .selected_index()
                    .map_or(0, |index| index.row)
            } else {
                0
            },
            focus: if self.category_navigation {
                crate::app::state::SettingsFocus::Sections
            } else {
                crate::app::state::SettingsFocus::Content
            },
            dialog,
        }
    }
    fn restore_draft(
        &mut self,
        draft: crate::app::settings_transfer::Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::app::sources::dialogs::Dialog;
        self.request_ownership = RequestOwnership::Editor;
        self.phase = Some(draft.progress.clone());
        self.category_index = draft.category;
        if draft.category == 1 && draft.item < crate::app::theme::ThemeName::all().len() {
            self.theme_list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(draft.item)), window, cx)
            });
        }
        self.category_navigation = draft.focus == crate::app::state::SettingsFocus::Sections;
        self.settings_revision = self.settings_revision.wrapping_add(1);
        let mut values: [String; 8] = Default::default();
        let mut focused = 0;
        self.editor_open = true;
        match draft.dialog {
            Some(Dialog::Navidrome(form)) => {
                self.kind = Kind::Server;
                self.draft_id = form.source.id.clone();
                focused = form.focus.min(7);
                self.selected=self.snapshot.libraries.iter().find(|choice|matches!(choice,LibraryChoice::Navidrome {source,..} if source.id==form.source.id)).cloned();

                for (value, field) in values.iter_mut().zip(form.fields.iter()) {
                    *value = field.value.to_string();
                }
            }
            Some(Dialog::Webdav(form)) => {
                self.kind = Kind::Webdav;
                self.draft_id = form.source_id.clone();
                focused = form.focus.min(3);
                self.selected=self.snapshot.libraries.iter().find(|choice|matches!(choice,LibraryChoice::Folder(source) if source.id==form.source_id)).cloned();
                for (value, field) in values.iter_mut().zip(form.fields.iter()) {
                    *value = field.value.to_string();
                }
            }
            Some(Dialog::Local(form)) => {
                self.kind = Kind::Local;
                focused = form.focus.min(1);
                self.selected=self.snapshot.libraries.iter().find(|choice|matches!(choice,LibraryChoice::Folder(source) if Some(&source.id)==form.existing_id.as_ref())).cloned();
                values[0] = form.fields[0].value.to_string();
                values[3] = form.fields[1].value.to_string();
            }
            _ => self.editor_open = false,
        }
        for (field, value) in self.fields.iter().zip(values) {
            field.update(cx, |field, cx| field.set_value(value, window, cx));
        }
        if self.editor_open {
            self.fields[focused]
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.phase == Some(Progress::Checking) {
            return;
        }
        let draft = self.connection_draft(cx);
        match self.commands.try_send(Command::SaveConnection {
            owner: self.owner,
            draft,
        }) {
            Ok(()) => {
                self.phase = Some(Progress::Checking);
                self.error = None;
            }
            Err(_) => self.error = Some("Unable to submit the connection. Try again.".into()),
        }
        cx.notify();
    }
    fn field(&self, index: usize, label: &str, description: &str) -> Field {
        Field::new()
            .label(label.to_owned())
            .required(index == 0)
            .when(
                index == 2 && (self.selected.is_some() || self.saved),
                |field| field.description(description.to_owned()),
            )
            .child(Input::new(&self.fields[index]).disabled(
                self.phase == Some(Progress::Checking)
                    || (index == 0 && self.kind == Kind::Local && self.selected.is_some()),
            ))
    }
}
impl Drop for ConnectionSettings {
    fn drop(&mut self) {
        if self.request_ownership != RequestOwnership::Editor {
            return;
        }
        let _ = self
            .commands
            .try_send(Command::CancelConnection(self.owner));
    }
}
impl ConnectionSettings {
    fn editor(&self, cx: &mut Context<Self>) -> Form {
        let busy = self.phase == Some(Progress::Checking);
        let mut form = Form::horizontal()
            .small()
            .label_width(px(148.))
            .child(self.field(
                0,
                if self.kind == Kind::Local {
                    "Folder path"
                } else {
                    "Server URL"
                },
                "",
            ));
        if self.kind != Kind::Local {
            form = form.child(self.field(1, "Username", "")).child(self.field(
                2,
                "Password",
                "Leave blank to keep the saved password",
            ));
        }
        form = form.child(self.field(3, "Name", ""));
        if self.kind == Kind::Server {
            form = form.child(
                Field::new().label_indent(false).child(
                    Button::new("analysis-fields")
                        .ghost()
                        .small()
                        .label(if self.analysis {
                            "Hide AudioMuse connection"
                        } else {
                            "AudioMuse (optional)…"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.analysis = !this.analysis;
                            this.resize_editor(window, cx);
                            cx.notify();
                        })),
                ),
            );
            if self.analysis {
                for (i, label) in [
                    (4, "AudioMuse URL"),
                    (5, "AudioMuse username"),
                    (6, "AudioMuse password"),
                    (7, "Music-server name in AudioMuse"),
                ] {
                    form = form.child(self.field(i, label, ""));
                }
            }
        }
        if let Some(error) = self.error.clone().or_else(|| match &self.phase {
            Some(Progress::Failed(error)) => Some(error.clone()),
            _ => None,
        }) {
            form = form.child(
                Field::new()
                    .label_indent(false)
                    .child(Alert::error("connection-error", error).small()),
            );
        } else if self.phase == Some(Progress::Saved) {
            form = form.child(
                Field::new()
                    .label_indent(false)
                    .child(Alert::success("connection-saved", "Connection saved.").small()),
            );
        } else if busy {
            form = form.child(
                Field::new()
                    .label_indent(false)
                    .description("Checking connection…"),
            );
        }
        form =
            form.footer(
                gpui_kit::component::h_flex()
                    .gap_2()
                    .child(Button::new("cancel-connection").label("Cancel").on_click(
                        cx.listener(|this, _, window, cx| this.cancel_editor(window, cx)),
                    ))
                    .child(
                        Button::new("save-connection")
                            .primary()
                            .label(if busy {
                                "Checking…"
                            } else {
                                "Save Connection"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            );
        form
    }
}
fn section_heading(title: impl Into<SharedString>) -> gpui_kit::component::setting::SettingItem {
    let title = title.into();
    gpui_kit::component::setting::SettingItem::render(move |_, _, _| {
        gpui_kit::component::label::Label::new(title.clone())
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
    })
}

impl Render for ConnectionSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::setting::{
            SettingField, SettingGroup, SettingItem, SettingPage, Settings as KitSettings,
        };
        let owner = cx.entity().downgrade();
        let editor_owner = owner.clone();
        let editor = SettingItem::render(move |_, _, cx| {
            editor_owner
                .update(cx, |this, cx| this.editor(cx).into_any_element())
                .unwrap_or_else(|_| div().into_any_element())
        });
        let mut source_groups: Vec<(String, SettingGroup)> = Vec::new();
        for (index, choice) in self.snapshot.libraries.iter().enumerate() {
            let choice = choice.clone();
            let owner = owner.clone();
            let manage = choice.clone();
            let sender = self.commands.clone();
            let group = choice.group();
            let item = SettingItem::render(move |_, _, cx| {
                let selected = choice.clone();
                let owner = owner.clone();
                let manage = manage.clone();
                let sender = sender.clone();
                gpui_kit::component::h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(choice.label())
                            .when_some(
                                match &choice {
                                    LibraryChoice::Navidrome { source, .. }
                                        if source.libraries.len() > 1 =>
                                    {
                                        Some(
                                            source
                                                .libraries
                                                .iter()
                                                .map(|f| f.name.as_str())
                                                .collect::<Vec<_>>()
                                                .join(" · "),
                                        )
                                    }
                                    _ => None,
                                },
                                |view, folders| {
                                    view.child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(folders),
                                    )
                                },
                            ),
                    )
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .child(
                                Button::new(("edit-library", index))
                                    .small()
                                    .label("Edit Connection")
                                    .on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.choose(
                                                Some(selected.clone()),
                                                connection_kind(&selected),
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                            .child(
                                Button::new(("manage-library", index))
                                    .small()
                                    .label("Manage…")
                                    .on_click(move |_, _, _| {
                                        let _ = sender.try_send(Command::Action(
                                            crate::app::action::SettingsAction::ManageLibrary(
                                                manage.clone(),
                                            )
                                            .into(),
                                        ));
                                    }),
                            ),
                    )
            });
            if let Some((_, existing)) = source_groups.iter_mut().find(|(name, _)| name == &group) {
                *existing = std::mem::replace(existing, SettingGroup::new()).item(item);
            } else {
                source_groups.push((
                    group.clone(),
                    SettingGroup::new().item(section_heading(group)).item(item),
                ));
            }
        }
        let mut add = SettingGroup::new().item(section_heading("Add Library"));
        for (label, kind) in [
            ("Subsonic / Navidrome", Kind::Server),
            ("WebDAV", Kind::Webdav),
            ("Local Folder", Kind::Local),
        ] {
            let owner = owner.clone();
            add = add.item(SettingItem::render(move |_, _, _| {
                let owner = owner.clone();
                gpui_kit::component::h_flex().child(
                    Button::new(SharedString::from(format!("add-{label}")))
                        .small()
                        .label(label)
                        .on_click(move |_, window, cx| {
                            let _ =
                                owner.update(cx, |this, cx| this.choose(None, kind, window, cx));
                        }),
                )
            }));
        }
        let mut libraries = if self.editor_open {
            SettingPage::new("Libraries").resettable(false).group(
                SettingGroup::new()
                    .item(section_heading(if self.selected.is_some() {
                        "Edit Connection"
                    } else {
                        "Add Connection"
                    }))
                    .item(editor),
            )
        } else {
            SettingPage::new("Libraries")
                .resettable(false)
                .groups(source_groups.into_iter().map(|(_, group)| group))
                .group(add)
        };
        let settings = KitSettings::new(SharedString::from(format!(
            "textamp-settings-{}",
            self.settings_revision
        )))
        .default_selected_index(gpui_kit::component::setting::SelectIndex {
            page_ix: self.category_index,
            group_ix: None,
        })
        .on_select({
            let owner = owner.clone();
            move |index, _, cx| {
                let _ = owner.update(cx, |this, _| this.category_index = index.page_ix);
            }
        })
        .sidebar_width(px(200.));
        let mut pages = Vec::new();
        for page in &self.snapshot.preferences {
            let mut group = SettingGroup::new();
            let page_view = SettingPage::new(page.page.name()).resettable(false);
            if page.page == crate::app::state::SettingsPage::Sidebar {
                let sender = self.commands.clone();
                let limit = self.snapshot.singles_track_limit as f64;
                let mut sidebar_page = SettingPage::new("Sidebar").resettable(false).group(
                    SettingGroup::new().item(
                        SettingItem::new(
                            "Singles: maximum tracks",
                            SettingField::number_input(
                                gpui_kit::component::setting::NumberFieldOptions {
                                    min: 1.,
                                    max: crate::services::singles::MAX_LIMIT as f64,
                                    step: 1.,
                                },
                                move |_| limit,
                                move |value, _| {
                                    let _ = sender.try_send(Command::Action(
                                        crate::app::action::SettingsAction::SetSinglesTrackLimit(
                                            value as u32,
                                        )
                                        .into(),
                                    ));
                                },
                            ),
                        )
                        .description("Albums with this many tracks or fewer appear in Singles."),
                    ),
                );
                let mut groups: Vec<(&str, SettingGroup)> = Vec::new();
                for option in &self.snapshot.sidebar_options {
                    if groups
                        .last()
                        .is_none_or(|(title, _)| *title != option.group)
                    {
                        groups.push((
                            option.group,
                            SettingGroup::new().item(section_heading(option.group)),
                        ));
                    }
                    let sender = self.commands.clone();
                    let generation = self.snapshot.generation;
                    let target = option.target;
                    let checked = option.checked;
                    let field = SettingItem::new(
                        option.label,
                        SettingField::checkbox(
                            move |_| checked,
                            move |visible, _| {
                                use super::preferences::SidebarTarget;
                                let command = match target {
                                    SidebarTarget::Root(root) => Command::LibraryRootVisibility {
                                        generation,
                                        root,
                                        visible,
                                    },
                                    SidebarTarget::Section(section) => Command::SidebarVisibility {
                                        generation,
                                        section,
                                        visible,
                                    },
                                };
                                let _ = sender.try_send(command);
                            },
                        ),
                    );
                    let (title, group) = groups.pop().expect("sidebar option group");
                    groups.push((title, group.item(field)));
                }
                for (_, group) in groups {
                    sidebar_page = sidebar_page.group(group);
                }
                pages.push(sidebar_page);
                continue;
            }
            let mut choice_groups = std::collections::HashSet::new();
            for (index, item) in page.items.iter().enumerate() {
                let sender = self.commands.clone();
                let generation = self.snapshot.generation;
                let setting = item.id;
                let field = if matches!(item.id, crate::app::state::TextampSetting::Theme(_)) {
                    if !choice_groups.insert(item.group) {
                        continue;
                    }
                    let theme_list = self.theme_list.clone();
                    let name = if self.category_navigation {
                        self.snapshot.theme
                    } else {
                        theme_list
                            .read(cx)
                            .selected_index()
                            .and_then(|index| {
                                crate::app::theme::ThemeName::all().get(index.row).copied()
                            })
                            .unwrap_or(self.snapshot.theme)
                    };
                    SettingItem::render(move |_, _, _| {
                        gpui_kit::div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child("Theme")
                            .child(
                                gpui_kit::div()
                                    .h(px(
                                        crate::app::theme::ThemeName::all().len() as f32 * 28. + 8.
                                    ))
                                    .w_full()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        gpui_kit::div()
                                            .w(px(180.))
                                            .h_full()
                                            .child(List::new(&theme_list).scrollbar_visible(true)),
                                    )
                                    .child(super::theme_preview::sample(name)),
                            )
                    })
                } else if matches!(
                    item.value_kind,
                    crate::app::preferences::Value::Choice { .. }
                ) {
                    if !choice_groups.insert(item.group) {
                        continue;
                    }
                    let members: Vec<_> = page
                        .items
                        .iter()
                        .filter(|entry| {
                            entry.group == item.group
                                && matches!(
                                    entry.value_kind,
                                    crate::app::preferences::Value::Choice { .. }
                                )
                        })
                        .cloned()
                        .collect();
                    let choices = members
                        .iter()
                        .map(|entry| {
                            (
                                SharedString::from(entry.label.clone()),
                                SharedString::from(entry.label.clone()),
                            )
                        })
                        .collect();
                    let selected = SharedString::from(
                        members
                            .iter()
                            .find(|entry| entry.checked == Some(true))
                            .map(|entry| entry.label.clone())
                            .unwrap_or_default(),
                    );
                    SettingItem::new(
                        item.group,
                        SettingField::dropdown(
                            choices,
                            move |_| selected.clone(),
                            move |value, _| {
                                if let Some(entry) =
                                    members.iter().find(|entry| entry.label == value.as_ref())
                                {
                                    let _ = sender.try_send(Command::Preference {
                                        generation,
                                        setting: entry.id,
                                    });
                                }
                            },
                        ),
                    )
                } else if let Some(checked) = item.checked {
                    SettingItem::new(
                        item.label.clone(),
                        SettingField::checkbox(
                            move |_| checked,
                            move |_, _| {
                                let _ = sender.try_send(Command::Preference {
                                    generation,
                                    setting,
                                });
                            },
                        ),
                    )
                } else {
                    let value = item.value.clone();
                    SettingItem::new(
                        item.label.clone(),
                        SettingField::<SharedString>::element(
                            move |_: &gpui_kit::component::setting::RenderOptions,
                                  _: &mut Window,
                                  _: &mut App| {
                                let sender = sender.clone();
                                Button::new(("preference", index))
                                    .small()
                                    .label(value.clone())
                                    .on_click(move |_, _, _| {
                                        let _ = sender.try_send(Command::Preference {
                                            generation,
                                            setting,
                                        });
                                    })
                            },
                        ),
                    )
                }
                .description(item.description);
                group = group.item(field);
            }
            pages.push(page_view.group(group));
        }
        libraries=libraries.group(SettingGroup::new().item(section_heading("Cache Policy")).item(SettingItem::render(|_,_,_|div().child("Libraries open from cached metadata, refresh weekly, and can be refreshed manually. Scans index metadata; they do not download music."))));
        for (index, cache) in self.snapshot.caches.iter().enumerate() {
            let metadata = cache.metadata.clone();
            let artwork = cache.artwork.clone();
            let status = cache.status.clone();
            let mut group = SettingGroup::new()
                .item(section_heading(cache.choice.label()))
                .item(SettingItem::render(move |_, _, cx| {
                    GroupBox::new()
                        .fill()
                        .content_style(
                            gpui::StyleRefinement::default().bg(cx.theme().muted.opacity(0.5)),
                        )
                        .child(
                            gpui_kit::component::description_list::DescriptionList::horizontal()
                                .bordered(false)
                                .label_width(px(166.))
                                .columns(1)
                                .item("Metadata and analysis", metadata.clone(), 1)
                                .item("Downloaded artwork", artwork.clone(), 1)
                                .item("Scan status", status.clone(), 1),
                        )
                }));
            for (action, label) in cache.controls {
                let sender = self.commands.clone();
                let choice = cache.choice.clone();
                let generation = self.snapshot.generation;
                group = group.item(SettingItem::render(move |_, _, _| {
                    let sender = sender.clone();
                    let choice = choice.clone();
                    gpui_kit::component::h_flex().child(
                        Button::new(SharedString::from(format!("cache-{index}-{action:?}")))
                            .small()
                            .label(label)
                            .on_click(move |_, _, _| {
                                let _ = sender.try_send(Command::Cache {
                                    generation,
                                    choice: choice.clone(),
                                    action,
                                });
                            }),
                    )
                }));
            }
            libraries = libraries.group(group);
        }
        let sender = self.commands.clone();
        libraries = libraries.group(SettingGroup::new().item(SettingItem::render(
            move |_, _, _| {
                let sender = sender.clone();
                gpui_kit::component::h_flex().child(
                    Button::new("refresh-cache-sizes")
                        .small()
                        .label("Refresh cache sizes")
                        .on_click(move |_, _, _| {
                            let _ = sender.try_send(Command::Action(
                                crate::app::action::SettingsAction::RefreshCacheStats.into(),
                            ));
                        }),
                )
            },
        )));
        pages.push(
            SettingPage::new("About")
                .resettable(false)
                .group(SettingGroup::new().item(
                SettingItem::render(move |_, _, cx| {
                    let owner = owner.clone();
                    let history = owner.upgrade().and_then(|entity| {
                        let editor = entity.read(cx);
                        editor
                            .activity_open
                            .then(|| editor.snapshot.activity_history.clone())
                            .flatten()
                    });
                    let logo = super::about::content(cx);
                    div()
                        .flex()
                        .flex_col()
                        .child(div().id("activity-log-disclosure").child(logo).on_click(
                            move |_, _, cx| {
                                let _ = owner.update(cx, |editor, cx| {
                                    editor.about_taps += 1;
                                    if editor.about_taps == 4 {
                                        editor.about_taps = 0;
                                        editor.activity_open = true;
                                        let _ = editor.commands.try_send(Command::Action(
                                            crate::app::action::SettingsAction::ReadActivityHistory
                                                .into(),
                                        ));
                                        cx.notify();
                                    }
                                });
                            },
                        ))
                        .children(history.map(|text| div().child(text.to_string())))
                }),
            )),
        );
        // The resizable sidebar divider separates the two panes. Spacing
        // separates each page heading from the controls below it.
        let page_header = gpui::StyleRefinement::default().border_0().pb_1();
        let settings = settings.pages(
            std::iter::once(libraries)
                .chain(pages)
                .map(|page| page.header_style(&page_header)),
        );
        div()
            .debug_selector(|| "settings-content".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .track_focus(&self.focus)
            .key_context("TextampSettings")
            .on_action(super::keyboard::next_focus)
            .on_action(super::keyboard::previous_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "f10" => {
                        this.category_navigation = true;
                        this.focus.focus(window, cx);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "up" | "down"
                        if this.category_navigation
                            && !this
                                .theme_list
                                .read(cx)
                                .focus_handle(cx)
                                .contains_focused(window, cx) =>
                    {
                        this.category_index = if event.keystroke.key == "up" {
                            this.category_index.saturating_sub(1)
                        } else {
                            (this.category_index + 1)
                                .min(crate::app::state::SettingsState::CATEGORY_COUNT - 1)
                        };
                        this.settings_revision += 1;
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" | "right"
                        if this.category_navigation
                            && !this
                                .theme_list
                                .read(cx)
                                .focus_handle(cx)
                                .contains_focused(window, cx) =>
                    {
                        this.category_navigation = false;
                        window.focus_next(cx);
                        cx.stop_propagation();
                    }
                    "escape" if !window.has_active_dialog(cx) => {
                        if this.editor_open {
                            this.cancel_editor(window, cx);
                        } else if this.category_navigation {
                            this.close(window, cx);
                        } else {
                            this.category_navigation = true;
                            this.focus.focus(window, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .on_action(cx.listener(|this, _: &CloseSettings, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &super::CloseWindow, window, cx| {
                this.close(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|_, _: &super::Minimize, window, cx| {
                window.minimize_window();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &SaveConnectionForm, window, cx| {
                if this
                    .fields
                    .iter()
                    .any(|field| field.read(cx).focus_handle(cx).is_focused(window))
                {
                    this.save(cx);
                }
            }))
            .font_family(cx.theme().font_family.clone())
            .line_height(px(20.))
            .when(self.host == SettingsHost::Window, |view| {
                view.child(
                    TitleBar::new().child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
                )
            })
            .child(settings)
    }
}

fn connection_kind(choice: &LibraryChoice) -> Kind {
    match choice {
        LibraryChoice::Folder(s) if matches!(s.location, FolderLocation::Local { .. }) => {
            Kind::Local
        }
        LibraryChoice::Folder(_) => Kind::Webdav,
        _ => Kind::Server,
    }
}

#[cfg(all(test, feature = "desktop-tests"))]
mod lifecycle_tests {
    use super::{
        Command, ConnectionSettings, Engine, Kind, Progress, RequestOwnership, SettingsHost,
        Snapshot,
    };
    use gpui_kit::gpui::{self, AppContext, Entity, TestAppContext, VisualTestContext};

    struct Fixture<'a> {
        editor: Entity<ConnectionSettings>,
        cx: &'a mut VisualTestContext,
        commands: super::super::engine::CommandPort,
        inbox: tokio::sync::mpsc::Receiver<Command>,
        registry: super::super::WindowRegistry,
    }

    fn fixture(cx: &mut TestAppContext) -> Fixture<'_> {
        cx.update(super::super::init);
        let (engine, _updates, mut inbox) = Engine::fixture(Snapshot::default());
        let commands = engine.commands.clone();
        let registry = super::super::WindowRegistry::default();
        let registration = std::rc::Rc::downgrade(&registry.settings_editor);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                ConnectionSettings::new(
                    SettingsHost::Dialog,
                    registration,
                    engine.commands.clone(),
                    engine.snapshot.clone(),
                    window,
                    cx,
                )
            });
            saved.replace(Some(editor.clone()));
            gpui_kit::base::Root::new(editor, window, cx)
        });
        let editor = slot.borrow().clone().unwrap();
        *registry.settings_editor.borrow_mut() = Some((
            cx.update(|window, _| window.window_handle()),
            editor.clone(),
        ));
        while inbox.try_recv().is_ok() {}
        Fixture {
            editor,
            cx,
            commands,
            inbox,
            registry,
        }
    }

    #[gpui::test]
    fn theme_highlight_previews_without_saving_and_enter_applies(cx: &mut TestAppContext) {
        use crate::app::state::TextampSetting;
        let Fixture {
            editor,
            cx,
            mut inbox,
            ..
        } = fixture(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let state = crate::app::AppState::new();
                editor.snapshot.preferences = super::super::preferences::project(&state);
                editor.category_index = 1;
                editor.settings_revision += 1;
                editor.category_navigation = false;
                editor
                    .theme_list
                    .update(cx, |list, cx| list.focus(window, cx));
                cx.notify();
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("theme-preview").is_some());
        while inbox.try_recv().is_ok() {}
        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        let row = editor.read_with(cx, |editor, cx| {
            editor.theme_list.read(cx).selected_index().unwrap().row
        });
        assert!(
            !std::iter::from_fn(|| inbox.try_recv().ok())
                .any(|command| matches!(command, Command::Preference { .. }))
        );
        cx.simulate_keystrokes("enter");
        let expected = crate::app::theme::ThemeName::all()[row];
        assert!(std::iter::from_fn(|| inbox.try_recv().ok()).any(|command| matches!(command, Command::Preference { setting: TextampSetting::Theme(name), .. } if name == expected)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let target = cx.debug_bounds("theme-choice-3").unwrap().center();
        cx.simulate_click(target, Default::default());
        assert!(
            !std::iter::from_fn(|| inbox.try_recv().ok())
                .any(|command| matches!(command, Command::Preference { .. }))
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(target, Default::default());
        let expected = crate::app::theme::ThemeName::all()[3];
        assert!(std::iter::from_fn(|| inbox.try_recv().ok()).any(|command| matches!(command, Command::Preference { setting: TextampSetting::Theme(name), .. } if name == expected)));
    }

    #[gpui::test]
    fn settings_close_cancels_the_current_connection_and_releases_registration(
        cx: &mut TestAppContext,
    ) {
        let Fixture {
            editor,
            cx,
            mut inbox,
            registry,
            ..
        } = fixture(cx);
        let initial_owner = editor.read_with(cx, |editor, _| editor.owner);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.choose(None, Kind::Server, window, cx)
            })
        });
        let current_owner = editor.read_with(cx, |editor, _| editor.owner);
        assert_ne!(initial_owner, current_owner);
        while inbox.try_recv().is_ok() {}
        let weak = editor.downgrade();
        cx.update(|_, cx| {
            weak.update(cx, |editor, cx| editor.finish_close(cx))
                .unwrap()
        });
        let sent: Vec<_> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(sent.iter().any(
            |command| matches!(command, Command::CancelConnection(owner) if *owner == current_owner)
        ));
        assert!(!sent.iter().any(
            |command| matches!(command, Command::CancelConnection(owner) if *owner == initial_owner)
        ));
        assert!(registry.settings_editor.borrow().is_none());
    }

    #[gpui::test]
    fn settings_handoff_retains_draft_and_ownership_when_the_command_queue_is_full(
        cx: &mut TestAppContext,
    ) {
        let Fixture {
            editor,
            cx,
            commands,
            mut inbox,
            ..
        } = fixture(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.choose(None, Kind::Server, window, cx);
                editor.fields[0].update(cx, |field, cx| {
                    field.set_value("http://127.0.0.1:1234", window, cx)
                });
                editor.phase = Some(Progress::Checking);
            })
        });
        while inbox.try_recv().is_ok() {}
        while commands.try_send(Command::SettingsClosed).is_ok() {}
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.handoff(window, cx).is_err());
                assert!(editor.request_ownership == RequestOwnership::Editor);
                assert_eq!(
                    editor.fields[0].read(cx).value().as_str(),
                    "http://127.0.0.1:1234"
                );
            })
        });
        while inbox.try_recv().is_ok() {}
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.handoff(window, cx).unwrap()));
        let Command::StageSettings(draft) = inbox.try_recv().unwrap() else {
            panic!("Expected the retained Settings draft")
        };
        assert!(matches!(
            draft.progress,
            crate::app::settings_transfer::Progress::Checking
        ));
        let Some(crate::app::sources::dialogs::Dialog::Navidrome(form)) = draft.dialog else {
            panic!("Expected the staged server form")
        };
        assert_eq!(form.fields[0].value.as_str(), "http://127.0.0.1:1234");
    }
}
