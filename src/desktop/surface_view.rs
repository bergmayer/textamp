//! Kit workflow content hosted in temporary dialogs or persistent native windows.
use super::{
    engine::{Command, Snapshot},
    surface::{Operation, Surface, WindowLayout},
    *,
};
use gpui_kit::assets::IconName as Glyph;
use gpui_kit::component::{
    Disableable, Icon, Selectable, Sizable, TitleBar, WindowExt,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::ButtonVariants,
    checkbox::Checkbox,
    command::{Command as KitCommand, CommandItem, CommandState},
    description_list::DescriptionList,
    dialog::DialogEntrance,
    form::{Field, Form},
    input::{Input, InputEvent, InputState},
    list::ListItem,
    scroll::{ScrollableElement, Scrollbar},
    spinner::Spinner,
    tab::{Tab, TabBar},
};
use tokio::sync::watch;
mod radio_browser;

#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
struct ActivateSurfaceItem {
    index: usize,
}
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
pub(super) struct DismissPalette;

pub struct SurfaceView {
    model: Surface,
    embedded: bool,
    artwork_mode: crate::app::state::ArtworkMode,
    inline_detail: Option<Entity<SurfaceView>>,
    fields: Vec<Entity<InputState>>,
    commands: super::engine::CommandPort,
    updates: watch::Receiver<Snapshot>,
    palette: Entity<CommandState>,
    palette_query: String,
    palette_matches: Vec<super::palette::Match>,
    recent_commands: crate::app::command_history::History,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    help_section: usize,
    help_scroll: ScrollHandle,
    lyrics_scroll: ScrollHandle,
    radio_scroll: std::collections::HashMap<String, UniformListScrollHandle>,
    reveal_selection: bool,
    auto_fit: Option<Size<Pixels>>,
    _subscriptions: Vec<Subscription>,
    _refresh: Task<()>,
}
impl SurfaceView {
    pub(super) fn new(
        model: Surface,
        embedded: bool,
        commands: super::engine::CommandPort,
        updates: watch::Receiver<Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let mut subscriptions = Vec::new();
        let fields = model
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let input = cx.new(|cx| {
                    let mut state = InputState::new(window, cx)
                        .masked(field.masked)
                        .placeholder(
                            if matches!(field.label.as_str(), "Start track" | "End track") {
                                "Choose a track".into()
                            } else if field.label == "Search" {
                                "Search music".into()
                            } else {
                                field.label.clone()
                            },
                        );
                    if field.label == "Maximum tracks (5–100)" {
                        state = state.step(1.).min(5.).max(100.);
                    }
                    state.set_value(field.value.clone(), window, cx);
                    state
                });
                // Observe value changes as well as semantic events: Kit's native
                // accessibility edits update InputState without emitting Change.
                let mut previous = field.value.clone();
                subscriptions.push(cx.observe_in(&input, window, move |this, input, _, cx| {
                    let value = input.read(cx).value().to_string();
                    if value != previous {
                        previous = value.clone();
                        if this
                            .model
                            .fields
                            .get(index)
                            .is_some_and(|field| field.value == value)
                        {
                            return;
                        }
                        let _ = this.commands.try_send(Command::SurfaceEdit {
                            id: this.model.id.clone(),
                            field: index,
                            value,
                        });
                    }
                }));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, _input, event: &InputEvent, window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. })
                            && this.model.title != "Search"
                        {
                            this.flush_fields(cx);
                            this.send(Operation::Key(crossterm::event::KeyCode::Enter));
                            this.focus.focus(window, cx);
                        }
                    },
                ));
                input
            })
            .collect::<Vec<_>>();
        let palette = cx.new(|cx| {
            let mut state = CommandState::new(window, cx);
            state.set_query(model.query.clone(), window, cx);
            state
        });
        let recent_commands = updates.borrow().recent_commands.clone();
        let palette_query = model.query.clone();
        let palette_matches = super::palette::matches(
            &model.rows,
            &palette_query,
            &recent_commands,
            &model.palette_path,
            &model.disabled_rows,
        );
        let mut update_stream = updates.clone();
        let refresh = cx.spawn_in(window, async move |entity, cx| {
            while update_stream.changed().await.is_ok() {
                let snapshot = update_stream.borrow_and_update().clone();
                if cx
                    .update(|window, cx| {
                        entity.update(cx, |this, cx| {
                            this.artwork_mode = snapshot.artwork_mode;
                            let recent_changed = this.recent_commands != snapshot.recent_commands;
                            this.recent_commands = snapshot.recent_commands;
                            let model = if snapshot
                                .workspace
                                .as_ref()
                                .is_some_and(|model| model.id == this.model.id)
                            {
                                snapshot.workspace
                            } else if snapshot
                                .radio_browser
                                .as_ref()
                                .is_some_and(|model| model.id == this.model.id)
                            {
                                snapshot.radio_browser
                            } else if snapshot.separate_player {
                                snapshot
                                    .surfaces
                                    .iter()
                                    .find(|model| model.id == this.model.id)
                                    .cloned()
                            } else {
                                snapshot.surface
                            };
                            match model {
                                Some(mut model) if model.id == this.model.id => {
                                    if this.model.fixed_size && model.fixed_size {
                                        model.toggles =
                                            stable_choices(&this.model.toggles, &model.toggles);
                                    }
                                    if model.fields.len() != this.fields.len() {
                                        return;
                                    }
                                    let matches_changed = recent_changed
                                        || model.revision != this.model.revision
                                        || model.query != this.model.query;
                                    if model.palette && model.query != this.model.query {
                                        this.palette_query = model.query.clone();
                                        this.palette.update(cx, |s, cx| {
                                            s.set_query(model.query.clone(), window, cx)
                                        });
                                    }
                                    if model.title == "Journey" && !model.form_hidden {
                                        let active = model
                                            .endpoints
                                            .iter()
                                            .position(|endpoint| endpoint.selected);
                                        let previous = this
                                            .model
                                            .endpoints
                                            .iter()
                                            .position(|endpoint| endpoint.selected);
                                        if active != previous
                                            && let Some(input) =
                                                active.and_then(|index| this.fields.get(index))
                                        {
                                            input.read(cx).focus_handle(cx).focus(window, cx);
                                        }
                                    }
                                    if model.form_hidden && !this.model.form_hidden {
                                        this.focus.focus(window, cx);
                                    } else if !model.form_hidden
                                        && this.model.form_hidden
                                        && let Some(field) = this.fields.first()
                                    {
                                        field.read(cx).focus_handle(cx).focus(window, cx);
                                    }
                                    if let Some(previous) = this.auto_fit {
                                        let actual = window.viewport_size();
                                        if (actual.width - previous.width).abs() < px(2.)
                                            && (actual.height - previous.height).abs() < px(2.)
                                        {
                                            let preferred =
                                                fit_window_size(preferred_size(&model), cx);
                                            let grown = if model.title == "Journey" {
                                                preferred
                                            } else {
                                                size(
                                                    actual.width.max(preferred.width),
                                                    actual.height.max(preferred.height),
                                                )
                                            };
                                            if grown != actual {
                                                window.resize(grown);
                                            }
                                            this.auto_fit = Some(grown);
                                        } else {
                                            // Respect a user resize instead of fighting it on every refresh.
                                            this.auto_fit = None;
                                        }
                                    }
                                    for (index, field) in model.fields.iter().enumerate() {
                                        if field.label != this.model.fields[index].label {
                                            this.fields[index].update(cx, |input, cx| {
                                                input.set_placeholder(
                                                    field.label.clone(),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                        if !field.masked
                                            && field.value != this.model.fields[index].value
                                            && this.fields[index].read(cx).value().as_str()
                                                == this.model.fields[index].value
                                        {
                                            this.fields[index].update(cx, |input, cx| {
                                                input.set_value(field.value.clone(), window, cx)
                                            });
                                        }
                                    }
                                    if this.reveal_selection {
                                        if let Some((key, selected)) =
                                            model.radio.as_ref().and_then(|radio| {
                                                let selected = radio.selected.as_ref()?;
                                                radio.columns.iter().find_map(|column| {
                                                    column
                                                        .stations
                                                        .iter()
                                                        .position(|station| {
                                                            station.key == selected.key
                                                        })
                                                        .map(|index| (column.key.clone(), index))
                                                })
                                            })
                                        {
                                            this.radio_scroll
                                                .entry(key)
                                                .or_default()
                                                .scroll_to_item(selected, ScrollStrategy::Nearest);
                                        } else if let Some(index) =
                                            model.rows.iter().position(|r| r.selected)
                                        {
                                            this.scroll
                                                .scroll_to_item(index, ScrollStrategy::Nearest);
                                        }
                                        this.reveal_selection = false;
                                    }
                                    let selected_key = this
                                        .palette
                                        .read(cx)
                                        .selected_index()
                                        .and_then(|path| this.palette_matches.get(path.row))
                                        .and_then(|matched| this.model.rows.get(matched.index))
                                        .map(|row| row.key.clone());
                                    if let Some(lyrics) = &model.lyrics {
                                        let previous = this.model.lyrics.as_ref();
                                        let replaced = previous.is_none_or(|old| {
                                            old.track != lyrics.track
                                                || old.language != lyrics.language
                                        });
                                        if replaced
                                            || (lyrics.follow
                                                && lyrics.active
                                                    != previous.and_then(|old| old.active))
                                        {
                                            this.lyrics_scroll
                                                .scroll_to_item(lyrics.active.unwrap_or(0));
                                        }
                                    }
                                    this.model = model;
                                    if this.model.palette && matches_changed {
                                        this.palette_matches = super::palette::matches(
                                            &this.model.rows,
                                            &this.palette_query,
                                            &this.recent_commands,
                                            &this.model.palette_path,
                                            &this.model.disabled_rows,
                                        );
                                        let selected = selected_key
                                            .and_then(|key| {
                                                this.palette_matches.iter().position(|matched| {
                                                    this.model.rows[matched.index].key == key
                                                })
                                            })
                                            .unwrap_or(0);
                                        this.palette.update(cx, |state, cx| {
                                            state.set_selected_index(
                                                Some(gpui_kit::component::IndexPath::new(selected)),
                                                window,
                                                cx,
                                            )
                                        });
                                    }
                                    cx.notify();
                                }
                                _ => {} // Desktop owns dialog replacement; never close a newer dialog here.
                            }
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let artwork_mode = updates.borrow().artwork_mode;
        Self {
            model,
            embedded,
            artwork_mode,
            inline_detail: None,
            fields,
            commands,
            updates,
            palette,
            palette_query,
            palette_matches,
            recent_commands,
            focus,
            scroll: UniformListScrollHandle::new(),
            help_section: 0,
            help_scroll: ScrollHandle::new(),
            lyrics_scroll: ScrollHandle::new(),
            radio_scroll: Default::default(),
            reveal_selection: false,
            auto_fit: None,
            _subscriptions: subscriptions,
            _refresh: refresh,
        }
    }
    pub(super) fn matches(&self, model: &Surface) -> bool {
        self.model.id == model.id && self.fields.len() == model.fields.len()
    }
    pub(super) fn focus_initial(&self, window: &mut Window, cx: &mut App) {
        self.fields
            .first()
            .map(|input| input.read(cx).focus_handle(cx))
            .unwrap_or_else(|| self.focus.clone())
            .focus(window, cx);
    }
    fn flush_fields(&self, cx: &App) {
        for (field, input) in self.fields.iter().enumerate() {
            if self
                .model
                .fields
                .get(field)
                .is_some_and(|model| model.value == input.read(cx).value().as_str())
            {
                continue;
            }
            let _ = self.commands.try_send(Command::SurfaceEdit {
                id: self.model.id.clone(),
                field,
                value: input.read(cx).value().to_string(),
            });
        }
    }
    fn dismiss(&self, window: &mut Window, cx: &mut App) {
        if self
            .updates
            .borrow()
            .workspace
            .as_ref()
            .is_some_and(|model| model.id == self.model.id)
            || (self.model.radio.is_some()
                && self
                    .updates
                    .borrow()
                    .radio_browser
                    .as_ref()
                    .is_some_and(|model| model.id == self.model.id))
        {
            let _ = self
                .commands
                .try_send(Command::CloseSurface(self.model.id.clone()));
        } else if self.embedded {
            window.close_dialog(cx);
        } else {
            let _ = self
                .commands
                .try_send(Command::CloseSurface(self.model.id.clone()));
            window.remove_window();
        }
    }
    fn send(&self, operation: Operation) {
        if self.model.stale {
            return;
        }
        let _ = self.commands.try_send(Command::Surface {
            id: self.model.id.clone(),
            revision: self.model.revision,
            operation,
        });
    }
}
impl Render for SurfaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.model.radio.is_some() {
            return self.radio_browser(window, cx);
        }
        if self.model.palette {
            let owner = cx.entity().downgrade();
            let rows: Vec<_> = self
                .palette_matches
                .iter()
                .map(|matched| {
                    let item = &self.model.rows[matched.index];
                    let label = item.label.clone();
                    let highlights = matched.highlights.clone();
                    let shortcut = if let Operation::Palette(command) = &item.operation {
                        super::palette::shortcut(command)
                    } else {
                        None
                    };
                    let group = matches!(
                        item.operation,
                        Operation::Palette(
                            crate::app::state::PaletteCommandKind::OpenCommandGroup(_)
                        )
                    );
                    let back = group && item.label == "Back";
                    let context = (!self.palette_query.is_empty())
                        .then(|| super::palette::path(item).join(" › "))
                        .filter(|path| !path.is_empty());
                    CommandItem::new()
                        .label(item.label.clone())
                        .disabled(
                            self.model.stale
                                || self.model.disabled_rows.contains_key(&matched.index)
                                || matches!(&item.operation, Operation::Native(native) if !native.available(cx.try_global::<super::menu_catalog::Origin>())),
                        )
                        .action(match &item.operation {
                            Operation::Native(native) => {
                                Box::new(native.clone()) as Box<dyn gpui_kit::Action>
                            }
                            _ => Box::new(ActivateSurfaceItem {
                                index: matched.index,
                            }),
                        })
                        .child(move |_, cx| {
                            let text = gpui::StyledText::new(label.clone()).with_highlights(
                                highlights.iter().cloned().map(|range| {
                                    (
                                        range,
                                        gpui::HighlightStyle {
                                            font_weight: Some(FontWeight::BOLD),
                                            ..Default::default()
                                        },
                                    )
                                }),
                            );
                            div()
                                .debug_selector(|| "palette-command-row".into())
                                .flex_1()
                                .min_w_0()
                                .pr_4()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .when(back, |row| row.child(Icon::new(Glyph::ChevronLeft).small()))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .truncate()
                                        .child(text),
                                )
                                .when_some(context.clone(), |row, context| {
                                    row.child(
                                        div()
                                            .max_w(px(150.))
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(context),
                                    )
                                })
                                .when(group && !back, |row| {
                                    row.child(Icon::new(Glyph::ChevronRight).small())
                                })
                                .when_some(shortcut, |view, shortcut| {
                                    view.child(
                                        div()
                                            .debug_selector(|| "palette-shortcut".into())
                                            .flex_none()
                                            .w(px(88.))
                                            .flex()
                                            .justify_end()
                                            .overflow_hidden()
                                            .child(
                                                gpui_kit::component::kbd::Kbd::new(
                                                    gpui::Keystroke::parse(shortcut)
                                                        .expect("desktop shortcut"),
                                                )
                                                .appearance(false)
                                                .text_color(cx.theme().muted_foreground),
                                            ),
                                    )
                                })
                        })
                })
                .collect::<Vec<_>>();
            let confirmations: Vec<_> = self
                .palette_matches
                .iter()
                .map(|matched| {
                    (
                        matched.index,
                        match &self.model.rows[matched.index].operation {
                            Operation::Native(_) => 2,
                            Operation::Palette(crate::app::state::PaletteCommandKind::Quit) => 1,
                            _ => 0,
                        },
                    )
                })
                .collect();
            let commands = self.commands.clone();
            let id = self.model.id.clone();
            let revision = self.model.revision;
            let mut breadcrumb = Breadcrumb::new();
            for depth in 0..=self.model.palette_path.len() {
                let label = if depth == 0 {
                    "Commands".to_string()
                } else {
                    self.model.palette_path[depth - 1].clone()
                };
                let target = self.model.palette_path[..depth].to_vec();
                breadcrumb = breadcrumb.child(BreadcrumbItem::new(label).on_click(cx.listener(
                    move |this, _, _, _| {
                        let _ = this.commands.try_send(Command::Surface {
                            id: this.model.id.clone(),
                            revision: this.model.revision,
                            operation: Operation::Palette(
                                crate::app::state::PaletteCommandKind::OpenCommandGroup(
                                    target.clone(),
                                ),
                            ),
                        });
                    },
                )));
            }
            return div()
                .debug_selector(|| "palette-content".into())
                .when(!self.embedded, |view| {
                    view.size_full()
                        .flex()
                        .flex_col()
                        .bg(cx.theme().background)
                        .child(TitleBar::new())
                })
                .key_context("TextampPalette")
                .on_action(super::keyboard::next_focus)
                .on_action(super::keyboard::previous_focus)
                .on_action(cx.listener(|this, _: &super::CloseWindow, window, cx| {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|this, _: &DismissPalette, window, cx| {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }))
                .when(!self.model.palette_path.is_empty(), |view| {
                    view.child(div().px_3().py_2().child(breadcrumb))
                })
                .child(
                    KitCommand::new(&self.palette)
                        .placeholder("Execute a command…")
                        .filterable(false)
                        .max_h(px(420.))
                        .bordered(false)
                        .items(rows)
                        .on_query(move |query, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.palette_query = query.into();
                                this.palette_matches = super::palette::matches(
                                    &this.model.rows,
                                    query,
                                    &this.recent_commands,
                                    &this.model.palette_path,
                                    &this.model.disabled_rows,
                                );
                                cx.notify();
                            });
                        })
                        .footer(|_, _, cx| {
                            div()
                                .flex()
                                .justify_end()
                                .p_1p5()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .child(
                                    Button::new("run-palette-command")
                                        .ghost()
                                        .small()
                                        .label("Run")
                                        .child(
                                            gpui_kit::component::kbd::Kbd::new(
                                                gpui::Keystroke::parse("enter")
                                                    .expect("Return key"),
                                            )
                                            .appearance(false),
                                        )
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(
                                                Box::new(gpui_kit::base::actions::Confirm {
                                                    secondary: false,
                                                }),
                                                cx,
                                            )
                                        }),
                                )
                        })
                        .on_confirm(move |path, _, _cx| {
                            if let Some(&(index, kind)) = confirmations.get(path.row) {
                                if kind == 2 {
                                    return;
                                }
                                let _ = commands.try_send(Command::ExecutePalette {
                                    id: id.clone(),
                                    revision,
                                    index,
                                });
                            }
                        }),
                )
                .into_any_element();
        }
        let mut form = Form::horizontal().small().label_width(px(140.));
        let endpoint_form = matches!(
            self.model.title.as_str(),
            "Journey" | "Sonic Mix" | "Stay Close Mix"
        ) && !self.model.form_hidden;
        for (index, endpoint) in self
            .model
            .endpoints
            .iter()
            .enumerate()
            .filter(|_| !endpoint_form)
        {
            let operation = endpoint.operation.clone();
            form = form.child(
                Field::new()
                    .label(if index == 0 { "Start" } else { "End" })
                    .child(
                        Button::new(("adventure-endpoint", index))
                            .debug_selector(move || format!("adventure-endpoint-{index}"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .child(
                                        div().truncate().child(
                                            endpoint
                                                .label
                                                .lines()
                                                .next()
                                                .unwrap_or("Choose a track")
                                                .to_owned(),
                                        ),
                                    )
                                    .when_some(
                                        endpoint.label.lines().nth(1).map(str::to_owned),
                                        |view, artist| {
                                            view.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .truncate()
                                                    .child(artist),
                                            )
                                        },
                                    ),
                            )
                            .h_auto()
                            .min_h(px(42.))
                            .accessibility_label(format!(
                                "{}: {}",
                                if index == 0 {
                                    "Start track"
                                } else {
                                    "End track"
                                },
                                endpoint.label
                            ))
                            .selected(endpoint.selected)
                            .outline()
                            .bg(cx.theme().background)
                            .text_color(cx.theme().foreground)
                            .when(endpoint.selected, |button| {
                                button.border_color(cx.theme().primary)
                            })
                            .w_full()
                            .justify_start()
                            .small()
                            .disabled(self.model.stale)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send(operation.clone());
                                if let Some(input) = this.fields.first() {
                                    input.focus_handle(cx).focus(window, cx);
                                }
                            })),
                    ),
            );
        }
        for (index, field) in self
            .model
            .fields
            .iter()
            .enumerate()
            .filter(|_| !endpoint_form)
        {
            let search = !field.masked
                && (field.label.starts_with("Find ") || field.label.starts_with("Search"));
            form = form.child(
                Field::new()
                    .label(if field.label == "Maximum tracks (5–100)" {
                        "Maximum tracks".into()
                    } else {
                        field.label.clone()
                    })
                    .when(field.label == "Maximum tracks (5–100)", |field| {
                        field.description("5–100 tracks")
                    })
                    .label_indent(!field.label.is_empty() && !search)
                    .child(
                        Input::new(&self.fields[index])
                            .when(self.model.title == "Describe Music", |input| {
                                input.aria_label("Music description")
                            })
                            .when(field.label == "Maximum tracks (5–100)", |input| {
                                input.w(px(120.))
                            })
                            .when(search, |input| {
                                input
                                    .prefix(gpui_kit::component::Icon::new(
                                        gpui_kit::assets::IconName::Search,
                                    ))
                                    .aria_label(field.label.clone())
                                    .rounded(px(5.))
                                    .h(px(24.))
                                    .bg(cx.theme().secondary)
                                    .border_color(cx.theme().border.opacity(0.55))
                                    .cleanable(true)
                            })
                            .small()
                            .disabled(self.model.stale),
                    ),
            );
        }
        let mut footer = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .items_center()
            .gap_2();
        for (index, item) in self.model.buttons.iter().enumerate() {
            let op = item.operation.clone();
            footer = footer.child(
                Button::new(("surface-control", index))
                    .small()
                    .disabled(
                        self.model.stale
                            || (self.model.primary_disabled
                                && self.model.primary.as_deref() == Some(item.key.as_str())),
                    )
                    .label(item.label.clone())
                    .when(
                        self.model.primary.as_deref() == Some(item.key.as_str()),
                        |button| button.primary(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if matches!(op, Operation::Key(crossterm::event::KeyCode::Esc)) {
                            this.dismiss(window, cx);
                        } else {
                            this.flush_fields(cx);
                            this.send(op.clone());
                        }
                    })),
            );
        }
        let scroll = self.scroll.clone();
        let owner = cx.entity().downgrade();
        let rows = uniform_list(
            "surface-rows",
            self.model.rows.len(),
            move |range, _, cx| {
                owner
                    .update(cx, |this, cx| {
                        range
                            .filter_map(|index| {
                                let row = this.model.rows.get(index)?.clone();
                                let op = row.operation;
                                if this.model.multiple_choice {
                                    return Some(
                                        div()
                                            .h(row_height(&this.model))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .child(
                                                Checkbox::new(("artist-choice", index))
                                                    .label(row.label)
                                                    .checked(row.selected)
                                                    .on_click(cx.listener(move |this, _, _, _| {
                                                        this.send(op.clone())
                                                    })),
                                            )
                                            .into_any_element(),
                                    );
                                }
                                Some(
                                    ListItem::new(index)
                                        .bg(cx.theme().background)
                                        .role(Role::ListItem)
                                        .accessibility_label(row.label.clone())
                                        .aria_selected(row.selected)
                                        .w_full()
                                        .selected(row.selected)
                                        .when(row.selected, |item| {
                                            item.text_color(super::theme::selection_foreground(
                                                cx.theme().foreground,
                                                cx.theme().list_active,
                                            ))
                                        })
                                        .h(row_height(&this.model))
                                        .text_size(px(13.))
                                        .line_height(px(18.))
                                        .child(row.label)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.reveal_selection = false;
                                            this.send(op.clone());
                                            if this.model.title == "Journey"
                                                && !this.model.form_hidden
                                            {
                                                let next = if this
                                                    .model
                                                    .endpoints
                                                    .first()
                                                    .is_some_and(|endpoint| endpoint.selected)
                                                {
                                                    1
                                                } else {
                                                    0
                                                };
                                                if let Some(input) = this.fields.get(next) {
                                                    input
                                                        .read(cx)
                                                        .focus_handle(cx)
                                                        .focus(window, cx);
                                                }
                                            } else {
                                                this.focus.focus(window, cx);
                                            }
                                        }))
                                        .into_any_element(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            },
        )
        .track_scroll(&scroll)
        .h_full();
        let result_area = div().relative().size_full().child(rows).child(
            Scrollbar::vertical(&scroll)
                .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                .viewport_from_layout(),
        );
        let mut result_area = Some(result_area.into_any_element());
        let journey_form = if endpoint_form {
            let mut form = Form::vertical().small();
            for (index, endpoint) in self.model.endpoints.iter().enumerate() {
                let input = Input::new(&self.fields[index])
                    .small()
                    .prefix(gpui_kit::component::Icon::new(
                        gpui_kit::assets::IconName::Search,
                    ))
                    .aria_label(if index == 0 {
                        "Start track"
                    } else {
                        "End track"
                    })
                    .cleanable(true)
                    .disabled(self.model.stale);
                let mut field_content = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().w_full().max_w(px(560.)).child(input));
                if endpoint.selected && !self.model.rows.is_empty() {
                    field_content =
                        field_content.child(div().h(px(200.)).children(result_area.take()));
                } else if endpoint.selected
                    && self
                        .model
                        .fields
                        .get(index)
                        .is_some_and(|field| !field.value.is_empty())
                    && endpoint.label == "Choose a track"
                    && !self.model.loading
                {
                    field_content = field_content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("No matches. Try a track, artist or album name."),
                    );
                }
                form = form.child(
                    Field::new()
                        .label(if index == 0 {
                            "Start track"
                        } else {
                            "End track"
                        })
                        .when_some(
                            endpoint.label.lines().nth(1).map(str::to_owned),
                            |field, artist| field.description(artist),
                        )
                        .child(field_content),
                );
            }
            if self.model.title == "Journey" {
                form = form.child(
                    Field::new()
                        .label("Maximum tracks")
                        .description("5–100 tracks")
                        .child(Input::new(&self.fields[2]).small().w(px(100.))),
                );
            }
            Some(form.into_any_element())
        } else {
            None
        };
        let photo = self
            .model
            .image
            .as_ref()
            .and_then(|bytes| super::player::image_from_bytes(bytes, self.artwork_mode));
        let search_panel = self.model.title == "Search";
        let mut tabs =
            TabBar::new("surface-tabs").when(search_panel, |tabs| tabs.segmented().small());
        for item in &self.model.tabs {
            let op = item.operation.clone();
            tabs = tabs.child(
                Tab::new()
                    .label(if search_panel {
                        match item.label.as_str() {
                            "all" => "All",
                            "artists" => "Artists",
                            "albums" => "Albums",
                            "playlists" => "Playlists",
                            "tracks" => "Tracks",
                            "genres" => "Genres",
                            other => other,
                        }
                        .to_owned()
                    } else {
                        item.label.clone()
                    })
                    .selected(item.selected)
                    .on_click(cx.listener(move |this, _, _, _| this.send(op.clone()))),
            );
        }
        let (top_tabs, bottom_tabs) = if self.model.title == "Search" {
            (
                None,
                Some(
                    div()
                        .debug_selector(|| "search-scope-bar".into())
                        .flex()
                        .justify_center()
                        .child(tabs)
                        .into_any_element(),
                ),
            )
        } else {
            (Some(tabs.into_any_element()), None)
        };
        let mut help_tabs = TabBar::new("help-topics").small();
        for (index, section) in self.model.help.iter().enumerate() {
            help_tabs = help_tabs.child(
                Tab::new()
                    .label(section.title)
                    .selected(index == self.help_section)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.help_section = index;
                        this.help_scroll.set_offset(point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );
        }
        let mut shortcuts = DescriptionList::horizontal()
            .columns(1)
            .small()
            .bordered(false)
            .label_width(px(150.));
        if let Some(section) = self.model.help.get(self.help_section) {
            for &(shortcut, description) in section.rows {
                shortcuts = shortcuts.item(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(shortcut)
                        .into_any_element(),
                    description,
                    1,
                );
            }
        }
        let mut footer = Some(footer);
        let content = div()
            .text_size(px(13.))
            .id("native-workflow-content")
            .flex_1()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_5()
            .overflow_y_scrollbar()
            .track_focus(&self.focus)
            .key_context("TextampSurface")
            .on_action(super::keyboard::next_focus)
            .on_action(super::keyboard::previous_focus)
            .flex()
            .flex_col()
            .gap_3()
            .when(search_panel, |content| {
                content
                    .p_3()
                    .gap_2()
                    .when(self.embedded, |content| content.p_0())
            })
            .when(self.model.fixed_size, |content| {
                content
                    .p_3()
                    .gap_2()
                    .when(self.embedded, |content| content.px_0())
            })
            .when(self.model.fixed_size, |content| {
                let context = self
                    .model
                    .title
                    .strip_prefix("View Options · ")
                    .unwrap_or(&self.model.title)
                    .to_owned();
                content.child(
                    div()
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(context),
                )
            })
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.model.title == "Search"
                    && event.keystroke.key == "enter"
                    && this
                        .fields
                        .iter()
                        .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
                {
                    this.flush_fields(cx);
                    this.send(Operation::Actions(vec![
                        crate::app::action::SearchAction::SubmitSearch.into(),
                    ]));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.model.help.is_empty() {
                    let delta = match event.keystroke.key.as_str() {
                        "up" => 36.,
                        "down" => -36.,
                        "pageup" => 360.,
                        "pagedown" => -360.,
                        "home" => 100000.,
                        "end" => -100000.,
                        "left" | "right" => {
                            let count = this.model.help.len();
                            this.help_section = if event.keystroke.key == "left" {
                                (this.help_section + count - 1) % count
                            } else {
                                (this.help_section + 1) % count
                            };
                            this.help_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                            cx.stop_propagation();
                            return;
                        }
                        _ => return,
                    };
                    let offset = this.help_scroll.offset();
                    this.help_scroll
                        .set_offset(point(offset.x, (offset.y + px(delta)).min(px(0.))));
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                use crossterm::event::KeyCode;
                let code = match event.keystroke.key.as_str() {
                    "up" => KeyCode::Up,
                    "down" => KeyCode::Down,
                    "enter" => KeyCode::Enter,
                    "right" if this.model.id.contains(":sidebar-group:") => KeyCode::Right,
                    "left" if this.model.id.contains(":sidebar-group:") => KeyCode::Left,
                    "pageup" => KeyCode::PageUp,
                    "pagedown" => KeyCode::PageDown,
                    "home" => KeyCode::Home,
                    "end" => KeyCode::End,
                    _ => return,
                };
                this.reveal_selection = true;
                this.focus.focus(window, cx);
                this.send(Operation::Key(code));
                cx.stop_propagation();
            }))
            .when(!self.model.help.is_empty(), |view| {
                view.child(
                    div()
                        .flex_none()
                        .text_size(px(22.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Textamp Help"),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(cx.theme().muted_foreground)
                        .child("Keyboard shortcuts and controls for Textamp on Mac."),
                )
                .child(div().flex_none().child(help_tabs))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .id("help-shortcuts")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.help_scroll)
                                .pr_4()
                                .child(shortcuts),
                        )
                        .child(
                            Scrollbar::vertical(&self.help_scroll)
                                .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                                .viewport_from_layout(),
                        ),
                )
            })
            .when(
                matches!(
                    self.model.title.as_str(),
                    "Journey" | "Artist Mix" | "Sonic Mix" | "Stay Close Mix"
                ) && !self.model.text.is_empty(),
                |view| {
                    view.child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.model.text.clone()),
                    )
                },
            )
            .when_some(
                top_tabs.filter(|_| !self.model.tabs.is_empty()),
                |d, tabs| d.child(tabs),
            )
            .when(!self.model.selected_names.is_empty(), |view| {
                view.child(div().flex_none().flex().flex_wrap().gap_1().children(
                    self.model.selected_names.iter().map(|name| {
                        gpui_kit::component::tag::Tag::secondary()
                            .small()
                            .child(name.clone())
                    }),
                ))
            })
            .when(
                !self.model.fields.is_empty() && !self.model.form_hidden && !endpoint_form,
                |view| {
                    view.child(
                        div()
                            .id("surface-form")
                            .w_full()
                            .max_w(px(680.))
                            .flex_none()
                            .child(form),
                    )
                },
            )
            .when_some(journey_form, |view, form| view.child(form))
            .when(endpoint_form && !self.model.buttons.is_empty(), |view| {
                view.child(footer.take().unwrap().w_full().max_w(px(560.)).pt_2())
            })
            .when(!self.model.examples.is_empty(), |view| {
                view.child(
                    div()
                        .id("description-examples")
                        .flex_1()
                        .min_h_0()
                        .pt_6()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(self.model.examples.iter().map(|text| {
                            gpui_kit::component::label::Label::new(format!("“{text}”"))
                                .text_color(cx.theme().muted_foreground)
                        })),
                )
            })
            .when_some(
                bottom_tabs.filter(|_| {
                    !self.model.tabs.is_empty()
                        && self
                            .model
                            .fields
                            .first()
                            .is_some_and(|field| !field.value.trim().is_empty())
                }),
                |d, tabs| d.child(tabs),
            )
            .when(
                search_panel && self.model.rows.is_empty() && !self.model.loading,
                |view| {
                    let queried = self
                        .model
                        .fields
                        .first()
                        .is_some_and(|field| !field.value.trim().is_empty());
                    view.child(
                        gpui_kit::component::empty::Empty::new().header(
                            gpui_kit::component::empty::EmptyHeader::new()
                                .title(gpui_kit::component::empty::EmptyTitle::new().child(
                                    if queried {
                                        "No Results"
                                    } else {
                                        "Search Your Library"
                                    },
                                ))
                                .description(
                                    gpui_kit::component::empty::EmptyDescription::new().child(
                                        if queried {
                                            "Try a different name or search scope."
                                        } else {
                                            "Find artists, albums, playlists, tracks and genres."
                                        },
                                    ),
                                ),
                        ),
                    )
                },
            )
            .when(!endpoint_form && !self.model.rows.is_empty(), |d| {
                d.child(div().flex_1().min_h(px(120.)).children(result_area.take()))
            })
            .child(
                div()
                    .when(self.model.fixed_size, |grid| {
                        grid.grid().grid_cols(2).gap_x_6().gap_y_3()
                    })
                    .when(!self.model.fixed_size, |column| {
                        column.flex().flex_col().gap_3()
                    })
                    .children(
                        self.model
                            .toggles
                            .iter()
                            .enumerate()
                            .map(|(index, toggle)| {
                                let operation = toggle.operation.clone();
                                match toggle.control {
                                    super::surface::ChoiceControl::Checkbox => {
                                        Checkbox::new(("view-option-toggle", index))
                                            .label(toggle.label.clone())
                                            .checked(toggle.checked)
                                            .disabled(toggle.disabled)
                                            .on_click(cx.listener(move |this, _, _, _| {
                                                this.send(operation.clone())
                                            }))
                                            .into_any_element()
                                    }
                                    super::surface::ChoiceControl::Radio => {
                                        gpui_kit::component::radio::Radio::new((
                                            "view-option-radio",
                                            index,
                                        ))
                                        .label(toggle.label.clone())
                                        .checked(toggle.checked)
                                        .disabled(toggle.disabled)
                                        .on_click(cx.listener(move |this, _, _, _| {
                                            this.send(operation.clone())
                                        }))
                                        .into_any_element()
                                    }
                                }
                            }),
                    ),
            )
            .when_some(photo, |d, image| {
                d.child(img(image).max_h(px(180.)).object_fit(ObjectFit::Contain))
            })
            .when_some(self.model.lyrics.as_ref(), |view, lyrics| {
                let lines = lyrics
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| {
                        let mut offset = 0;
                        let ranges = line
                            .spans
                            .iter()
                            .filter_map(|span| {
                                let start = offset;
                                offset += span.text.len();
                                span.active.then_some((
                                    start..offset,
                                    gpui::HighlightStyle {
                                        color: Some(cx.theme().primary),
                                        font_weight: Some(FontWeight::BOLD),
                                        ..Default::default()
                                    },
                                ))
                            })
                            .collect::<Vec<_>>();
                        div()
                            .id(("lyrics-line", index))
                            .debug_selector(move || format!("lyrics-line-{index}"))
                            .w_full()
                            .px_3()
                            .py_2()
                            .text_lg()
                            .text_color(if line.active {
                                cx.theme().primary
                            } else {
                                cx.theme().foreground
                            })
                            .when(line.active, |row| row.font_weight(FontWeight::SEMIBOLD))
                            .child(
                                gpui::StyledText::new(line.value.clone()).with_highlights(ranges),
                            )
                    })
                    .collect::<Vec<_>>();
                view.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .when_some(lyrics.message.as_ref(), |view, message| {
                            view.child(
                                div()
                                    .p_3()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(message.clone()),
                            )
                        })
                        .child(
                            div()
                                .id("lyrics-lines")
                                .debug_selector(|| "lyrics-lines".into())
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.lyrics_scroll)
                                .children(lines),
                        )
                        .child(
                            Scrollbar::vertical(&self.lyrics_scroll)
                                .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                                .viewport_from_layout(),
                        ),
                )
            })
            .when(
                !self.model.text.is_empty()
                    && !search_panel
                    && !matches!(
                        self.model.title.as_str(),
                        "Journey" | "Artist Mix" | "Sonic Mix" | "Stay Close Mix"
                    ),
                |d| {
                    d.child(
                        div()
                            .id("surface-text")
                            .flex_1()
                            .min_h(px(40.))
                            .when(!self.model.rows.is_empty(), |text| {
                                text.flex_none().h_auto()
                            })
                            .text_sm()
                            .overflow_y_scrollbar()
                            .child(self.model.text.clone()),
                    )
                },
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("TextampSurface")
            .on_action(super::keyboard::next_focus)
            .on_action(super::keyboard::previous_focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::actions::Cancel, window, cx| {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(|this, _: &super::CloseWindow, window, cx| {
                this.dismiss(window, cx);
                cx.stop_propagation();
            }))
            .font_family(cx.theme().font_family.clone())
            .text_size(px(13.))
            .line_height(px(20.))
            .when(
                self.embedded && self.model.id.contains(":sidebar-group:"),
                |view| {
                    view.child(
                        div()
                            .flex_none()
                            .h(px(32.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .bg(cx.theme().muted.opacity(0.35))
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.model.title.clone()),
                            )
                            .child(
                                Button::new("close-sidebar-group")
                                    .ghost()
                                    .xsmall()
                                    .p_0()
                                    .size(px(18.))
                                    .icon(
                                        gpui_kit::component::Icon::new(
                                            gpui_kit::assets::IconName::X,
                                        )
                                        .size(px(10.)),
                                    )
                                    .accessibility_label(format!(
                                        "Close {} column",
                                        self.model.title
                                    ))
                                    .tooltip("Close Column · ⌘[")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.dismiss(window, cx)),
                                    ),
                            ),
                    )
                },
            )
            .when(!self.embedded, |view| {
                view.child(
                    TitleBar::new().child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .min_w_0()
                            .truncate()
                            .child(if self.model.fixed_size {
                                "View Options".to_owned()
                            } else {
                                self.model.title.clone()
                            }),
                    ),
                )
            })
            .when(self.model.stale, |view| {
                view.child(
                    gpui_kit::component::alert::Alert::info(
                        "previous-library",
                        "From another library",
                    )
                    .small(),
                )
            })
            .when(self.model.loading, |view| {
                view.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_5()
                        .py_2()
                        .bg(cx.theme().muted.opacity(0.35))
                        .child(Spinner::new().small())
                        .child(if self.model.title == "Journey" {
                            "Building your journey…"
                        } else {
                            "Loading…"
                        }),
                )
            })
            .child(content)
            .when(!endpoint_form && !self.model.buttons.is_empty(), |root| {
                root.child(footer.take().unwrap().px_5().pb_4().pt_2().flex_none())
            })
            .into_any_element()
    }
}

fn stable_choices(
    previous: &[super::surface::Toggle],
    current: &[super::surface::Toggle],
) -> Vec<super::surface::Toggle> {
    previous
        .iter()
        .map(|old| {
            let current = current
                .iter()
                .find(|new| match (&old.operation, &new.operation) {
                    (Operation::ViewOption(a), Operation::ViewOption(b)) => a == b,
                    _ => old.label == new.label,
                });
            let mut choice = current.unwrap_or(old).clone();
            choice.label = old.label.clone();
            choice.disabled = current.is_none() || choice.disabled;
            choice
        })
        .collect()
}

pub fn open_overlay(
    owner: WeakEntity<Desktop>,
    model: Surface,
    commands: super::engine::CommandPort,
    updates: watch::Receiver<Snapshot>,
    window: &mut Window,
    cx: &mut App,
) {
    let entity =
        cx.new(|cx| SurfaceView::new(model.clone(), true, commands.clone(), updates, window, cx));
    let initial = std::cell::Cell::new(true);
    let preferred = preferred_size(&model);
    let width = preferred
        .width
        .min((window.viewport_size().width - px(32.)).max(px(240.)));
    let height = preferred
        .height
        .min((window.viewport_size().height - px(120.)).max(px(160.)));
    window.open_dialog(cx, move |dialog, window, cx| {
        if initial.replace(false) {
            let view = entity.read(cx);
            if model.palette {
                let palette = view.palette.clone();
                palette.update(cx, |s, cx| s.focus(window, cx));
            } else {
                let focus = view
                    .fields
                    .first()
                    .map(|field| field.read(cx).focus_handle(cx))
                    .unwrap_or_else(|| view.focus.clone());
                focus.focus(window, cx);
            }
        }
        let (width, height) = if matches!(model.title.as_str(), "Search" | "Journey") {
            let preferred = preferred_size(&entity.read(cx).model);
            (
                preferred.width.min(window.viewport_size().width - px(32.)),
                preferred
                    .height
                    .min(window.viewport_size().height - px(120.)),
            )
        } else {
            (width, height)
        };
        let commands = commands.clone();
        let id = model.id.clone();
        let owner = owner.clone();
        dialog
            .title(
                div()
                    .flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if model.fixed_size {
                        "View Options".to_owned()
                    } else {
                        model.title.clone()
                    })
                    .child(
                        gpui_kit::base::DialogClose::new().child(
                            Button::new("close-surface")
                                .debug_selector(|| "embedded-close-surface".into())
                                .ghost()
                                .xsmall()
                                .icon(gpui_kit::component::Icon::new(
                                    gpui_kit::assets::IconName::X,
                                ))
                                .accessibility_label("Close"),
                        ),
                    ),
            )
            .when(model.title == "Search" || model.fixed_size, |dialog| {
                dialog.p_3()
            })
            .w(width)
            .margin_top(px(45.))
            .entrance(DialogEntrance::None)
            .close_button(false)
            .when(model.title == "Search", |dialog| {
                let entity = entity.clone();
                dialog.on_ok(move |_, _, cx| {
                    entity.update(cx, |view, cx| {
                        view.flush_fields(cx);
                        view.send(Operation::Actions(vec![
                            crate::app::action::SearchAction::SubmitSearch.into(),
                        ]));
                    });
                    false
                })
            })
            .on_close(move |_, _, cx| {
                let owner = owner.clone();
                let closed_id = id.clone();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |this, _| {
                        this.overlay.did_close(&closed_id);
                    });
                });
                let _ = commands.try_send(Command::CloseSurface(id.clone()));
            })
            .when(model.palette, |dialog| dialog.child(entity.clone()))
            .when(!model.palette, |dialog| {
                dialog.child(
                    div()
                        .h(if model.fixed_size {
                            (height - px(48.)).max(px(160.))
                        } else if model.title == "Search" {
                            (height - px(44.)).max(px(120.))
                        } else {
                            height
                        })
                        .child(entity.clone()),
                )
            })
    });
    window.refresh();
    window.on_next_frame(|window, _| window.refresh());
}

/// Clamp initial content to the usable screen instead of opening under the Dock.
pub(super) fn fit_window_size(preferred: Size<Pixels>, cx: &App) -> Size<Pixels> {
    if let Some(display) = cx.primary_display() {
        let available = display.visible_bounds().size;
        size(
            preferred
                .width
                .min((available.width - px(48.)).max(px(320.))),
            preferred
                .height
                .min((available.height - px(48.)).max(px(180.))),
        )
    } else {
        preferred
    }
}

fn row_height(model: &Surface) -> Pixels {
    px(if model.rows.iter().any(|row| row.label.contains('\n')) {
        48.
    } else {
        32.
    })
}

fn preferred_size(model: &Surface) -> Size<Pixels> {
    if model.lyrics.is_some() {
        return size(px(660.), px(620.));
    }
    if model.radio.is_some() {
        return size(px(940.), px(620.));
    }
    if model.title == "Journey" {
        return size(
            px(600.),
            px(if model.rows.is_empty() { 360. } else { 590. }),
        );
    }
    if matches!(
        model.title.as_str(),
        "Journey" | "Sonic Mix" | "Artist Mix" | "Stay Close Mix"
    ) {
        return size(px(640.), px(600.));
    }
    if model.title == "Search" {
        return size(
            px(560.),
            px(
                if model.rows.is_empty()
                    && model
                        .fields
                        .first()
                        .is_none_or(|field| field.value.trim().is_empty())
                {
                    220.
                } else if model.rows.is_empty() {
                    260.
                } else {
                    460.
                },
            ),
        );
    }
    if model.palette {
        return size(px(608.), px(460.));
    }
    if model.fixed_size {
        return size(px(540.), px(300.));
    }
    if !model.help.is_empty() {
        return size(px(680.), px(612.));
    }
    let browsing = model.layout == WindowLayout::Browser || !model.tabs.is_empty();
    let width = if browsing || model.fields.len() > 1 {
        620.
    } else if model.layout == WindowLayout::Reading || model.text.chars().count() > 500 {
        660.
    } else {
        460.
    };
    let line_width = ((width - 48.) / 7.) as usize;
    let text_lines = model
        .text
        .lines()
        .map(|line| line.chars().count().div_ceil(line_width).max(1))
        .sum::<usize>();
    let text_height = if model.text.is_empty() {
        0.
    } else {
        text_lines.clamp(2, 24) as f32 * 18.
    };
    let rows = f32::from(row_height(model)) * model.rows.len().clamp(0, 9) as f32;
    let height = 96.
        + model.fields.len() as f32 * 46.
        + model.endpoints.len() as f32 * 32.
        + rows
        + model.toggles.len() as f32 * 28.
        + text_height
        + if model.tabs.is_empty() { 0. } else { 36. }
        + if model.buttons.is_empty() { 0. } else { 44. }
        + if model.image.is_some() { 180. } else { 0. };
    size(
        px(width),
        px(height.clamp(
            if model.layout == WindowLayout::Reading && model.image.is_some() {
                400.
            } else if browsing {
                420.
            } else {
                190.
            },
            720.,
        )),
    )
}

/// Native title bar and traffic lights own close, drag, resize and window movement.
pub fn open_native(
    model: Surface,
    commands: super::engine::CommandPort,
    updates: watch::Receiver<Snapshot>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let title = model.title.clone();
    let auto_fit = !model.fixed_size;
    let initial = fit_window_size(preferred_size(&model), cx);
    let minimum = size(
        px(if model.title == "Search" {
            480.
        } else if model.layout == WindowLayout::Browser {
            620.
        } else {
            360.
        }),
        px(if model.radio.is_some() {
            420.
        } else if !model.rows.is_empty() {
            280.
        } else {
            160.
        }),
    );
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..TitleBar::title_bar_options()
            }),
            window_bounds: Some(WindowBounds::Windowed(super::cascade_bounds(initial, cx))),
            window_min_size: Some(minimum.min(&initial)),
            is_resizable: !model.fixed_size,
            ..TitleBar::window_options()
        },
        cx,
        |window, cx| {
            let entity = cx.new(|cx| SurfaceView::new(model, false, commands, updates, window, cx));
            entity.update(cx, |this, _| {
                this.auto_fit = auto_fit.then(|| window.viewport_size())
            });
            let owner = entity.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                let _ = owner.update(cx, |this, _| {
                    let _ = this
                        .commands
                        .try_send(Command::CloseSurface(this.model.id.clone()));
                });
                true
            });
            let view = entity.read(cx);
            let focus = if view.model.palette {
                view.palette.read(cx).focus_handle(cx)
            } else {
                view.fields
                    .first()
                    .map(|field| field.read(cx).focus_handle(cx))
                    .unwrap_or_else(|| view.focus.clone())
            };
            focus.focus(window, cx);
            entity
        },
    )?;
    Ok(handle)
}

/// macOS owns filename entry, location selection and overwrite confirmation.
pub(super) fn prompt_export(model: &Surface, commands: super::engine::CommandPort, cx: &mut App) {
    let Some(save) = model.save_file.clone() else {
        return;
    };
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let chosen = cx.prompt_for_new_path(&directory, Some(&save.suggested_name));
    let id = model.id.clone();
    let revision = model.revision;
    cx.spawn(async move |_| {
        let action = match chosen.await {
            Ok(Ok(Some(path))) => {
                let mut action = save.action;
                if let crate::app::Action::Queue(
                    crate::app::action::QueueAction::ExportPlaylist { path: target, .. },
                ) = &mut action
                {
                    *target = path;
                }
                Some(action)
            }
            Ok(Ok(None)) => None,
            result => Some(
                crate::app::action::SystemAction::ShowError(format!(
                    "Unable to choose export file: {result:?}"
                ))
                .into(),
            ),
        };
        if let Some(action) = action {
            let _ = commands
                .send(Command::Surface {
                    id: id.clone(),
                    revision,
                    operation: Operation::Actions(vec![action]),
                })
                .await;
        }
        let _ = commands.send(Command::CloseSurface(id)).await;
    })
    .detach();
}

#[cfg(test)]
mod sizing_tests {
    use super::{Operation, Surface, WindowLayout, preferred_size};
    use gpui_kit::px;
    #[test]
    fn view_choices_keep_positions_and_labels_when_options_change() {
        use super::super::surface::{ChoiceControl, Operation, Toggle};
        use crate::app::state::SortPopupOption;
        let old = vec![Toggle {
            disabled: false,
            control: ChoiceControl::Checkbox,
            label: "Reverse order".into(),
            checked: true,
            operation: Operation::ViewOption(SortPopupOption::Direction),
        }];
        let hidden = super::stable_choices(&old, &[]);
        assert_eq!(hidden.len(), 1);
        assert!(hidden[0].disabled);
        let new = vec![Toggle {
            disabled: false,
            label: "Oldest first".into(),
            checked: false,
            ..old[0].clone()
        }];
        let stable = super::stable_choices(&old, &new);
        assert_eq!(stable[0].label, "Reverse order");
        assert!(!stable[0].checked);
        assert!(!stable[0].disabled);
    }

    #[test]
    fn view_options_size_does_not_depend_on_grouping_or_available_controls() {
        let mut options = Surface::default();
        options.fixed_size = true;
        let original = preferred_size(&options);
        options.text = "A different column title".into();
        options.rows = vec![];
        options.toggles = (0..9)
            .map(|_| super::super::surface::Toggle {
                disabled: false,
                control: super::super::surface::ChoiceControl::Checkbox,
                label: "Option".into(),
                checked: false,
                operation: super::super::surface::Operation::Key(crossterm::event::KeyCode::Esc),
            })
            .collect();
        assert_eq!(preferred_size(&options), original);
        options.toggles.truncate(4);
        assert_eq!(preferred_size(&options), original);
    }
    #[test]
    fn search_panel_is_compact_until_it_has_results() {
        let mut state = crate::app::AppState::new();
        state.popups.set_search_active(true);
        let empty = super::super::surface::project(&state).unwrap();
        assert_eq!(preferred_size(&empty).height, px(220.));
        state.search.query = "no matching title".into();
        let no_results = super::super::surface::project(&state).unwrap();
        assert_eq!(preferred_size(&no_results).height, px(260.));
        assert!(preferred_size(&no_results).width <= px(600.));
    }
    #[test]
    fn workflow_windows_fit_short_prompts_and_allocate_space_for_results_and_reading() {
        let mut prompt = Surface::default();
        prompt.fields.push(super::super::surface::Field {
            label: String::new(),
            value: String::new(),
            masked: false,
        });
        let mut reading = Surface::default();
        reading
            .text
            .push_str(&"A long paragraph of help text. ".repeat(80));
        let mut browser = Surface::default();
        browser.layout = WindowLayout::Browser;
        browser.rows = (0..9)
            .map(|index| super::super::surface::Item {
                key: index.to_string(),
                label: "Result".into(),
                selected: false,
                aliases: vec![],
                operation: Operation::Row(index),
            })
            .collect();
        assert!(preferred_size(&prompt).height < px(300.));
        assert!(preferred_size(&reading).height > preferred_size(&prompt).height);
        assert!(preferred_size(&browser).width > preferred_size(&prompt).width);
        assert!(preferred_size(&browser).height >= px(360.));
    }
}

#[cfg(test)]
mod search_input_tests {
    use super::{Command, Operation, Snapshot, SurfaceView};
    use gpui_kit::gpui::{self, AppContext, Focusable};
    #[gpui::test]
    fn enter_in_search_input_executes_without_selecting_or_closing(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let mut state = crate::app::AppState::new();
        state.popups.set_search_active(true);
        state.search.query = "song".into();
        state.search.results = Some(crate::library::models::SearchResults {
            tracks: vec![crate::library::models::Track {
                rating_key: "song".into(),
                title: "Song".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let model = super::super::surface::project(&state).unwrap();
        let (engine, _updates, mut commands) = super::super::Engine::fixture(Snapshot::default());
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                SurfaceView::new(
                    model,
                    true,
                    engine.commands.clone(),
                    engine.snapshot.clone(),
                    window,
                    cx,
                )
            });
            saved.replace(Some(view.clone()));
            gpui_kit::base::Root::new(view, window, cx)
        });
        let view = slot.borrow().clone().unwrap();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.fields[0].read(cx).focus_handle(cx).focus(window, cx)
            });
            window.draw(cx).clear(cx);
        });
        while commands.try_recv().is_ok() {}
        cx.simulate_keystrokes("enter");
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert_eq!(sent.iter().filter(|command| matches!(command, Command::Surface { operation: Operation::Actions(actions), .. } if matches!(actions.as_slice(), [crate::app::Action::Search(crate::app::action::SearchAction::SubmitSearch)]))).count(), 1);
        assert!(!sent.iter().any(|command| matches!(
            command,
            Command::Surface {
                operation: Operation::Key(crossterm::event::KeyCode::Enter),
                ..
            } | Command::CloseSurface(_)
        )));
        cx.update(|window, cx| {
            assert!(
                view.read(cx).fields[0]
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            )
        });
    }
}
