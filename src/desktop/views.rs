//! Desktop composition: restrained native chrome around the column browser.
use super::catalog::ControlTooltip as _;
use super::*;
use gpui_kit::assets::IconName as Glyph;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{
    Collapsible, Disableable, Icon, Selectable, Sizable, TitleBar,
    avatar::Avatar,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::ButtonVariants,
    description_list::DescriptionList,
    empty::{Empty as EmptyState, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle},
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    scroll::{ScrollableElement, Scrollbar, ScrollbarHandle, ScrollbarMode},
    sidebar::{SidebarItem, SidebarMenu, SidebarMenuItem},
    slider::Slider,
    spinner::Spinner,
    status_bar::StatusBar,
};

struct HeaderClock {
    offset: Option<f32>,
    _task: Task<()>,
}

impl HeaderClock {
    fn start(overflow: f32, cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(crate::util::column_heading::DELAY)
                    .await;
                let started = std::time::Instant::now();
                loop {
                    let offset = (started.elapsed().as_secs_f32() * 30.).min(overflow);
                    if entity
                        .update(cx, |clock: &mut HeaderClock, cx| {
                            clock.offset = Some(offset);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    if offset >= overflow {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(33))
                        .await;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                if entity
                    .update(cx, |clock, cx| {
                        clock.offset = None;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
        HeaderClock {
            offset: None,
            _task: task,
        }
    }
}

pub(super) fn control_icon(control: &super::catalog::Control) -> Option<Glyph> {
    use crate::app::{
        Action,
        action::{FolderAction, QueueAction, QueueLoadIntent, RadioAction, SystemAction},
    };
    match control.actions.as_slice() {
        [
            Action::Queue(
                QueueAction::PlayAlbumNow { .. }
                | QueueAction::PlayPlaylistNow { .. }
                | QueueAction::PlayTrack(_)
                | QueueAction::PlayTracksNow(_),
            ),
        ]
        | [
            Action::Folders(FolderAction::PlayFolderTrack { .. } | FolderAction::PlayFolderTracks),
        ] => Some(Glyph::Play),
        [
            Action::Queue(
                QueueAction::EnqueueAlbum { .. }
                | QueueAction::EnqueueTrack(_)
                | QueueAction::TracksLoaded {
                    intent: QueueLoadIntent::Append { .. },
                    ..
                },
            ),
        ] => Some(Glyph::Plus),
        [
            Action::Queue(QueueAction::EnqueueAlbumNext { .. } | QueueAction::EnqueueTracksNext(_)),
        ] => Some(Glyph::ListStart),
        [Action::Source(crate::app::sources::SourceAction::Navidrome(action))] => {
            match action.as_ref() {
                crate::app::sources::navidrome::NavAction::Command(
                    crate::app::sources::navidrome::commands::Command::EditEntry { edit, .. },
                ) => Some(match edit {
                    crate::app::sources::navidrome::playlist_edit::Edit::Up => Glyph::ArrowUp,
                    crate::app::sources::navidrome::playlist_edit::Edit::Down => Glyph::ArrowDown,
                    crate::app::sources::navidrome::playlist_edit::Edit::Remove => Glyph::Trash,
                }),
                _ => None,
            }
        }
        [Action::Radio(RadioAction::StartArtistRadio { .. })] => Some(Glyph::Radio),
        [Action::Search(crate::app::action::SearchAction::SearchArtistInfoOnWeb)] => {
            Some(Glyph::Search)
        }
        [
            Action::System(SystemAction::Downloads(
                crate::app::downloads::Command::Album(_)
                | crate::app::downloads::Command::Tracks(_),
            )),
        ] => Some(Glyph::Download),
        [Action::System(SystemAction::Downloads(crate::app::downloads::Command::Remove(_)))] => {
            Some(Glyph::Trash)
        }
        _ => None,
    }
}

fn playback_menu(
    button: Button,
    control: &super::catalog::Control,
    commands: super::engine::CommandPort,
    generation: u64,
    target: Option<browser::Target>,
) -> AnyElement {
    use crate::app::playback_controls::Variant;
    use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
    if !control.has_next() {
        return button.into_any_element();
    }
    let choices = [Variant::Primary, Variant::Next].map(|variant| {
        let (label, actions) = control.resolve(variant);
        (label.to_owned(), actions.to_vec())
    });
    button
        .context_menu(move |mut menu, _, _| {
            for (label, actions) in &choices {
                let commands = commands.clone();
                let actions = actions.clone();
                let target = target.clone();
                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, _| {
                    let _ = commands.try_send(Command::GuardedActions {
                        generation,
                        target: target.clone(),
                        actions: actions.clone(),
                    });
                }));
            }
            menu
        })
        .open_on_hold(true)
        .into_any_element()
}

/// Report the range laid out by the stock list, never its measurement callbacks.
/// UniformList also calls its row renderer with 0..1 to measure row geometry.
struct ColumnViewport {
    page_scroll: Option<ScrollHandle>,
    owner: WeakEntity<Desktop>,
    column: usize,
}
impl UniformListDecoration for ColumnViewport {
    fn compute(
        &self,
        mut range: std::ops::Range<usize>,
        bounds: Bounds<Pixels>,
        _: Point<Pixels>,
        row_height: Pixels,
        item_count: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // The page clips the album rows and owns the only vertical offset.
        if self.page_scroll.is_some() {
            let viewport = window.content_mask().bounds;
            let to_row = |y| f32::from(y - bounds.top()) / f32::from(row_height).max(1.);
            range.start = (to_row(viewport.top()).floor().max(0.) as usize).min(item_count);
            range.end = (to_row(viewport.bottom()).ceil().max(0.) as usize).min(item_count);
        }
        let _ = self.owner.update(cx, |this, cx| {
            if let Some(column) = this.snapshot.columns.get(self.column) {
                if this
                    .grid_widths
                    .insert(column.identity.clone(), bounds.size.width)
                    != Some(bounds.size.width)
                {
                    cx.notify();
                }
            }
            if let Some(column) = this.snapshot.columns.get(self.column)
                && let Some(row) = this.pending_centered_rows.remove(&column.identity)
            {
                // Center with the measured list geometry.
                let maximum = (row_height * item_count - bounds.size.height).max(px(0.));
                let minimum = px(0.);
                let centered = row_height * this.grid(&this.snapshot, self.column).band(row)
                    + row_height / 2.
                    - bounds.size.height / 2.;
                let top = centered.max(minimum).min(maximum);
                if let Some(page) = &self.page_scroll {
                    let viewport = window.content_mask().bounds;
                    let center = bounds.top()
                        + row_height
                            * (this.grid(&this.snapshot, self.column).band(row) as f32 + 0.5);
                    let adjustment = if this.snapshot.navigation_origin == browser::Origin::Keyboard
                    {
                        let top = center - row_height / 2.;
                        let bottom = center + row_height / 2.;
                        if top < viewport.top() {
                            viewport.top() - top
                        } else {
                            (viewport.bottom() - bottom).min(px(0.))
                        }
                    } else {
                        viewport.center().y - center
                    };
                    let y = (page.offset().y + adjustment).clamp(-page.max_offset().y, px(0.));
                    page.set_offset(point(px(0.), y));
                    cx.notify();
                } else if let Some(scroll) = this.scroll_handles.get(&column.identity) {
                    let offset = scroll.offset();
                    scroll.set_offset(point(offset.x, -top));
                    cx.notify();
                }
                return;
            }
            if super::filter::indices(&this.snapshot, self.column).is_some() {
                return;
            }
            if let Some(column) = this.snapshot.columns.get(self.column) {
                let header = super::filter::header_count(&this.snapshot, self.column);
                let grid = this.grid(&this.snapshot, self.column);
                let range = grid.start(range.start.saturating_sub(header))
                    ..grid
                        .start(range.end.saturating_sub(header))
                        .min(column.rows.len());
                let visible = (
                    range.clone(),
                    column.rows.len(),
                    this.snapshot.art_generation,
                );
                if this.visible_rows.get(&column.identity) != Some(&visible) {
                    this.visible_rows.insert(column.identity.clone(), visible);
                    let _ = this.engine.commands.try_send(Command::Viewport {
                        generation: this.snapshot.generation,
                        column: self.column,
                        identity: column.identity.clone(),
                        range,
                    });
                }
            }
        });
        div().into_any_element()
    }
}

pub(super) fn breadcrumb_items(snapshot: &Snapshot) -> Vec<(String, Option<usize>)> {
    let player = if snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Text) {
        snapshot.presentation.workspace()
            == Some(crate::app::windowed_presentation::Workspace::NowPlaying)
    } else {
        snapshot.player.active
    };
    if player {
        return vec![("Now Playing".into(), None)];
    }
    if let Some(title) = &snapshot.workspace_title {
        return vec![(title.clone(), None)];
    }
    let mut items: Vec<_> = snapshot
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| {
            !column.metadata
                && (!column.hidden
                    || column.rows.get(column.selected).is_some_and(|row| {
                        matches!(
                            row.kind,
                            browser::Kind::Artist | browser::Kind::Playlist | browser::Kind::Genre
                        )
                    }))
        })
        .map(|(index, column)| (browser::breadcrumb_label(column), Some(index)))
        .collect();
    if items.is_empty() {
        items.push((snapshot.section.clone(), None));
    }
    items
}

impl Desktop {
    fn command_button(
        &self,
        id: String,
        control: super::catalog::Control,
        target: Option<browser::Target>,
        cx: &mut Context<Self>,
    ) -> Button {
        let generation = self.snapshot.generation;
        Button::new(SharedString::from(id))
            .small()
            .label(control.label.clone())
            .map(|button| {
                let mut button = button;
                gpui_kit::InteractiveElement::interactivity(&mut button).tooltip(control.tooltip());
                button
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _, _| {
                let _ = this.engine.commands.try_send(Command::GuardedActions {
                    generation,
                    target: target.clone(),
                    actions: control.actions_for(event.modifiers().alt, event.modifiers().shift),
                });
            }))
    }
    pub(super) fn titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut breadcrumb = Breadcrumb::new();
        for (label, column) in breadcrumb_items(&self.snapshot) {
            let generation = self.snapshot.generation;
            let identity = column
                .and_then(|index| self.snapshot.columns.get(index))
                .map(|column| column.identity.clone());
            breadcrumb = breadcrumb.child(BreadcrumbItem::new(label).on_click(cx.listener(
                move |this, _, _, _| {
                    if let (Some(column), Some(identity)) = (column, identity.clone()) {
                        let _ = this.engine.commands.try_send(Command::FocusColumn {
                            generation,
                            column,
                            identity,
                        });
                    } else {
                        this.command(PaletteCommandKind::GotoLibrary);
                    }
                },
            )));
        }
        let brushed = self.snapshot.theme == crate::app::theme::ThemeName::BrushedAluminum;
        let bar = TitleBar::new()
            .when(brushed, |bar| bar.bg(transparent_black()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .size_full()
                    .pr_3()
                    .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                        if !this.snapshot.separate_player && this.overlay.id().is_some() {
                            window.close_dialog(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(breadcrumb),
                            ),
                    ),
            );
        if brushed {
            div()
                .relative()
                .flex_none()
                .child(super::theme::aluminum_surface())
                .child(bar)
                .into_any_element()
        } else {
            bar.into_any_element()
        }
    }

    fn sidebar_expanded(&self) -> bool {
        self.snapshot.sheet_browser
            || (self.sidebar_visible && (!self.sheet_open() || self.snapshot.separate_player))
    }

    fn sidebar(
        &mut self,
        progress: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let compact = progress <= 0.;
        let mut rows = Vec::new();
        let visible: std::collections::HashSet<_> = super::sidebar::visible_rows(
            &self.snapshot.sidebar,
            &self.snapshot.collapsed_sidebar_sections,
            compact,
        )
        .into_iter()
        .collect();

        for (index, row) in self.snapshot.sidebar.iter().enumerate() {
            if !visible.contains(&index) {
                continue;
            }
            let generation = self.snapshot.generation;
            if row.header {
                let open = !self
                    .snapshot
                    .collapsed_sidebar_sections
                    .contains(&row.label);
                let title = row.label.clone();
                let expandable = super::sidebar::allows_disclosure(&self.snapshot.sidebar, &title);
                let commands = self.engine.commands.clone();
                let disclosure_title = title.clone();
                let item = SidebarMenuItem::new(title.clone())
                    .bg(cx.theme().sidebar)
                    .active(self.snapshot.sidebar_cursor == Some(index) && !self.rapid_navigation)
                    .accessibility_label(format!("Open {title}"))
                    .icon(super::sidebar::icon(row))
                    .when(expandable && !compact, |item| {
                        item.suffix(move |_, _| {
                            let commands = commands.clone();
                            let title = disclosure_title.clone();
                            let label =
                                format!("{} {title}", if open { "Collapse" } else { "Expand" });
                            Button::new(("sidebar-disclosure", index))
                                .debug_selector({
                                    let title = title.clone();
                                    move || format!("sidebar-disclosure-{title}")
                                })
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(if open {
                                    Glyph::ChevronDown
                                } else {
                                    Glyph::ChevronRight
                                }))
                                .accessibility_label(label.clone())
                                .tooltip(label)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let _ = commands.try_send(Command::ToggleSidebarSection {
                                        generation,
                                        title: title.clone(),
                                    });
                                })
                        })
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sheet_sidebar_root = !this.snapshot.sheet_browser;
                        this.queue_focused = false;
                        cx.notify();
                        this.focus.focus(window, cx);
                        let _ = this.engine.commands.try_send(Command::SidebarGroup {
                            generation,
                            title: title.clone(),
                        });
                    }));
                rows.push(
                    SidebarMenu::new()
                        .collapsed(compact)
                        .child(item)
                        .render(index, window, cx)
                        .into_any_element(),
                );
                continue;
            }
            let playlist = row.playlist_key.clone().map(|key| (key, row.label.clone()));
            let item = SidebarMenuItem::new(row.label.clone())
                .bg(cx.theme().sidebar)
                .icon(super::sidebar::icon(row))
                .active(self.snapshot.sidebar_cursor.map_or(row.active, |cursor| {
                    cursor == index && !self.rapid_navigation
                }))
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.sheet_sidebar_root = !this.snapshot.sheet_browser;
                    this.queue_focused = false;
                    cx.notify();
                    this.focus.focus(window, cx);
                    if let Some((key, title)) =
                        playlist.as_ref().filter(|_| event.click_count() == 2)
                    {
                        let _ = this.engine.commands.try_send(Command::GuardedActions {
                            generation,
                            target: None,
                            actions: vec![
                                crate::app::action::QueueAction::PlayPlaylistNow {
                                    playlist_key: key.clone(),
                                    title: title.clone(),
                                }
                                .into(),
                            ],
                        });
                    } else {
                        let _ = this
                            .engine
                            .commands
                            .try_send(Command::SidebarOpen { generation, index });
                    }
                }));
            rows.push(
                div()
                    .debug_selector({
                        let label = row.label.clone();
                        move || format!("sidebar-{label}")
                    })
                    .when(!compact && row.parent.is_some(), |row| {
                        row.pl_5().when(progress < 1., |row| {
                            row.h(px(28. * progress)).overflow_hidden()
                        })
                    })
                    .child(
                        SidebarMenu::new()
                            .collapsed(compact)
                            .child(item)
                            .render(index, window, cx),
                    )
                    .into_any_element(),
            );
        }
        // Kit Sidebar owns an inaccessible group scroll handle. Compose its stock
        // menus and disclosures in a tracked pane so keyboard rows stay visible.
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("sidebar-content")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll)
                            .py(px(12. * progress))
                            .pl(px(4. + 8. * progress))
                            .pr(px(4. + 16. * progress))
                            .children(rows),
                    )
                    .child(
                        div().relative().w(px(12.)).h_full().flex_none().child(
                            Scrollbar::vertical(&self.sidebar_scroll)
                                .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                                .viewport_from_layout(),
                        ),
                    ),
            )
    }

    fn column_action_buttons(
        &self,
        index: usize,
        column_snapshot: &browser::Column,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        column_snapshot
            .header_controls
            .iter()
            .enumerate()
            .map(|(control_index, control)| {
                let generation = self.snapshot.generation;
                let identity = column_snapshot.identity.clone();
                let revision = column_snapshot.revision;
                let playlist_action = (column_snapshot.playlist_editing
                    || matches!(
                        control.actions.first(),
                        Some(crate::app::Action::Browse(
                            crate::app::action::BrowseAction::SetPlaylistEditing { .. }
                        ))
                    ))
                .then(|| control.actions.as_ref().clone());
                Button::new(SharedString::from(format!(
                    "column-control-{index}-{control_index}"
                )))
                .debug_selector(move || format!("column-control-{index}-{control_index}"))
                .ghost()
                .xsmall()
                .map(|button| match control.actions.first() {
                    Some(crate::app::Action::Browse(
                        crate::app::action::BrowseAction::SetPlaylistEditing { editing, .. },
                    )) => button.icon(Icon::new(if *editing {
                        Glyph::Pencil
                    } else {
                        Glyph::Check
                    })),
                    _ => button.icon(Icon::new(control_icon(control).unwrap_or(Glyph::Play))),
                })
                .when(
                    column_snapshot.album_page.is_some() && control_index == 0,
                    |button| {
                        button
                            .primary()
                            .large()
                            .rounded_full()
                            .label("Play")
                            .min_w(px(96.))
                    },
                )
                .accessibility_label(control.label.clone())
                .map(|button| {
                    let mut button = button;
                    gpui_kit::InteractiveElement::interactivity(&mut button)
                        .tooltip(control.tooltip());
                    button
                })
                .on_click(cx.listener(move |this, event: &ClickEvent, _, _| {
                    if let Some(actions) = &playlist_action {
                        let _ = this.engine.commands.try_send(Command::GuardedActions {
                            generation,
                            target: None,
                            actions: actions.clone(),
                        });
                        return;
                    }
                    let _ = this.engine.commands.try_send(Command::ColumnControl {
                        generation,
                        column: index,
                        identity: identity.clone(),
                        revision,
                        control: control_index,
                        option: event.modifiers().alt,
                        shift: event.modifiers().shift,
                    });
                }))
                .map(|button| {
                    playback_menu(
                        button,
                        control,
                        self.engine.commands.clone(),
                        generation,
                        column_snapshot.target(generation, index, column_snapshot.selected),
                    )
                })
            })
            .collect()
    }

    fn album_info_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(column) = self.snapshot.columns.get(index) else {
            return div().into_any_element();
        };
        let generation = self.snapshot.generation;
        let identity = column.identity.clone();
        ListItem::new("album-info")
            .bg(cx.theme().background)
            .h(px(column.row_height()))
            .w_full()
            .mx_1()
            .rounded(px(4.))
            .selected(
                column.album_info_selected
                    && self.snapshot.focused_column == index
                    && self.snapshot.sidebar_cursor.is_none(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(Glyph::Info).size_4())
                    .child("Album Notes"),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus.focus(window, cx);
                let _ = this.engine.commands.try_send(Command::AlbumInfo {
                    generation,
                    column: index,
                    identity: identity.clone(),
                });
            }))
            .into_any_element()
    }
    pub(super) fn workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.snapshot.player.active {
            return self.now_playing(cx).into_any_element();
        }
        self.library_workspace(window, cx).0
    }

    pub(super) fn submit_description(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.description_input.read(cx).value().trim().to_owned();
        if query.is_empty() || self.snapshot.workspace_title.as_deref() != Some("Describe Music") {
            return;
        }
        let _ = self.engine.commands.try_send(Command::GuardedActions {
            generation: self.snapshot.generation,
            target: None,
            actions: vec![
                crate::app::sources::audiomuse::Command::Search {
                    feature: crate::audiomuse::Feature::Describe,
                    query,
                }
                .into(),
            ],
        });
        self.focus.focus(window, cx);
    }

    fn navigation_back_button(&self, drilled: bool, cx: &Context<Self>) -> Button {
        let id = if drilled {
            "sheet-browser-back"
        } else {
            "library-back"
        };
        let label = if drilled {
            "Back one level"
        } else if self.sheet_open() {
            "Return to Library"
        } else {
            "Go back"
        };
        Button::new(id)
            .debug_selector(move || id.into())
            .ghost()
            .xsmall()
            .w(px(30.))
            .h(px(30.))
            .icon(Icon::new(Glyph::ChevronLeft).size(px(18.)))
            .accessibility_label(label)
            .tooltip(if drilled {
                "Back one level · ⌘["
            } else {
                label
            })
            .disabled(!drilled && !self.sheet_open() && self.snapshot.jump_head.is_none())
            .on_click(cx.listener(move |this, _, _, cx| {
                if drilled {
                    this.sheet_browser_back(cx);
                } else if this.sheet_open() {
                    this.command(PaletteCommandKind::GotoLibrary);
                } else {
                    let _ = this.engine.commands.try_send(Command::Action(
                        crate::app::action::NavigationAction::Back.into(),
                    ));
                }
            }))
    }

    pub(super) fn sheet_browser_back(&mut self, cx: &mut Context<Self>) {
        let snapshot = &self.snapshot;
        let generation = snapshot.generation;
        if snapshot.sheet_shows_inspector() {
            let _ = self.engine.commands.try_send(Command::ClosePreview {
                generation,
                navigation_serial: snapshot.navigation_serial,
            });
        } else if snapshot.radio_browser.is_some()
            && let Some(workspace) = &snapshot.workspace
        {
            let _ = self
                .engine
                .commands
                .try_send(Command::CloseSurface(workspace.id.clone()));
        } else if snapshot
            .radio_browser
            .as_ref()
            .and_then(|s| s.radio.as_ref())
            .is_some_and(|radio| radio.focused_column > 0)
        {
            let _ = self.engine.commands.try_send(Command::Action(
                crate::app::action::RadioAction::NavigateStationsBack.into(),
            ));
        } else if snapshot.workspace.is_none()
            && snapshot.radio_browser.is_none()
            && !snapshot.player_sync.visible
            && let Some((index, current)) = snapshot
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.hidden && !c.metadata)
                .min_by_key(|(i, _)| i.abs_diff(snapshot.focused_column))
            && snapshot.columns[..index]
                .iter()
                .any(|c| !c.hidden && !c.metadata)
        {
            let _ = self.engine.commands.try_send(Command::CloseColumn {
                generation,
                column: index,
                identity: current.identity.clone(),
            });
        } else {
            self.sheet_sidebar_root = true;
            cx.notify();
        }
    }
    pub(super) fn library_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Pixels) {
        use gpui_kit::base::motion::{Spring, spring};
        let sidebar_progress = spring(
            "navigation-sidebar",
            if self.sidebar_expanded() {
                1_f32
            } else {
                0_f32
            },
            Spring::new(self.presentation_duration(super::motion::SIDEBAR_DURATION)),
            window,
            cx,
        )
        .clamp(0., 1.);
        let sidebar_settled = self.sidebar_expanded() && sidebar_progress == 1.;
        let drilled = self.snapshot.sheet_browser && !self.sheet_sidebar_root;
        let body = if drilled {
            self.library_content(true, window, cx)
        } else {
            self.sidebar(sidebar_progress, window, cx)
                .into_any_element()
        };
        let body = if self.snapshot.sheet_browser {
            use gpui::AnimationExt;
            let page = if !drilled {
                "root"
            } else if self.snapshot.sheet_shows_inspector() {
                "inspector"
            } else {
                self.snapshot
                    .workspace_title
                    .as_deref()
                    .or_else(|| {
                        self.snapshot
                            .columns
                            .get(self.snapshot.focused_column)
                            .map(|col| col.identity.as_str())
                    })
                    .unwrap_or(&self.snapshot.section)
            };
            div()
                .size_full()
                .child(body)
                .with_animation(
                    SharedString::from(format!("sheet-sidebar-page-{page}")),
                    gpui::Animation::new(super::motion::COLUMN_DURATION),
                    move |body, progress| {
                        let remaining = (1. - progress).powi(3);
                        body.relative()
                            .left(px(if drilled { 10. } else { -10. }) * remaining)
                            .opacity(1. - 0.3 * remaining)
                    },
                )
                .into_any_element()
        } else {
            body
        };
        let sidebar = div()
            .track_focus(self.pane_focus.handle(super::keyboard::Pane::Sidebar))
            .tab_group()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(9.))
                    .gap_1()
                    .overflow_hidden()
                    .child(
                        Button::new("toggle-sidebar")
                            .debug_selector(|| "toggle-sidebar".into())
                            .ghost()
                            .xsmall()
                            .w(px(30.))
                            .h(px(30.))
                            .icon(Icon::new(Glyph::PanelLeft).size(px(20.)))
                            .selected(self.sidebar_expanded())
                            .when(self.sidebar_expanded(), |button| button.primary())
                            .accessibility_label(if self.sidebar_expanded() {
                                "Hide Sidebar"
                            } else {
                                "Show Sidebar"
                            })
                            .tooltip(if self.sidebar_expanded() {
                                "Hide Sidebar"
                            } else {
                                "Show Sidebar"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                let _ = this.engine.commands.try_send(Command::Action(
                                    crate::app::action::NavigationAction::ToggleSidebar.into(),
                                ));
                                cx.notify();
                            })),
                    )
                    .when(sidebar_progress > 0.6, |header| {
                        header
                            .pr(px(7.))
                            .child(div().flex_1().overflow_hidden().text_sm().child("Library"))
                            .when(self.sheet_open(), |header| {
                                header.child(self.navigation_back_button(drilled, cx))
                            })
                    }),
            )
            .when(sidebar_progress <= 0.6 && self.sheet_open(), |sidebar| {
                sidebar.child(
                    div()
                        .flex_none()
                        .flex()
                        .pl(px(9.))
                        .child(self.navigation_back_button(drilled, cx)),
                )
            })
            .child(div().flex_1().min_h_0().overflow_hidden().child(body));
        let rail = px(super::motion::SIDEBAR_RAIL_WIDTH);
        let sidebar_width = rail + (self.sidebar_width - rail) * sidebar_progress;
        let sidebar_owner = cx.entity().downgrade();
        let browser = if self.snapshot.sheet_browser {
            div().into_any_element()
        } else {
            self.library_content(false, window, cx)
        };
        let panels = h_resizable(SharedString::from(format!(
            "desktop-layout-{}-{sidebar_settled}",
            self.layout_epoch
        )))
        .child(
            resizable_panel()
                .size(sidebar_width)
                .size_range(if sidebar_settled {
                    px(140.)..px(500.)
                } else {
                    sidebar_width..sidebar_width
                })
                .fixed()
                .child(
                    div()
                        .debug_selector(|| "navigation-sidebar".into())
                        .size_full()
                        .overflow_hidden()
                        .on_prepaint(move |bounds, _, cx| {
                            if sidebar_settled {
                                let _ = sidebar_owner
                                    .update(cx, |this, _| this.sidebar_width = bounds.size.width);
                            }
                        })
                        .child(
                            div()
                                .size_full()
                                .when(
                                    self.snapshot.theme
                                        != crate::app::theme::ThemeName::BrushedAluminum,
                                    |pane| pane.border_r_2().border_color(cx.theme().border),
                                )
                                .child(sidebar),
                        ),
                ),
        )
        .child(
            resizable_panel().child(
                div()
                    .size_full()
                    .when(!self.snapshot.sheet_browser, |pane| {
                        pane.track_focus(self.pane_focus.handle(super::keyboard::Pane::Library))
                            .tab_group()
                    })
                    .child(browser),
            ),
        );
        let workspace = div()
            .debug_selector(|| "library-workspace".into())
            .size_full()
            .flex_1()
            .min_h_0()
            .child(panels)
            .into_any_element();
        (workspace, sidebar_width)
    }

    pub(super) fn library_content(
        &mut self,
        single: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.snapshot.player_sync.visible {
            self.player_devices = None;
        }
        let browser = if self.snapshot.player_sync.visible {
            self.player_devices
                .get_or_insert_with(|| {
                    cx.new(|cx| {
                        super::player_devices::PlayerDevices::new(self.engine.clone(), window, cx)
                    })
                })
                .clone()
                .into_any_element()
        } else if let Some(model) = self.snapshot.radio_browser.clone() {
            let entity = self
                .radio_browser
                .get_or_insert_with(|| {
                    cx.new(|cx| {
                        surface_view::SurfaceView::new(
                            model,
                            true,
                            self.engine.commands.clone(),
                            self.engine.snapshot.clone(),
                            window,
                            cx,
                        )
                    })
                })
                .clone();
            entity.into_any_element()
        } else if let Some(model) = self.snapshot.workspace.clone() {
            self.radio_browser = None;
            if self
                .workspace_view
                .as_ref()
                .is_none_or(|view| !view.read(cx).matches(&model))
            {
                let entity = cx.new(|cx| {
                    surface_view::SurfaceView::new(
                        model.clone(),
                        true,
                        self.engine.commands.clone(),
                        self.engine.snapshot.clone(),
                        window,
                        cx,
                    )
                });
                entity.update(cx, |view, cx| view.focus_initial(window, cx));
                self.workspace_view = Some(entity);
            }
            div()
                .size_full()
                .child(self.workspace_view.as_ref().unwrap().clone())
                .into_any_element()
        } else {
            self.radio_browser = None;
            self.workspace_view = None;
            self.columns(single, window, cx).into_any_element()
        };
        let browser = if self.snapshot.workspace_title.as_deref() == Some("Describe Music")
            && self.snapshot.workspace.is_none()
        {
            let empty_history =
                self.snapshot.columns.len() == 1 && self.snapshot.columns[0].rows.is_empty();
            let body = if empty_history {
                div()
                    .flex()
                    .size_full()
                    .child(div().w(px(240.)).h_full().child(browser))
                    .child(
                        div()
                            .id("description-examples")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_5()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(super::workspace::DESCRIPTION_EXAMPLES.iter().map(|text| {
                                gpui_kit::component::label::Label::new(format!("“{text}”"))
                                    .text_color(cx.theme().muted_foreground)
                            })),
                    )
                    .into_any_element()
            } else {
                browser
            };
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_none()
                        .p_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().w_full().max_w(px(560.)).child(
                                gpui_kit::component::input::Input::new(&self.description_input)
                                    .small()
                                    .aria_label("Music description")
                                    .cleanable(true),
                            ),
                        )
                        .child(
                            Button::new("submit-description")
                                .debug_selector(|| "submit-description".into())
                                .small()
                                .label("Search")
                                .disabled(self.description_input.read(cx).value().trim().is_empty())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_description(window, cx)
                                })),
                        ),
                )
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element()
        } else {
            browser
        };
        browser
    }

    fn columns(&mut self, single: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if single && self.snapshot.sheet_shows_inspector() {
            return self
                .inspector(None, self.details_scroll.clone(), cx)
                .into_any_element();
        }
        let selected = self
            .snapshot
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.hidden && !c.metadata)
            .min_by_key(|(i, _)| i.abs_diff(self.snapshot.focused_column))
            .map(|(i, _)| i);
        let count = self
            .snapshot
            .columns
            .iter()
            .filter(|column| !column.metadata && !column.hidden)
            .count();
        let panel_count = count + usize::from(self.snapshot.has_inspector());
        let reveal_owner = cx.entity().downgrade();
        let mut columns = h_resizable(SharedString::from(format!(
            "miller-columns-{}-{}",
            self.layout_epoch, self.snapshot.generation
        )));
        let panel_state = window.use_keyed_state(
            SharedString::from(format!(
                "miller-sizing-{}-{}",
                self.layout_epoch, self.snapshot.generation
            )),
            cx,
            |_, _| gpui_kit::component::resizable::ResizableState::default(),
        );
        self.size_columns(&panel_state, window, cx);
        columns = columns.with_state(&panel_state);
        for (index, column_snapshot) in self
            .snapshot
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.hidden)
        {
            if column_snapshot.metadata || (single && Some(index) != selected) {
                continue;
            }
            let owner = cx.entity().downgrade();
            let title =
                crate::util::column_heading::capitalized(&browser::heading(column_snapshot));
            let heading_width = window.use_keyed_state(
                SharedString::from(format!("column-heading-width-{}", column_snapshot.identity)),
                cx,
                |_, _| px(0.),
            );
            let available = *heading_width.read(cx);
            let mut heading_style = window.text_style();
            heading_style.font_family = cx.theme().font_family.clone();
            heading_style.font_weight = if column_snapshot.artist_header.is_some() {
                FontWeight::BOLD
            } else {
                FontWeight::SEMIBOLD
            };
            let measure = |label: &str, size| {
                window
                    .text_system()
                    .shape_line(
                        label.to_owned().into(),
                        size,
                        &[heading_style.to_run(label.len())],
                        None,
                    )
                    .width
            };
            let full_width = measure(&title, px(12.));
            let compact = available > px(0.) && full_width > available;
            let heading = if compact {
                browser::compact_heading(&title)
            } else {
                &title
            };
            let width = if heading == title {
                full_width
            } else {
                measure(heading, px(12.))
            };
            let shrink = compact && column_snapshot.named_heading && width > available;
            let font_size = if shrink { px(11.) } else { px(12.) };
            let width = if shrink {
                measure(heading, font_size)
            } else {
                width
            };
            let heading = heading.to_owned();
            let filtering = self.snapshot.filter.active && self.snapshot.filter.column == index;
            let moving =
                if !filtering && !cx.reduce_motion() && available > px(0.) && width > available {
                    let overflow = f32::from(width - available);
                    let clock = window.use_keyed_state(
                        SharedString::from(format!(
                            "header-scroll-{}-{heading}-{available:?}",
                            column_snapshot.identity
                        )),
                        cx,
                        |_, cx| HeaderClock::start(overflow, cx),
                    );
                    clock.read(cx).offset
                } else {
                    None
                };
            let heading = match moving {
                Some(offset) => div()
                    .relative()
                    .left(px(-offset))
                    .w(width)
                    .whitespace_nowrap()
                    .child(heading),
                None => div()
                    .w_full()
                    .truncate()
                    .text_ellipsis_middle()
                    .child(heading),
            };
            let entry_count = super::filter::indices(&self.snapshot, index)
                .map_or(column_snapshot.rows.len(), |indices| indices.len());
            let scroll = self
                .scroll_handles
                .entry(column_snapshot.identity.clone())
                .or_default()
                .clone();
            // Keep a normal scrollable header even when the catalog is shorter
            // than its viewport. Empty slots are layout space, never selectable rows.
            let viewport = scroll.viewport_bounds().size.height;
            let viewport = if viewport > px(0.) {
                viewport
            } else {
                self.initial_column_height
            };
            let slots = (f32::from(viewport) / column_snapshot.row_height()).ceil() as usize;
            let grid = self.grid(&self.snapshot, index);
            let reflow = window.use_keyed_state(
                SharedString::from(format!(
                    "grid-reflow-{}-{}",
                    self.snapshot.generation, column_snapshot.identity
                )),
                cx,
                |_, _| crate::ui::motion::GridReflow::default(),
            );
            let opacity = reflow.update(cx, |motion, _| {
                motion.sample(grid.columns, std::time::Instant::now())
            });
            let opacity = if cx.reduce_motion() { 1. } else { opacity };
            if opacity < 1. {
                window.request_animation_frame();
            }
            let bands = if entry_count == 0 {
                0
            } else {
                grid.band(entry_count - 1) + 1
            };
            let content_rows = bands + usize::from(column_snapshot.album_info);
            let content_overflows =
                px(content_rows as f32 * column_snapshot.row_height()) > viewport;
            let shared_scroll =
                column_snapshot.artist_header.is_some() || column_snapshot.album_page.is_some();
            let page_scroll = window
                .use_keyed_state(
                    SharedString::from(format!(
                        "album-scroll-{}-{}",
                        self.snapshot.generation, column_snapshot.identity
                    )),
                    cx,
                    |_, _| ScrollHandle::new(),
                )
                .read(cx)
                .clone();
            let size = if shared_scroll {
                bands
            } else {
                bands.max(slots)
            } + super::filter::header_count(&self.snapshot, index);
            let list = uniform_list(("column-list", index), size, move |range, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        range
                            .map(|row| {
                                if grid.columns > 1 && row >= grid.first {
                                    let start = grid.start(row);
                                    div()
                                        .grid()
                                        .grid_cols(grid.columns as u16)
                                        .w_full()
                                        .children(
                                            (start..(start + grid.columns).min(entry_count))
                                                .filter_map(|pos| {
                                                    super::filter::row(&this.snapshot, index, pos)
                                                        .map(|raw| {
                                                            this.browser_row(
                                                                index, raw, None, window, cx,
                                                            )
                                                        })
                                                }),
                                        )
                                        .into_any_element()
                                } else if super::filter::album_info_index(&this.snapshot, index)
                                    == Some(row)
                                {
                                    this.album_info_row(index, cx)
                                } else {
                                    super::filter::row(&this.snapshot, index, row)
                                        .map(|raw| {
                                            this.browser_row(index, raw, Some(row), window, cx)
                                        })
                                        .unwrap_or_else(|| div().into_any_element())
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .with_decoration(ColumnViewport {
                page_scroll: shared_scroll.then(|| page_scroll.clone()),
                owner: cx.entity().downgrade(),
                column: index,
            })
            .h_full()
            .when(!shared_scroll, |list| list.track_scroll(&scroll));
            let rail_height = scroll.viewport_bounds().size.height;
            let rail_columns = if rail_height > px(0.) && rail_height < px(224.) {
                4
            } else if rail_height > px(0.) && rail_height < px(448.) {
                2
            } else {
                1
            };
            let rail_rows = 28_u16.div_ceil(rail_columns);
            let letter_height = if rail_height > px(0.) {
                ((rail_height - px(8.)) / rail_rows as f32).max(px(1.))
            } else {
                px(22.)
            };
            let show_index = !shared_scroll
                && !self.snapshot.filter.active
                && column_snapshot.alphabet_ascending.is_some()
                && self
                    .letter_shortcuts
                    .as_ref()
                    .is_some_and(|(key, _)| key == &column_snapshot.identity);
            let body = div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .debug_selector(move || format!("column-viewport-{index}"))
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .relative()
                        .overflow_hidden()
                        .py_1()
                        .child(list),
                )
                .when(show_index, |body| {
                    let generation = self.snapshot.generation;
                    let identity = column_snapshot.identity.clone();
                    let hover_identity = identity.clone();
                    body.child(
                        div()
                            .id(SharedString::from(format!("alphabet-rail-{index}")))
                            .debug_selector(move || format!("alphabet-rail-{index}"))
                            .absolute()
                            .top_0()
                            .right(px(12.))
                            .occlude()
                            .w(px(8. + 24. * rail_columns as f32))
                            .h_full()
                            .flex_none()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                                if let Some((key, until)) = this
                                    .letter_shortcuts
                                    .as_mut()
                                    .filter(|(key, _)| key == &hover_identity)
                                {
                                    let _ = key;
                                    *until = std::time::Instant::now()
                                        + if *hovered {
                                            std::time::Duration::from_secs(3600)
                                        } else {
                                            std::time::Duration::from_millis(1400)
                                        };
                                }
                            }))
                            .overflow_hidden()
                            .child(
                                div()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .when(rail_columns > 1, |rail| {
                                        rail.grid().grid_cols(rail_columns)
                                    })
                                    .items_start()
                                    .py_1()
                                    .pl_1()
                                    .children(
                                        (if column_snapshot.alphabet_ascending == Some(false) {
                                            "ZYXWVUTSRQPONMLKJIHGFEDCBA#%"
                                        } else {
                                            "%#ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                                        })
                                        .chars()
                                        .map(|letter| {
                                            let identity = identity.clone();
                                            Button::new(SharedString::from(format!(
                                                "jump-{index}-{letter}"
                                            )))
                                            .debug_selector(move || {
                                                format!("jump-{index}-{letter}")
                                            })
                                            .ghost()
                                            .xsmall()
                                            .label(letter.to_string())
                                            .flex_1()
                                            .min_h_0()
                                            .h(letter_height)
                                            .text_size(px(11.))
                                            .line_height(px(12.))
                                            .py_0()
                                            .w(px(24.))
                                            .tooltip(format!("Go to {letter}"))
                                            .on_click(cx.listener(move |this, _, _, _| {
                                                let _ = this.engine.commands.try_send(
                                                    Command::JumpLetter {
                                                        generation,
                                                        column: index,
                                                        identity: identity.clone(),
                                                        letter,
                                                        second: false,
                                                    },
                                                );
                                            }))
                                        }),
                                    ),
                            ),
                    )
                })
                .child(div().relative().w(px(12.)).h_full().flex_none().when(
                    content_overflows && !shared_scroll,
                    |rail| {
                        rail.bg(cx.theme().muted.opacity(0.2)).child(
                            Scrollbar::vertical(&scroll)
                                .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                                .viewport_from_layout(),
                        )
                    },
                ));
            let body = if column_snapshot.album_page.is_some()
                || column_snapshot.artist_header.is_some()
            {
                let height = bands as f32 * column_snapshot.row_height() + 8.;
                let tracks = div()
                    .flex_none()
                    .h(px(height))
                    .flex()
                    .child(body)
                    .into_any_element();
                let scroll = page_scroll;
                if let Some((key, name, text)) = &column_snapshot.artist_header {
                    let provider = column_snapshot.bio_search;
                    let action: crate::app::Action = if let Some(provider) = provider {
                        crate::app::action::SystemAction::OpenBrowser(provider.artist_url(name))
                            .into()
                    } else {
                        crate::app::action::SearchAction::ShowArtistBio {
                            artist_key: key.clone(),
                            artist_name: name.clone(),
                        }
                        .into()
                    };
                    let generation = self.snapshot.generation;
                    let header = div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .p_2()
                        .gap_2()
                        .child(
                            self.recommendation_art(Some(key), true)
                                .debug_selector(move || format!("artist-portrait-{index}"))
                                .size(px(184.))
                                .max_w_full(),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .w_full()
                                .flex()
                                .flex_col()
                                .items_end()
                                .when(text.is_empty(), |actions| actions.items_center())
                                .when(!text.is_empty(), |preview| {
                                    preview.child(
                                        div()
                                            .debug_selector(move || {
                                                format!("artist-bio-excerpt-{index}")
                                            })
                                            .w_full()
                                            .line_height(px(18.))
                                            .max_h(px(54.))
                                            .overflow_hidden()
                                            .line_clamp(3)
                                            .child(
                                                text.lines().next().unwrap_or_default().to_owned(),
                                            ),
                                    )
                                })
                                .child(
                                    Button::new(("artist-header-bio", index))
                                        .debug_selector(move || {
                                            format!("artist-header-bio-{index}")
                                        })
                                        .ghost()
                                        .small()
                                        .when_some(provider, |button, provider| {
                                            button
                                                .child(super::provider_logo(provider))
                                                .tooltip(provider.search_label())
                                        })
                                        .when(provider.is_none(), |button| button.label("…more"))
                                        .accessibility_label(
                                            provider
                                                .map_or("Artist Biography", |p| p.search_label()),
                                        )
                                        .on_click(cx.listener(move |this, _, _, _| {
                                            let _ = this.engine.commands.try_send(
                                                Command::GuardedActions {
                                                    generation,
                                                    target: None,
                                                    actions: vec![action.clone().into()],
                                                },
                                            );
                                        })),
                                ),
                        );
                    div()
                        .id(("artist-albums-page", index))
                        .debug_selector(move || format!("artist-page-{index}"))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .vertical_scrollbar(&scroll)
                        .child(header)
                        .child(tracks)
                        .when(!column_snapshot.similar_artists.is_empty(), |page| {
                            page.child(
                                div()
                                    .child(div().px_2().py_2().child("Similar Artists"))
                                    .child(self.related_artist_grid(
                                        &column_snapshot.similar_artists,
                                        self.grid_columns_for(&self.snapshot, index),
                                        cx,
                                    )),
                            )
                        })
                        .into_any_element()
                } else {
                    self.inspector(Some((index, tracks)), scroll, cx)
                        .into_any_element()
                }
            } else {
                body.into_any_element()
            };
            let column = div()
                .debug_selector(move || format!("browser-column-{index}"))
                .size_full()
                .flex()
                .flex_col()
                .bg(cx.theme().background)
                .child(
                    div()
                        .flex_none()
                        .h(super::theme::column_header_height(self.snapshot.theme))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .bg(cx.theme().muted.opacity(0.35))
                        .when(
                            self.snapshot.theme != crate::app::theme::ThemeName::BrushedAluminum,
                            |header| header.border_b_1().border_color(cx.theme().border),
                        )
                        .child(
                            if self.snapshot.filter.active && self.snapshot.filter.column == index {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.column_filter(index, cx))
                                    .into_any_element()
                            } else {
                                div()
                                    .id(("column-heading", index))
                                    .focusable()
                                    .tab_stop(true)
                                    .focus(|style| {
                                        style.border_1().border_color(cx.theme().selection)
                                    })
                                    .debug_selector(move || {
                                        format!(
                                            "column-heading-{index}-{}",
                                            if shrink {
                                                "small"
                                            } else if compact {
                                                "compact"
                                            } else {
                                                "full"
                                            }
                                        )
                                    })
                                    .text_size(font_size)
                                    .font_weight(heading_style.font_weight)
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_ellipsis_middle()
                                    .aria_label(format!("Filter {title}"))
                                    .role(Role::Button)
                                    .cursor_pointer()
                                    .on_click({
                                        let identity = column_snapshot.identity.clone();
                                        cx.listener(move |this, _, _, _| {
                                            let _ = this.engine.commands.try_send(
                                                Command::ToggleColumnFilter {
                                                    generation: this.snapshot.generation,
                                                    column: index,
                                                    identity: identity.clone(),
                                                },
                                            );
                                        })
                                    })
                                    .on_prepaint(move |bounds, window, cx| {
                                        if *heading_width.read(cx) != bounds.size.width {
                                            let heading_width = heading_width.clone();
                                            window.defer(cx, move |window, cx| {
                                                heading_width.update(cx, |width, _| {
                                                    *width = bounds.size.width
                                                });
                                                window.refresh();
                                            });
                                        }
                                    })
                                    .child(heading)
                                    .into_any_element()
                            },
                        )
                        .when(column_snapshot.album_page.is_none(), |header| {
                            header.children(self.column_action_buttons(index, column_snapshot, cx))
                        })
                        .when_some(column_snapshot.album_page.as_ref(), |header, (_, page)| {
                            let mut header = header.children(
                                self.column_action_buttons(index, column_snapshot, cx)
                                    .into_iter()
                                    .skip(1),
                            );
                            if let Some(download) = page.controls.iter().find(|control| {
                                matches!(
                                    control.actions.first(),
                                    Some(crate::app::Action::System(
                                        crate::app::action::SystemAction::Downloads(_)
                                    ))
                                )
                            }) {
                                let actions = download.actions.as_ref().clone();
                                let generation = self.snapshot.generation;
                                header = header.child(
                                    Button::new(("album-download", index))
                                        .ghost()
                                        .small()
                                        .icon(Icon::new(Glyph::Download))
                                        .tooltip("Download Album")
                                        .accessibility_label("Download Album")
                                        .on_click(cx.listener(move |this, _, _, _| {
                                            let _ = this.engine.commands.try_send(
                                                Command::GuardedActions {
                                                    generation,
                                                    target: None,
                                                    actions: actions.clone(),
                                                },
                                            );
                                        })),
                                );
                            }
                            header
                        })
                        .when(column_snapshot.has_view_options, |header| {
                            header.child(
                                Button::new(("column-options", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Glyph::Settings2))
                                    .tooltip("View Options")
                                    .on_click({
                                        let generation = self.snapshot.generation;
                                        let identity = column_snapshot.identity.clone();
                                        cx.listener(move |this, _, window, cx| {
                                            this.focus.focus(window, cx);
                                            let _ = this.engine.commands.try_send(
                                                Command::ColumnOptions {
                                                    generation,
                                                    column: index,
                                                    identity: identity.clone(),
                                                },
                                            );
                                        })
                                    }),
                            )
                        })
                        .child(
                            Button::new(("close-column", index))
                                .debug_selector(move || format!("close-column-{index}"))
                                .ghost()
                                .xsmall()
                                .p_0()
                                .size(px(18.))
                                .icon(Icon::new(Glyph::X).size(px(10.)))
                                .accessibility_label(format!(
                                    "Close {} column",
                                    browser::heading(column_snapshot)
                                ))
                                .tooltip("Close Column · ⌘[")
                                .on_click({
                                    let generation = self.snapshot.generation;
                                    let identity = column_snapshot.identity.clone();
                                    cx.listener(move |this, _, window, cx| {
                                        this.focus.focus(window, cx);
                                        if this.snapshot.sheet_browser {
                                            this.sheet_sidebar_root = true;
                                            cx.notify();
                                        }
                                        let _ =
                                            this.engine.commands.try_send(Command::CloseColumn {
                                                generation,
                                                column: index,
                                                identity: identity.clone(),
                                            });
                                    })
                                }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .opacity(opacity)
                        .child(body),
                );
            if single {
                return column.into_any_element();
            }
            let content = super::motion::pane(
                super::motion::PaneKey::Column(index),
                column.into_any_element(),
                cx.entity().downgrade(),
                self.pane_presented
                    .get(&super::motion::PaneKey::Column(index))
                    .copied(),
                self.column_motion_until
                    .is_some_and(|until| std::time::Instant::now() < until),
            );
            columns = columns.child(
                resizable_panel()
                    .size(px(column_layout::MIN_COLUMN))
                    .size_range(px(180.)..Pixels::MAX)
                    .child(content),
            );
        }
        if self.snapshot.has_inspector() {
            let details = super::motion::pane(
                super::motion::PaneKey::Inspector,
                self.inspector(None, self.details_scroll.clone(), cx)
                    .into_any_element(),
                cx.entity().downgrade(),
                self.pane_presented
                    .get(&super::motion::PaneKey::Inspector)
                    .copied(),
                self.column_motion_until
                    .is_some_and(|until| std::time::Instant::now() < until),
            );
            columns = columns.child(
                resizable_panel()
                    .size(px(320.))
                    .size_range(px(180.)..Pixels::MAX)
                    .child(details),
            );
        }
        let body = if self.snapshot.preview.is_none()
            && self.snapshot.loading.is_none()
            && (count == 0 || self.snapshot.columns.iter().all(|c| c.rows.is_empty()))
        {
            let message = if self.snapshot.libraries.is_empty() {
                Some((
                    "Add your music library",
                    "Connect a server or choose a local music folder.",
                ))
            } else if self.snapshot.section == "Queue" {
                Some((
                    "Queue is empty",
                    "Choose a track in your library to start listening.",
                ))
            } else {
                self.snapshot
                    .error
                    .as_deref()
                    .map(|error| ("Could not load this view", error))
            };
            let media = div().debug_selector(|| "library-empty-state".into());
            let header =
                if let Some((title, description)) = message {
                    EmptyHeader::new()
                        .media(EmptyMedia::new().child(media.child(Icon::new(Glyph::LibraryBig))))
                        .title(EmptyTitle::new().child(title))
                        .description(EmptyDescription::new().child(description.to_owned()))
                } else {
                    EmptyHeader::new()
                        .max_w(px(660.))
                        .media(
                            EmptyMedia::new().w_full().child(
                                media.w_full().h(px(128.)).flex().child(
                                    super::about::themed_artwork("library-empty-wordmark", cx),
                                ),
                            ),
                        )
                        .description(
                            EmptyDescription::new()
                                .child("“Why be bleak when you can be Blake?” — john balance"),
                        )
                };
            EmptyState::new()
                .header(header)
                .when(
                    !self.snapshot.libraries.is_empty() && self.snapshot.error.is_some(),
                    |empty| {
                        empty.child(
                            Button::new("retry-library")
                                .debug_selector(|| "retry-library".into())
                                .label("Retry")
                                .on_click(cx.listener(|this, _, _, _| {
                                    this.command(PaletteCommandKind::Refresh)
                                })),
                        )
                    },
                )
                .when(self.snapshot.libraries.is_empty(), |empty| {
                    empty.child(
                        Button::new("empty-settings")
                            .label("Open Settings")
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    )
                })
                .into_any_element()
        } else {
            div()
                .size_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("column-strip")
                        .debug_selector(|| "column-strip".into())
                        .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.column_motion_until = None;
                            this.queue_focused = false;
                            cx.notify();
                        }))
                        .relative()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_x_scroll()
                        .track_scroll(&self.column_scroll)
                        .on_prepaint(move |bounds, window, cx| {
                            let sizes = panel_state.read(cx).sizes();
                            if let Some(index) = reveal_owner
                                .read_with(cx, |view, _| {
                                    if view.column_viewport_width != bounds.size.width {
                                        panel_count.checked_sub(1)
                                    } else {
                                        view.column_reveal
                                    }
                                })
                                .ok()
                                .flatten()
                                && index < sizes.len()
                                && sizes.len() == panel_count
                                && sizes.iter().all(|size| *size > px(0.))
                            {
                                let owner = reveal_owner.clone();
                                let viewport_width = bounds.size.width;
                                window.defer(cx, move |_, cx| {
                                    let _ = owner.update(cx, |view, cx| {
                                        if view.column_viewport_width != viewport_width {
                                            view.column_viewport_width = viewport_width;
                                            view.column_reveal = Some(index);
                                        }
                                        if view.column_reveal != Some(index) {
                                            return;
                                        }
                                        let offset = view.column_scroll.offset();
                                        let rightmost = -view.column_scroll.max_offset().x;
                                        if (offset.x - rightmost).abs() < px(0.5) {
                                            view.column_reveal = None;
                                        } else {
                                            view.column_scroll
                                                .set_offset(point(rightmost, offset.y));
                                        }
                                        cx.notify();
                                    });
                                });
                            }
                        })
                        .child(
                            div()
                                .size_full()
                                .min_w(px(column_layout::MIN_COLUMN
                                    * (count + usize::from(self.snapshot.has_inspector())) as f32))
                                .child(columns),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(12.))
                        .flex_shrink_0()
                        .bg(cx.theme().background)
                        .debug_selector(|| "column-horizontal-scrollbar".into())
                        .child(
                            Scrollbar::horizontal(&self.column_scroll)
                                .id("column-horizontal-scrollbar")
                                .mode(ScrollbarMode::Always)
                                .viewport_from_layout()
                                .styles(|style| {
                                    style
                                        .track(|track| track.bg(cx.theme().muted).width(px(12.)))
                                        .thumb(|thumb| {
                                            thumb
                                                .bg(cx.theme().muted_foreground)
                                                .width(px(8.))
                                                .inset(px(2.))
                                        })
                                }),
                        ),
                )
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    pub(super) fn status_strip(
        &mut self,
        integrated: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let error = self.snapshot.error.is_some();
        let message = if self.snapshot.error.is_none() && self.snapshot.loading.is_some() {
            self.snapshot.loading.clone().unwrap_or_default()
        } else if self.snapshot.status.is_empty() {
            self.snapshot.loading.clone().unwrap_or_else(|| {
                if self.snapshot.player.active {
                    if self.snapshot.player.message.is_empty() {
                        self.snapshot.player.station.clone()
                    } else {
                        self.snapshot.player.message.clone()
                    }
                } else {
                    String::new()
                }
            })
        } else {
            self.snapshot.status.clone()
        };
        let count = self.snapshot.item_summary.clone().unwrap_or_default();
        let footer_height = if integrated {
            super::motion::FOOTER_HEIGHT
        } else {
            30.
        };
        let show_message = !message.is_empty();
        let displayed = message.clone();
        StatusBar::new()
            .h(px(footer_height))
            .flex_none()
            .px_0()
            .py_0()
            .gap_0()
            .bg(cx.theme().title_bar)
            .border_color(cx.theme().border)
            .when(integrated, |row| row.border_t_0().bg(transparent_black()))
            .when(!integrated, |footer| {
                footer.left(self.navigation_back_button(false, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .px_3()
                    .when(
                        self.snapshot.loading.is_some()
                            && message == self.snapshot.loading.clone().unwrap_or_default(),
                        |row| {
                            row.child(
                                div()
                                    .debug_selector(|| "utility-loading".into())
                                    .child(Spinner::new().xsmall()),
                            )
                        },
                    )
                    .child(
                        div()
                            .id("transport-status-message")
                            .debug_selector(|| "utility-status".into())
                            .min_w_0()
                            .text_right()
                            .truncate()
                            .when(error, |text| text.text_color(cx.theme().danger))
                            .when(self.snapshot.download_notification, |text| {
                                text.cursor_pointer().on_click(cx.listener(|this, _, _, _| {
                                    this.command(PaletteCommandKind::Downloads(
                                        crate::app::downloads::Menu::Open,
                                    ))
                                }))
                            })
                            .child(if show_message {
                                displayed
                            } else {
                                String::new()
                            }),
                    )
                    .when(error, |row| {
                        row.child(
                            Button::new("dismiss-status-error")
                                .ghost()
                                .xsmall()
                                .size(px(20.))
                                .icon(Icon::new(Glyph::X).size(px(14.)))
                                .tooltip("Dismiss Error")
                                .on_click(cx.listener(|this, _, _, _| {
                                    let _ = this.engine.commands.try_send(Command::Action(
                                        crate::app::action::SystemAction::ClearError.into(),
                                    ));
                                })),
                        )
                    }),
            )
            .right(
                div()
                    .debug_selector(|| "utility-queue-segment".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_3()
                    .when(!count.is_empty(), |row| {
                        row.child(
                            div()
                                .debug_selector(|| "utility-item-count".into())
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(count),
                        )
                    }),
            )
    }

    fn close_column_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.engine.commands.try_send(Command::Action(
            crate::app::action::SearchAction::DeactivateListFilter.into(),
        ));
        self.syncing_filter = true;
        self.filter_expected.clear();
        self.filter_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.syncing_filter = false;
        self.focus.focus(window, cx);
    }

    fn column_filter(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("column-filter", index))
            .debug_selector(move || format!("column-filter-{index}"))
            .w_full()
            .flex_none()
            .h(px(28.))
            .flex()
            .items_center()
            .child(
                gpui_kit::component::input::Input::new(&self.filter_input)
                    .small()
                    .w_full()
                    .suffix(
                        Button::new(("close-column-filter", index))
                            .debug_selector(move || format!("close-column-filter-{index}"))
                            .ghost()
                            .size(px(16.))
                            .icon(Icon::new(Glyph::X).size(px(10.)))
                            .accessibility_label("Close Column Filter")
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_column_filter(window, cx)
                            })),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // The entire frame focuses the editor, including its padding.
                    // A frame-only focus would leave list shortcuts active.
                    this.filter_input
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_kit::component::input::MoveUp, _, cx| {
                    let _ = this.engine.commands.try_send(Command::CoreKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Up,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    cx.stop_propagation();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_kit::component::input::MoveDown, _, cx| {
                    let _ = this.engine.commands.try_send(Command::CoreKey(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Down,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                    ));
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_column_filter(window, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }

    pub(super) fn browser_row(
        &mut self,
        column: usize,
        row: usize,
        display_row: Option<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = (if column == browser::PLAYER_QUEUE {
            self.snapshot.playback_queue.as_ref()
        } else {
            self.snapshot.columns.get(column)
        }) else {
            return div().into_any_element();
        };
        let Some(item) = data.rows.get(row) else {
            return div().into_any_element();
        };
        let Some(target) = data.target(self.snapshot.generation, column, row) else {
            return div().into_any_element();
        };
        let caption = crate::app::view_options::album_caption(&item.title, item.year);
        let below = data.artwork_visible && item.kind == browser::Kind::Album && data.album_grid;
        let caption = if below && let Some(width) = self.grid_widths.get(&data.identity) {
            let available = *width / self.grid_columns_for(&self.snapshot, column) as f32 - px(8.);
            let mut style = window.text_style();
            style.font_family = cx.theme().font_family.clone();
            let fits = |line: &str| {
                window
                    .text_system()
                    .shape_line(
                        line.to_owned().into(),
                        px(13.),
                        &[style.to_run(line.len())],
                        None,
                    )
                    .width
                    <= available
            };
            if !fits(&caption)
                && let Some((a, b)) = crate::app::view_options::caption_lines(&caption)
                && fits(a)
                && fits(b)
            {
                format!("{a}\n{b}").into()
            } else {
                caption
            }
        } else {
            caption
        };
        let queue_row = column == browser::PLAYER_QUEUE;
        let row_height = if queue_row { 40. } else { data.row_height() };
        let cover_size = if queue_row {
            32.
        } else if data.album_grid {
            browser::GRID_COVER_SIZE
        } else {
            crate::ui::layout::SMALL_ARTWORK
        };
        let row_image =
            if data.artwork_visible && !(data.album_grid && item.kind == browser::Kind::Folder) {
                self.cached_artwork(data.artwork_keys.get(&row).unwrap_or(&item.key))
            } else {
                None
            };
        let pending = item.download_id.is_some();
        let download_action = item.download_action;
        let cancel_target = target.clone();
        let cancel_sender = self.engine.commands.clone();
        let subtitle = item.download_status.clone().unwrap_or_else(|| {
            data.tracks
                .get(row)
                .filter(|_| item.kind == browser::Kind::Track)
                .map(|t| format!("{} · {}", t.track_artist(), t.album_name()))
                .unwrap_or_default()
        });
        let editable_playlist = data.editable_playlist;
        let editing = data.playlist_editing;
        let checked = data.selected_set.contains(&row);
        let playlist_len = data.tracks.len();
        let playlist_edits = crate::app::sources::navidrome::playlist_edit::Edit::ALL.map(|edit| {
            (
                edit,
                edit.available_for(row, &data.selected_set, playlist_len),
            )
        });
        let multiple_tracks = data.selected_set.len() > 1 && data.selected_set.contains(&row);
        let add_album =
            self.snapshot.context_policy.server_playlists && item.key.starts_with("navidrome:");
        let context = data
            .tracks
            .get(row)
            .filter(|_| item.kind == browser::Kind::Track)
            .map(|track| {
                crate::app::track_context::track_context_with_policy(
                    crate::app::track_context::ContextPolicy {
                        queued: column == browser::PLAYER_QUEUE,
                        ..self.snapshot.context_policy
                    },
                    track,
                    false,
                )
            })
            .unwrap_or_default();
        let drag_target = target.clone();
        let drop_target = target.clone();
        let hover_target = target.clone();
        let drop_after = self
            .queue_drop_after
            .as_ref()
            .is_some_and(|(t, after)| t == &target && *after);
        let drag_label = item.title.clone();
        let queue = column == browser::PLAYER_QUEUE
            || matches!(self.snapshot.section.as_str(), "Queue" | "Now Playing");
        let target_click = target.clone();
        let target_right = target.clone();
        let selected = !data.album_info_selected
            && if data.selected_set.is_empty() && !editing {
                row == data.selected
            } else {
                data.selected_set.contains(&row)
            };
        let focused = column == browser::PLAYER_QUEUE
            || (self.snapshot.sidebar_cursor.is_none() && column == self.snapshot.focused_column);
        let kind = item.kind;
        let is_playing = data.playing == Some(row);
        let icon = if is_playing {
            Glyph::Play
        } else {
            match kind {
                browser::Kind::Artist => Glyph::UserRound,
                browser::Kind::Album => Glyph::Disc3,
                browser::Kind::Track => Glyph::Music2,
                browser::Kind::Genre => Glyph::Tags,
                browser::Kind::Playlist => Glyph::ListMusic,
                browser::Kind::Folder => Glyph::Folder,
                browser::Kind::Description => Glyph::Search,
                browser::Kind::Info => Glyph::Info,
            }
        };
        let background = display_row
            .filter(|_| !below)
            .and_then(|index| super::theme::aluminum_list_stripe(self.snapshot.theme, index))
            .unwrap_or(cx.theme().background);
        ListItem::new(("item", row))
            .bg(background)
            .debug_selector({
                let title = item.title.clone();
                move || format!("browser-row-{title}")
            })
            .role(Role::ListItem)
            .aria_selected(selected)
            .accessibility_label(
                if item.kind == browser::Kind::Artist
                    && self.snapshot.ghost_artist.as_ref() == Some(&item.key)
                {
                    format!("{} · Compilation-only artist", item.title)
                } else if is_playing {
                    format!("{} · Now Playing", item.title)
                } else {
                    item.title.clone()
                },
            )
            .selected(selected && focused && !self.rapid_navigation && !pending)
            .secondary_selected(selected && (!focused || pending))
            .text_color(if pending {
                cx.theme().muted_foreground
            } else if selected && focused && !self.rapid_navigation {
                super::theme::selection_foreground(cx.theme().foreground, cx.theme().list_active)
            } else {
                cx.theme().foreground
            })
            .h(px(row_height))
            .w_full()
            .when(!below && !queue_row, |item| item.mx_1())
            .when(queue_row, |item| item.px_1().min_w_0().overflow_hidden())
            .when(below, |item| {
                item.min_w_0().overflow_hidden().px_1().justify_start()
            })
            .rounded(px(4.))
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(editing, |card| {
                        card.child(
                            gpui_kit::component::checkbox::Checkbox::new(("playlist-check", row))
                                .debug_selector(move || format!("playlist-checkbox-{column}-{row}"))
                                .accessibility_label(item.title.clone())
                                .checked(checked)
                                .mixed(data.partial_albums.contains(&row))
                                .on_change({
                                    let target = target.clone();
                                    let commands = self.engine.commands.clone();
                                    move |_, _, cx| {
                                        cx.stop_propagation();
                                        let _ = commands.try_send(Command::Select {
                                            target: target.clone(),
                                            mode: browser::Selection::Check,
                                            origin: browser::Origin::Pointer,
                                            intent: browser::Intent::Focus,
                                        });
                                    }
                                }),
                        )
                    })
                    .when(below, |card| {
                        card.flex_col()
                            .items_center()
                            .justify_start()
                            .h(px(row_height - 8.))
                    })
                    .w_full()
                    .gap_2()
                    .when(queue_row, |row| row.gap_1())
                    .min_w_0()
                    .when(!queue_row || row_image.is_some() || is_playing, |card| {
                        card.child(
                            if data.artwork_visible && item.kind == browser::Kind::Album {
                                Avatar::new()
                                    .debug_selector(move || {
                                        format!("browser-artwork-{column}-{row}")
                                    })
                                    .with_size(px(cover_size))
                                    .rounded(px(4.))
                                    .placeholder(
                                        Icon::new(Glyph::Disc3)
                                            .size(px(cover_size * 0.5))
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .when_some(row_image, |cover, image| cover.src(image))
                                    .into_any_element()
                            } else if let Some(image) = row_image {
                                img(image)
                                    .size(px(cover_size))
                                    .object_fit(ObjectFit::Contain)
                                    .into_any_element()
                            } else {
                                Icon::new(icon).size_4().into_any_element()
                            },
                        )
                    })
                    .child(
                        div()
                            .debug_selector({
                                let key = item.key.clone();
                                move || format!("browser-title-{key}")
                            })
                            .flex_1()
                            .when(below, |text| text.w_full().flex_none())
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .text_ellipsis()
                            .when(
                                item.kind == browser::Kind::Artist
                                    && self.snapshot.ghost_artist.as_ref() == Some(&item.key),
                                |text| {
                                    text.italic().when(!selected || !focused, |text| {
                                        text.text_color(cx.theme().muted_foreground)
                                    })
                                },
                            )
                            .child(
                                div()
                                    .when(below, |label| {
                                        label
                                            .whitespace_normal()
                                            .text_center()
                                            .line_height(px(16.))
                                            .line_clamp(3)
                                    })
                                    .child(caption.into_owned()),
                            )
                            .when(!subtitle.is_empty(), |row| {
                                row.child(div().text_size(px(11.)).text_ellipsis().child(subtitle))
                            }),
                    ),
            )
            .when(pending, |item| {
                item.suffix(move |_, _| {
                    let cancel_sender = cancel_sender.clone();
                    let cancel_target = cancel_target.clone();
                    Button::new("cancel-download")
                        .debug_selector(|| "download-cancel".into())
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Glyph::X).size_4())
                        .accessibility_label(crate::app::downloads::Menu::CancelSelection.label())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = cancel_sender.try_send(Command::Select {
                                target: cancel_target.clone(),
                                mode: browser::Selection::Single,
                                origin: browser::Origin::Pointer,
                                intent: browser::Intent::ManageDownload,
                            });
                        })
                })
            })
            .when(kind != browser::Kind::Track && !below, |item| {
                item.suffix(|_, _| Icon::new(Glyph::ChevronRight).size_3())
            })
            .when(queue, |item| {
                item.drag_over::<QueueDrag>(move |style, _, _, cx| {
                    if drop_after {
                        style.border_b_2().border_color(cx.theme().primary)
                    } else {
                        style.border_t_2().border_color(cx.theme().primary)
                    }
                })
                .on_drag_move(
                    cx.listener(move |this, event: &DragMoveEvent<QueueDrag>, _, cx| {
                        let after = event.event.position.y > event.bounds.center().y;
                        if this.queue_drop_after.as_ref() != Some(&(hover_target.clone(), after)) {
                            this.queue_drop_after = Some((hover_target.clone(), after));
                            cx.notify();
                        }
                    }),
                )
                .on_drag(
                    QueueDrag {
                        target: drag_target,
                        label: drag_label,
                    },
                    |drag, _, _, cx| cx.new(|_| drag.clone()),
                )
                .on_drop(cx.listener(move |this, drag: &QueueDrag, _, _| {
                    let _ = this.engine.commands.try_send(Command::QueueDrop {
                        from: drag.target.clone(),
                        to: drop_target.clone(),
                        after: this
                            .queue_drop_after
                            .take()
                            .is_some_and(|(target, after)| target == drop_target && after),
                    });
                }))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                this.scroll_pin = Some(target_click.identity.clone());
                let mode = if event.modifiers().shift {
                    browser::Selection::Extend
                } else if editing && kind == browser::Kind::Track {
                    browser::Selection::Check
                } else if event.modifiers().platform
                    || (!cfg!(target_os = "macos") && event.modifiers().control)
                {
                    browser::Selection::Toggle
                } else {
                    browser::Selection::Single
                };
                let _ = this.engine.commands.try_send(Command::Select {
                    target: target_click.clone(),
                    mode,
                    origin: browser::Origin::Pointer,
                    intent: if mode != browser::Selection::Single {
                        browser::Intent::Focus
                    } else if !editing && event.click_count() == 2 {
                        browser::Intent::Play
                    } else {
                        browser::Intent::Inspect
                    },
                });
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.focus.focus(window, cx);
                    this.scroll_pin = Some(target_right.identity.clone());
                    let _ = this.engine.commands.try_send(Command::Select {
                        target: target_right.clone(),
                        mode: browser::Selection::Context,
                        origin: browser::Origin::Pointer,
                        intent: browser::Intent::Focus,
                    });
                }),
            )
            .map(|item| {
                if pending {
                    return item
                        .context_menu(move |menu, _, _| {
                            menu.menu(
                                download_action.unwrap_or(
                                    crate::app::downloads::Menu::CancelSelection.label(),
                                ),
                                Box::new(ItemCommand {
                                    target: target.clone(),
                                    intent: browser::Intent::ManageDownload,
                                }),
                            )
                        })
                        .into_any_element();
                }
                if kind == browser::Kind::Description {
                    return item
                        .context_menu(move |menu, _, _| {
                            menu.menu(
                                "Remove from History",
                                Box::new(DescriptionContextCommand {
                                    target: target.clone(),
                                }),
                            )
                        })
                        .into_any_element();
                }
                if !matches!(
                    kind,
                    browser::Kind::Artist
                        | browser::Kind::Album
                        | browser::Kind::Playlist
                        | browser::Kind::Track
                ) {
                    return item.into_any_element();
                }
                item.context_menu(move |menu, _, _| {
                    let mut menu = menu;
                    if let Some(label) = download_action {
                        menu = menu
                            .menu(
                                label,
                                Box::new(ItemCommand {
                                    target: target.clone(),
                                    intent: browser::Intent::ManageDownload,
                                }),
                            )
                            .separator();
                    }
                    if editable_playlist && kind == browser::Kind::Track {
                        for (edit, available) in playlist_edits {
                            if available {
                                menu = menu.menu(
                                    edit.label(),
                                    Box::new(ItemCommand {
                                        target: target.clone(),
                                        intent: browser::Intent::EditPlaylist(edit),
                                    }),
                                );
                            }
                        }
                        menu = menu.separator();
                    }
                    if add_album && kind == browser::Kind::Album {
                        menu = menu
                            .menu(
                                "Add Album to Playlist…",
                                Box::new(ItemCommand {
                                    target: target.clone(),
                                    intent: browser::Intent::AddAlbumToPlaylist,
                                }),
                            )
                            .separator();
                    }
                    if column == browser::PLAYER_QUEUE {
                        for label in ["Remove Above", "Remove Below"] {
                            menu = menu.menu(
                                label,
                                Box::new(TrackContextCommand {
                                    target: target.clone(),
                                    label: label.into(),
                                }),
                            );
                        }
                        menu = menu.separator();
                    }
                    if !context.is_empty() {
                        let mut menu = menu;
                        for entry in &context {
                            if matches!(
                                entry.kind,
                                crate::app::track_context::ContextKind::Separator
                            ) {
                                menu = menu.separator();
                            } else {
                                menu = menu.menu(
                                    if multiple_tracks
                                        && matches!(
                                            entry.kind,
                                            crate::app::track_context::ContextKind::AddToPlaylist
                                        )
                                    {
                                        "Add Selected Tracks to Playlist…".to_owned()
                                    } else {
                                        entry.label.clone()
                                    },
                                    Box::new(TrackContextCommand {
                                        target: target.clone(),
                                        label: entry.label.clone(),
                                    }),
                                );
                            }
                        }
                        return menu;
                    }
                    menu.when(kind == browser::Kind::Artist, |menu| {
                        menu.menu(
                            "Open in Library",
                            Box::new(ArtistContextCommand {
                                target: target.clone(),
                                biography: true,
                            }),
                        )
                        .separator()
                        .menu(
                            "Artist Mix",
                            Box::new(ArtistContextCommand {
                                target: target.clone(),
                                biography: false,
                            }),
                        )
                    })
                    .when(kind == browser::Kind::Album, |menu| {
                        menu.menu(
                            "Open in Library",
                            Box::new(ItemCommand {
                                target: target.clone(),
                                intent: browser::Intent::OpenAlbumDetails,
                            }),
                        )
                        .separator()
                        .menu(
                            "Play Album",
                            Box::new(ItemCommand {
                                target: target.clone(),
                                intent: browser::Intent::Play,
                            }),
                        )
                    })
                    .when(kind == browser::Kind::Playlist, |menu| {
                        menu.menu(
                            "Play Playlist",
                            Box::new(ItemCommand {
                                target: target.clone(),
                                intent: browser::Intent::Play,
                            }),
                        )
                    })
                    .when(kind == browser::Kind::Track, |menu| {
                        menu.menu(
                            "Play",
                            Box::new(ItemCommand {
                                target: target.clone(),
                                intent: browser::Intent::Play,
                            }),
                        )
                    })
                    .when(
                        matches!(kind, browser::Kind::Album | browser::Kind::Playlist),
                        |menu| {
                            menu.separator().menu(
                                "Download",
                                Box::new(ItemCommand {
                                    target: target.clone(),
                                    intent: browser::Intent::Download,
                                }),
                            )
                        },
                    )
                })
                .into_any_element()
            })
            .into_any_element()
    }

    fn related_artist_grid(
        &self,
        artists: &[crate::library::models::Artist],
        columns: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .grid()
            .grid_cols(columns as u16)
            .pr(px(12.))
            .children(artists.iter().enumerate().map(|(index, artist)| {
                let tile = div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        self.recommendation_art(Some(&artist.rating_key), true)
                            .size(px(browser::GRID_COVER_SIZE))
                            .debug_selector(move || format!("related-artist-cover-{index}")),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .line_height(px(16.))
                            .line_clamp(2)
                            .child(artist.title.clone()),
                    );
                let track = crate::library::models::Track {
                    grandparent_rating_key: Some(artist.rating_key.clone()),
                    grandparent_title: Some(artist.title.clone()),
                    ..Default::default()
                };
                div().min_w_0().child(
                    ListItem::new(("related-artist", index))
                        .accessibility_label(artist.title.clone())
                        .h(px(browser::GRID_ROW_HEIGHT))
                        .px_1()
                        .text_size(px(13.))
                        .items_start()
                        .overflow_hidden()
                        .child(tile)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.command(PaletteCommandKind::JumpToArtist(Box::new(track.clone())))
                        })),
                )
            }))
    }
    fn recommendation_art(&self, key: Option<&str>, portrait: bool) -> gpui::Div {
        let image = key.and_then(|key| self.cached_artwork(key));
        div()
            .size(px(88.))
            .flex_none()
            .flex()
            .justify_center()
            .items_center()
            .overflow_hidden()
            .when(portrait, |art| art.rounded_full())
            .when(!portrait, |art| art.rounded_md())
            .child(match image {
                Some(image) => img(image)
                    .when(portrait, |image| image.rounded_full())
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => Icon::new(if portrait {
                    Glyph::UserRound
                } else {
                    Glyph::Disc3
                })
                .size(relative(0.55))
                .into_any_element(),
            })
    }

    fn inspector(
        &self,
        album_list: Option<(usize, AnyElement)>,
        scroll: ScrollHandle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let inline = album_list.as_ref().map(|(index, _)| *index);
        let recommendation_columns = inline.map_or(2, |i| self.grid_columns_for(&self.snapshot, i));
        let page = inline.and_then(|index| self.snapshot.columns[index].album_page.as_ref());
        let preview = page
            .map(|(_, page)| page.clone())
            .or_else(|| self.snapshot.preview.clone())
            .unwrap_or_default();

        let title = preview.title.clone();
        let target = self
            .snapshot
            .columns
            .get(self.snapshot.focused_column)
            .and_then(|c| {
                c.target(
                    self.snapshot.generation,
                    self.snapshot.focused_column,
                    c.selected,
                )
            });
        let cover = if let Some(image) =
            page.and_then(|(key, _)| self.cached_artwork(key))
                .or_else(|| {
                    self.inspector_art
                        .as_ref()
                        .filter(|_| inline.is_none() || self.snapshot.album_page_column() == inline)
                        .cloned()
                }) {
            img(image)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(Glyph::Disc3)
                        .size(px(64.))
                        .text_color(cx.theme().muted_foreground.opacity(0.5)),
                )
                .into_any_element()
        };
        let mut details = DescriptionList::horizontal()
            .columns(1)
            .small()
            .bordered(false)
            .label_width(px(95.));
        for (index, (label, value)) in preview
            .rows
            .iter()
            .filter(|(label, value)| !(label == "Artist" && value == &preview.subtitle))
            .enumerate()
        {
            if index > 0 {
                details = details.separator();
            }
            if let Some(track) = preview
                .track
                .as_ref()
                .filter(|_| label == "Artist" || label == "Album")
            {
                let track = track.clone();
                let artist = label == "Artist";
                details = details.item(
                    label.clone(),
                    Button::new(("detail-link", index))
                        .debug_selector(move || format!("detail-link-{index}"))
                        .ghost()
                        .small()
                        .p_0()
                        .h_auto()
                        .justify_start()
                        .w_full()
                        .min_w_0()
                        .accessibility_label(value.clone())
                        .child(
                            div()
                                .min_w_0()
                                .w_full()
                                .whitespace_normal()
                                .debug_selector(move || format!("detail-value-{index}"))
                                .text_left()
                                .hover(|style| style.text_decoration_1())
                                .child(value.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.command(if artist {
                                PaletteCommandKind::JumpToArtist(Box::new(track.clone()))
                            } else {
                                PaletteCommandKind::JumpToAlbum(Box::new(track.clone()))
                            })
                        }))
                        .into_any_element(),
                    1,
                );
            } else if label == "Genres" && !preview.genres.is_empty() {
                let generation = self.snapshot.generation;
                details = details.item(
                    label.clone(),
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(preview.genres.iter().enumerate().map(|(i, genre)| {
                            let genre = genre.clone();
                            Button::new(("detail-genre", i))
                                .ghost()
                                .small()
                                .p_0()
                                .h_auto()
                                .justify_start()
                                .accessibility_label(genre.clone())
                                .child(
                                    div()
                                        .flex_1()
                                        .text_left()
                                        .hover(|style| style.text_decoration_1())
                                        .child(genre.clone()),
                                )
                                .on_click(cx.listener(move |this, _, _, _| {
                                    let _ = this.engine.commands.try_send(Command::BrowseGenre {
                                        generation,
                                        genre: genre.clone(),
                                    });
                                }))
                        }))
                        .into_any_element(),
                    1,
                );
            } else {
                details = details.item(label.clone(), value.clone().into_any_element(), 1);
            }
        }
        div()
            .debug_selector(|| "details-pane".into())
            .when(inline.is_none(), |pane| pane.on_mouse_down(MouseButton::Left, cx.listener(|this,_,window,cx| {
                this.focus.focus(window,cx);
                let _=this.engine.commands.try_send(Command::FocusDetails {generation:this.snapshot.generation,navigation_serial:this.snapshot.navigation_serial});
            })))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .when(inline.is_none(), |pane| pane.child(
                div()
                    .h(super::theme::column_header_height(self.snapshot.theme))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .bg(cx.theme().muted.opacity(0.35))
                    .when(
                        self.snapshot.theme != crate::app::theme::ThemeName::BrushedAluminum,
                        |header| header.border_b_1().border_color(cx.theme().border),
                    )
                    .child(div().debug_selector(|| "details-header".into()).flex_1().min_w_0().truncate().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(preview.kind.label()))
                    .children(preview.controls.iter().enumerate().filter_map(|(index, control)| {
                        let glyph = control_icon(control)?;
                        let generation = self.snapshot.generation;
                        let target = target.clone();
                        let control = control.clone();
                        let menu_control = control.clone();
                        let menu_target = target.clone();
                        Some(Button::new(("details-header-action",index)).debug_selector(move || format!("details-control-{index}")).ghost().xsmall()
                            .map(|button| match control.search_provider { Some(provider) => button.child(super::provider_logo(provider)), None => button.icon(Icon::new(glyph)) }).accessibility_label(control.label.clone()).map(|button| { let mut button = button; gpui_kit::InteractiveElement::interactivity(&mut button).tooltip(control.tooltip()); button })
                            .on_click(cx.listener(move |this,event: &ClickEvent,_,_| {
                                let _=this.engine.commands.try_send(Command::GuardedActions {generation,target:target.clone(),actions:control.actions_for(event.modifiers().alt, event.modifiers().shift)});
                            })).map(|button| playback_menu(button, &menu_control, self.engine.commands.clone(), generation, menu_target)))
                    }))
                    .child(
                        Button::new("close-preview")
                            .debug_selector(|| "close-preview".into())
                            .ghost()
                            .xsmall()
                            .p_0()
                            .size(px(18.))
                            .icon(Icon::new(Glyph::X).size(px(10.)))
                            .accessibility_label("Close Details")
                            .tooltip("Close Column · ⌘[")
                            .on_click({
                                let generation = self.snapshot.generation;
                                let navigation_serial = self.snapshot.navigation_serial;
                                cx.listener(move |this, _, window, cx| {
                                    this.focus.focus(window, cx);
                                    if this.snapshot.sheet_browser { this.sheet_sidebar_root = true; cx.notify(); }
                                    let _ = this.engine.commands.try_send(Command::ClosePreview {
                                        generation,
                                        navigation_serial,
                                    });
                                })
                            }),
                    ),
            ))
            .child(
                div()
                    .id("details-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_5()
                    .track_scroll(&scroll)
                    .vertical_scrollbar(&scroll)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when(match preview.kind { super::catalog::PreviewKind::Track => preview.track_artwork, super::catalog::PreviewKind::Artist => preview.artist_artwork_visible, super::catalog::PreviewKind::Album => true }, |pane| pane.child(
                        div()
                            .debug_selector(|| "details-artwork".into())
                            .flex_none()
                            .w_full()
                            .max_w(px(320.))
                            .max_h(px(320.))
                            .mx_auto()
                            .mt_4()
                            .mb_4()
                            .aspect_square()
                            .rounded(px(7.))
                            .bg(cx.theme().muted)
                            .border_1()
                            .border_color(cx.theme().border)
                            .shadow_sm()
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .child(cover),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .debug_selector(|| "details-heading".into())
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if let Some(track) = preview.track.clone().filter(|_|!preview.subtitle.is_empty()) {
                                        Button::new("detail-artist").ghost().small().p_0().h_auto().justify_start().accessibility_label(preview.subtitle.clone())
                                            .child(div().flex_1().text_left().hover(|style|style.text_decoration_1()).child(preview.subtitle.clone()))
                                            .on_click(cx.listener(move |this,_,_,_|this.command(PaletteCommandKind::JumpToArtist(Box::new(track.clone()))))).into_any_element()
                                    }else{div().child(preview.subtitle.clone()).into_any_element()}),
                            ),
                    )
                    .when_some(inline, |pane, index| {
                        let mut notes = div().w_8();
                        if preview.album_info.as_ref().is_some_and(|info| !info.review.is_empty() || info.display_link().is_some()) {
                            let identity = self.snapshot.columns[index].identity.clone();
                            notes = notes.child(Button::new(("album-notes", index)).debug_selector(move || format!("album-notes-{index}")).ghost().large()
                                .icon(Icon::new(Glyph::Disc3)).tooltip("Album Info").accessibility_label("Album Info")
                                .on_click(cx.listener(move |this, _, _, _| {
                                    let _ = this.engine.commands.try_send(Command::AlbumInfo {
                                        generation: this.snapshot.generation, column: index, identity: identity.clone()
                                    });
                                })));
                        }
                        pane.child(div().flex().gap_2().justify_center().items_center()
                            .child(div().w_8().when_some(preview.artist_bio.clone(), |slot, bio| {
                                let generation = self.snapshot.generation;
                                slot.child(Button::new(("album-bio",index)).debug_selector(move || format!("album-bio-{index}")).ghost().large().map(|button| match bio.search_provider {Some(provider)=>button.child(super::provider_logo(provider)),None=>button.icon(Icon::new(Glyph::UserRound))})
                                    .tooltip(bio.label.clone()).accessibility_label(bio.label.clone())
                                    .on_click(cx.listener(move |this,_,_,_| {let _ = this.engine.commands.try_send(Command::GuardedActions {
                                        generation,target:None,actions:bio.actions.as_ref().clone()
                                    });})))
                            }))
                            .children(self.column_action_buttons(index, &self.snapshot.columns[index], cx).into_iter().take(1))
                            .child(notes))
                    })
                    .when(preview.loading,|view|view.child(Spinner::new().small()))
                    .when(!preview.rows.is_empty(), |view| {
                        view.child(
                            div()
                                .debug_selector(|| "details-metadata".into())
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(details),
                        )
                    })
                    .children(
                        preview
                            .controls
                            .into_iter()
                            .filter(|control| control_icon(control).is_none())
                            .enumerate()
                            .map(|(index, control)| {
                                self.command_button(
                                    format!("preview-{index}"),
                                    control,
                                    target.clone(),
                                    cx,
                                )
                            }),
                    )
                    .when_some(preview.lyrics, |pane, lyrics| pane.child(div().child("Lyrics").font_weight(FontWeight::SEMIBOLD)).child(div().child(lyrics)))
                    .when(!preview.text.is_empty(),|view|view.child(div().text_size(px(13.)).line_height(relative(1.5)).child(preview.text.clone())))
                    .when_some(preview.album_info.filter(|_| inline.is_none()), |mut view, info| {
                        let link = info.display_link().cloned();
                        if !info.review.is_empty() {
                            let mut text = String::new();
                            let mut emphasis = Vec::new();
                            for run in info.review {
                                let start = text.len();
                                text.push_str(&run.text);
                                if run.italic {
                                    emphasis.push((start..text.len(), gpui::HighlightStyle {
                                        font_style: Some(gpui::FontStyle::Italic), ..Default::default()
                                    }));
                                }
                            }
                            view = view.child(div().text_size(px(13.)).line_height(relative(1.5))
                                .child(gpui::StyledText::new(text).with_highlights(emphasis)));
                        }
                        if let Some(url) = link {
                            let label = url.host_str().unwrap_or("Album link");
                            view = view.child(gpui_kit::component::link::Link::new("album-source-link")
                                .href(url.to_string()).child(label.to_owned()));
                        }
                        view
                    })
                    .when_some(album_list, |view,(_,tracks)| view.child(tracks))
                    .when_some(preview.similar, |view,similar| {
                        let albums=similar.albums;
                        let seed_key=self.snapshot.preview.as_ref().and_then(|preview|preview.track.as_ref()).map(|track|track.rating_key.clone()).unwrap_or_default();
                        view.child(div().mt_2().pt_3().border_t_1().border_color(cx.theme().border).text_size(px(16.)).font_weight(FontWeight::BOLD).child(if albums {"Similar Albums"} else {"Similar Tracks"}))
                            .when(!similar.message.is_empty(),|view|view.child(div().text_size(px(12.)).text_color(cx.theme().muted_foreground).child(similar.message)))
                            .child(div().flex().when(albums,|grid|grid.grid().grid_cols(recommendation_columns as u16).pr(px(12.))).when(!albums,|grid|grid.flex_col()).children(similar.tracks.into_iter().enumerate().map(|(index,track)| {
                                let title=if albums {track.album_name().to_string()} else {track.title.clone()};
                                let seed_key=seed_key.clone();
                                let play=track.clone();let commands=self.engine.commands.clone();let generation=self.snapshot.generation;
                                let item=ListItem::new(("preview-similar",index)).debug_selector(move||format!("similar-track-{index}")).role(Role::ListItem).accessibility_label(title.clone()).child(div().flex().flex_col().gap_1().when(albums,|tile|tile.child(self.recommendation_art(track.parent_rating_key.as_deref(),false).size(px(browser::GRID_COVER_SIZE)))).child(title).child(div().text_size(px(11.)).text_color(cx.theme().muted_foreground).child(track.track_artist().to_string())))
                                    .suffix(move|_,_| {
                                        let play=play.clone();let commands=commands.clone();
                                        let menu_commands=commands.clone();
                                        let controls=if albums {crate::app::playback_controls::album_controls(play.parent_rating_key.as_deref().unwrap_or_default(),play.album_name())} else {crate::app::playback_controls::track_controls(&play,None)};
                                        let control=super::catalog::compact_controls(controls).remove(0);
                                        let click_control=control.clone();
                                        Button::new(("play-similar",index)).ghost().small().icon(Icon::new(Glyph::Play)).tooltip(if albums {"Play Album"} else {"Play Track"}).on_click(move|event: &ClickEvent,_,_| {
                                            let _=commands.try_send(Command::GuardedActions{generation,target:None,actions:click_control.actions_for(event.modifiers().alt,false)});
                                        }).map(|button|playback_menu(button,&control,menu_commands,generation,None))
                                    })
                                    .on_click(cx.listener(move |this,_,_,_| {
                                        if albums { let _=this.engine.commands.try_send(Command::GuardedActions {generation,target:None,actions:vec![crate::app::action::BrowseAction::OpenDetailsInLibrary {track:Box::new(track.clone()),destination:crate::app::library_details::Destination::Album}.into()]}); }
                                        else if inline.is_some() { let _=this.engine.commands.try_send(Command::GuardedActions {generation,target:None,actions:vec![crate::app::action::BrowseAction::OpenDetailsInLibrary {track:Box::new(track.clone()),destination:crate::app::library_details::Destination::Track}.into()]}); }
                                        else { let _=this.engine.commands.try_send(Command::GuardedActions {generation,target:None,actions:vec![crate::app::action::BrowseAction::OpenSimilarTrack {seed_key:seed_key.clone(),track_key:track.rating_key.clone()}.into()]}); }
                                    }));
                                if albums {div().min_w_0().child(item).into_any_element()}
                                else {item.into_any_element()}
                            })))
                    })
                    .when_some(preview.related_artists.filter(|artists|inline.is_none() || !artists.is_empty()), |view, artists| {
                        view.child(div().font_weight(FontWeight::SEMIBOLD).child(if inline.is_some() {"Similar Artists"} else {"Related Artists"}))
                            .child(self.related_artist_grid(&artists, recommendation_columns,cx))
                    }),
            )
    }

    pub(super) fn transport(&mut self, docked: bool, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::menu::DropdownMenu;
        let target = self.snapshot.playback_queue.as_ref().and_then(|queue| {
            queue.target(
                self.snapshot.generation,
                browser::PLAYER_QUEUE,
                queue.playing?,
            )
        });
        let context = self
            .snapshot
            .player
            .track
            .as_ref()
            .map(|track| {
                crate::app::track_context::now_playing_context(self.snapshot.context_policy, track)
            })
            .unwrap_or_default();
        let available = f32::from(self.library_size.width);
        let (wing, mut info_width) = crate::ui::layout::transport_widths(available);
        let compact_controls = wing < 184. || (docked && available < 1000.);
        let dense_controls = wing < 144. || (docked && available < 800.);
        if docked {
            let control_width = if dense_controls {
                198.
            } else if compact_controls {
                222.
            } else {
                260.
            };
            info_width = info_width.min((available - 82. - 2. * control_width).max(0.));
        }
        let wing = px(wing);
        let info_width = px(info_width);
        let card_background = super::theme::track_card(self.snapshot.theme, cx);
        let card_foreground =
            super::theme::selection_foreground(cx.theme().foreground, card_background);
        let card_secondary =
            super::theme::selection_foreground(cx.theme().muted_foreground, card_background);
        let toggle_player =
            |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                cx.stop_propagation();
                this.command(if this.sheet_open() {
                    PaletteCommandKind::GotoLibrary
                } else {
                    PaletteCommandKind::GotoNowPlaying
                });
            };
        let (elapsed, remaining) = crate::media::time_axis::transport_times(
            self.snapshot.position_ms,
            self.snapshot.duration_ms,
        );
        let info = div()
            .id("open-now-playing")
            .debug_selector(|| "transport-info".into())
            .relative()
            .w(info_width)
            .flex_none()
            .h(px(64.))
            .rounded_md()
            .when(
                self.snapshot.theme == crate::app::theme::ThemeName::BrushedAluminum,
                |display| display.rounded(px(9.)),
            )
            .bg(card_background)
            .text_color(card_foreground)
            .when(
                matches!(
                    self.snapshot.theme,
                    crate::app::theme::ThemeName::BlackAndWhite
                        | crate::app::theme::ThemeName::BrushedAluminum
                ),
                |card| card.border_1().border_color(cx.theme().border),
            )
            .flex()
            .items_center()
            .justify_center()
            .pb(px(18.))
            .px_2()
            .cursor_pointer()
            .role(Role::Button)
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if !this.sheet_open() {
                    this.command(PaletteCommandKind::GotoNowPlaying);
                }
            }))
            .focusable()
            .tab_stop(true)
            .focus(|style| style.border_1().border_color(cx.theme().selection))
            .aria_label("Show Now Playing")
            .child(
                div()
                    .debug_selector(|| "transport-track-content".into())
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .min_w_0()
                    .max_w_full()
                    .child(
                        div()
                            .id("mini-player-cover-button")
                            .focusable()
                            .tab_stop(true)
                            .focus(|style| style.border_1().border_color(cx.theme().selection))
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_label("Open Mini Player")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.enter_mini(window, cx)),
                            )
                            .debug_selector(|| "transport-cover-slot".into())
                            .w(px(32.))
                            .h(px(32.))
                            .flex_none()
                            .rounded_sm()
                            .overflow_hidden()
                            .map(|art| {
                                if let Some(image) = self.transport_art.clone() {
                                    art.child(img(image).size_full().object_fit(ObjectFit::Contain))
                                } else if self.snapshot.theme
                                    == crate::app::theme::ThemeName::BrushedAluminum
                                {
                                    art.flex().items_center().justify_center().child(
                                        Icon::default()
                                            .data(include_bytes!(
                                                "../../assets/icons/transport/lcd-notes.svg"
                                            ))
                                            .size(px(24.))
                                            .text_color(rgb(0x404832)),
                                    )
                                } else {
                                    art.bg(cx.theme().muted)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(Glyph::Music2)
                                                .size(px(14.4))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                }
                            }),
                    )
                    .child(
                        div()
                            .debug_selector(|| "transport-track-text".into())
                            .text_center()
                            .min_w_0()
                            .max_w(px(450.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(card_foreground)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_ellipsis()
                                    .child(self.snapshot.now_playing.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .min_w_0()
                                    .max_w_full()
                                    .when_some(self.snapshot.player.track.as_ref(), |row, track| {
                                        let metadata = [track.track_artist(), track.album_name()]
                                            .into_iter()
                                            .filter(|name| !name.is_empty())
                                            .collect::<Vec<_>>()
                                            .join(" · ");
                                        row.child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(card_secondary)
                                                .text_ellipsis()
                                                .child(metadata),
                                        )
                                    })
                                    .when(!self.snapshot.player.station.is_empty(), |text| {
                                        text.child(
                                            div()
                                                .pl_1()
                                                .text_size(px(10.))
                                                .text_color(card_secondary)
                                                .text_ellipsis()
                                                .child(self.snapshot.player.station.clone()),
                                        )
                                    }),
                            ),
                    )
                    .child(div().w(px(32.)).flex_none()),
            )
            .child(
                div()
                    .id("transport-seek-row")
                    .debug_selector(|| "transport-seek-row".into())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left_2()
                    .right_2()
                    .bottom(px(1.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(10.))
                    .text_color(card_secondary)
                    .track_focus(&self.seek_focus)
                    .key_context("TextampSlider")
                    .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.seek_focus.focus(window, cx);
                    }))
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let delta = match event.keystroke.key.as_str() {
                            "up" | "down" if event.keystroke.modifiers == Modifiers::default() => {
                                this.pane_focus
                                    .step(event.keystroke.key == "up", true, window, cx);
                                return;
                            }
                            "left" => Some(-5000),
                            "right" => Some(5000),
                            _ => None,
                        };
                        if let Some(delta_ms) = delta {
                            let _ = this.engine.commands.try_send(Command::SeekRelative {
                                playback_id: this.snapshot.playback_id,
                                delta_ms,
                            });
                            cx.stop_propagation();
                        }
                    }))
                    .child(div().flex_none().child(elapsed))
                    .child(
                        div().flex_1().min_w_0().child(
                            Slider::new(&self.seek_slider)
                                .accessibility_label("Playback position")
                                .disabled(!self.snapshot.seekable),
                        ),
                    )
                    .child(div().flex_none().child(remaining)),
            );
        let mut controls = div()
            .debug_selector(|| "transport-controls".into())
            .flex()
            .items_center()
            .gap(px(if dense_controls {
                0.
            } else if self.snapshot.theme == crate::app::theme::ThemeName::BrushedAluminum {
                2.
            } else {
                12.
            }))
            .flex_none();
        for (id, icon, label, action) in [
            (
                "previous",
                Glyph::SkipBack,
                "Previous Track",
                PlaybackAction::Previous,
            ),
            (
                "play-pause",
                if self.snapshot.playing {
                    Glyph::Pause
                } else {
                    Glyph::Play
                },
                if self.snapshot.playing {
                    "Pause · Space"
                } else {
                    "Play · Space"
                },
                PlaybackAction::TogglePlayPause,
            ),
            (
                "next",
                Glyph::SkipForward,
                "Next Track",
                PlaybackAction::Next,
            ),
        ] {
            let button = Button::new(id)
                .debug_selector(move || {
                    if id == "play-pause" {
                        "transport-play".into()
                    } else {
                        format!("transport-{id}")
                    }
                })
                .accessibility_label(label)
                .tooltip(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let _ = this
                        .engine
                        .commands
                        .try_send(Command::Action(action.clone().into()));
                }));
            controls = controls.child(super::theme::playback_button(
                button,
                icon,
                self.snapshot.theme,
            ));
        }
        let lyrics_surface = self
            .snapshot
            .surface
            .iter()
            .chain(self.snapshot.surfaces.iter())
            .find(|surface| surface.title.starts_with("Lyrics · "))
            .map(|surface| surface.id.clone());
        let more = Button::new("now-playing-more")
            .ghost()
            .small()
            .when(dense_controls, |button| button.w(px(24.)))
            .icon(Icon::new(Glyph::Ellipsis))
            .disabled(target.is_none())
            .accessibility_label("Track Actions")
            .tooltip("Track Actions")
            .dropdown_menu_with_anchor(gpui::Anchor::BottomRight, move |mut menu, _, _| {
                if let Some(target) = &target {
                    for entry in &context {
                        if matches!(
                            entry.kind,
                            crate::app::track_context::ContextKind::Separator
                        ) {
                            menu = menu.separator();
                        } else {
                            menu = menu.menu(
                                entry.label.clone(),
                                Box::new(TrackContextCommand {
                                    target: target.clone(),
                                    label: entry.label.clone(),
                                }),
                            );
                        }
                    }
                }
                menu
            });
        let bar = div()
            .id("transport-bar")
            .track_focus(self.pane_focus.handle(super::keyboard::Pane::Transport))
            .tab_group()
            .debug_selector(|| "transport-bar".into())
            .on_click(cx.listener(toggle_player))
            .relative()
            .h(px(super::motion::TRANSPORT_HEIGHT))
            .flex_none()
            .when(!docked, |bar| {
                bar.border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().title_bar)
                    .when(
                        self.snapshot.theme == crate::app::theme::ThemeName::BrushedAluminum,
                        |view| view.child(super::theme::aluminum_surface()),
                    )
            })
            .flex()
            .items_center()
            .gap_4()
            .when_else(docked, |bar| bar.px_6(), |bar| bar.px_4())
            .child(
                div()
                    .debug_selector(|| "transport-leading".into())
                    .w(wing)
                    .flex_auto()
                    .min_w(px(if dense_controls { 108. } else { 132. }))
                    .child(controls),
            )
            .child(info)
            .child(
                div()
                    .debug_selector(|| "transport-trailing".into())
                    .w(wing)
                    .flex_auto()
                    .min_w(wing)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("transport-more-hit")
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(more),
                    )
                    .child(
                        div()
                            .id("transport-lyrics-hit")
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new("transport-lyrics")
                                    .ghost()
                                    .small()
                                    .when(dense_controls, |button| button.w(px(24.)))
                                    .icon(Icon::new(Glyph::MessageSquareQuote))
                                    .selected(lyrics_surface.is_some())
                                    .when(lyrics_surface.is_some(), |button| button.primary())
                                    .disabled(self.snapshot.player.track.is_none())
                                    .accessibility_label("Lyrics")
                                    .tooltip("Lyrics")
                                    .on_click(cx.listener(move |this, _, _, _| {
                                        if lyrics_surface.is_none() && !this.sheet_open() {
                                            this.command(PaletteCommandKind::GotoNowPlaying);
                                        }
                                        let command = lyrics_surface.as_ref().map_or_else(
                                            || Command::Action(PlaybackAction::ShowLyrics.into()),
                                            |id| Command::CloseSurface(id.clone()),
                                        );
                                        let _ = this.engine.commands.try_send(command);
                                    })),
                            ),
                    )
                    .when(docked, |row| {
                        row.child(
                            Button::new("toggle-visualizer")
                                .debug_selector(|| "toggle-visualizer".into())
                                .ghost()
                                .small()
                                .icon(Icon::new(Glyph::AudioWaveform))
                                .selected(
                                    self.sheet_open()
                                        && self.show_visualizer
                                        && self.player_reader.is_none(),
                                )
                                .accessibility_label("Toggle Visualizer")
                                .tooltip(
                                    if self.sheet_open()
                                        && self.show_visualizer
                                        && self.player_reader.is_none()
                                    {
                                        "Hide Visualizer"
                                    } else {
                                        "Show Visualizer"
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(reader) =
                                        this.snapshot.surface.as_ref().filter(|r| r.player_reader)
                                    {
                                        let _ = this
                                            .engine
                                            .commands
                                            .try_send(Command::CloseSurface(reader.id.clone()));
                                        this.show_visualizer = true;
                                    } else if this.sheet_open() {
                                        this.show_visualizer = !this.show_visualizer;
                                    } else {
                                        this.show_visualizer = true;
                                        this.command(PaletteCommandKind::GotoNowPlaying);
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .id("transport-volume")
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .debug_selector(|| "transport-volume".into())
                            .w(px(if dense_controls {
                                66.
                            } else if compact_controls {
                                84.
                            } else {
                                112.
                            }))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .track_focus(&self.volume_focus)
                            .key_context("TextampSlider")
                            .capture_any_mouse_down(cx.listener(
                                |this, _: &MouseDownEvent, window, cx| {
                                    this.volume_focus.focus(window, cx);
                                },
                            ))
                            .capture_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    let action = match event.keystroke.key.as_str() {
                                        "up" | "down"
                                            if event.keystroke.modifiers
                                                == Modifiers::default() =>
                                        {
                                            this.pane_focus.step(
                                                event.keystroke.key == "up",
                                                true,
                                                window,
                                                cx,
                                            );
                                            return;
                                        }
                                        "left" if this.volume_focus.is_focused(window) => {
                                            Some(PlaybackAction::VolumeDown)
                                        }
                                        "right" if this.volume_focus.is_focused(window) => {
                                            Some(PlaybackAction::VolumeUp)
                                        }
                                        _ => None,
                                    };
                                    if let Some(action) = action {
                                        let _ = this
                                            .engine
                                            .commands
                                            .try_send(Command::Action(action.into()));
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(
                                Button::new("transport-mute")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(if self.snapshot.muted {
                                        Glyph::VolumeX
                                    } else {
                                        Glyph::Volume2
                                    }))
                                    .selected(self.snapshot.muted)
                                    .when(self.snapshot.muted, |button| button.primary())
                                    .accessibility_label(if self.snapshot.muted {
                                        "Unmute"
                                    } else {
                                        "Mute"
                                    })
                                    .tooltip(if self.snapshot.muted {
                                        "Unmute"
                                    } else {
                                        "Mute"
                                    })
                                    .on_click(cx.listener(|this, _, _, _| {
                                        let _ = this.engine.commands.try_send(Command::Action(
                                            PlaybackAction::ToggleMute.into(),
                                        ));
                                    })),
                            )
                            .child(div().flex_1().min_w_0().child(
                                Slider::new(&self.volume_slider).accessibility_label(
                                    if self.snapshot.muted {
                                        "Volume — Muted"
                                    } else {
                                        "Volume"
                                    },
                                ),
                            )),
                    ),
            );
        div()
            .relative()
            .h(px(super::motion::TRANSPORT_HEIGHT))
            .flex_none()
            .child(bar)
            .when(docked && !self.sheet_open(), |v| {
                v.child(
                    div()
                        .absolute()
                        .right(
                            (self.library_size.width
                                - self.sheet_size.width
                                - px(
                                    if self.snapshot.theme
                                        == crate::app::theme::ThemeName::BrushedAluminum
                                    {
                                        super::theme::ALUMINUM_RIM
                                    } else {
                                        0.
                                    },
                                ))
                            .max(px(0.))
                                + px(4.),
                        )
                        .top_1()
                        .child(
                            Button::new("expand-player-sheet")
                                .debug_selector(|| "expand-player-sheet".into())
                                .ghost()
                                .small()
                                .icon(Icon::new(Glyph::ChevronUp))
                                .accessibility_label("Show Now Playing")
                                .tooltip("Show Now Playing")
                                .on_click(cx.listener(toggle_player)),
                        ),
                )
            })
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.presentation.active() == Some(crate::app::launch::WindowedMode::Text) {
            let _ = self.engine.commands.try_send(Command::OpenPalette {
                query: String::new(),
                player: false,
            });
            return;
        }

        if !self.snapshot.separate_player
            && window.has_active_dialog(cx)
            && self
                .snapshot
                .surface
                .as_ref()
                .is_some_and(|surface| surface.palette)
        {
            if let Some(surface) = self.snapshot.surface.as_ref() {
                let _ = self
                    .engine
                    .commands
                    .try_send(Command::CloseSurface(surface.id.clone()));
            }
            return;
        }
        if self.snapshot.separate_player
            && let Some(palette) = self.snapshot.surfaces.iter().find(|model| model.palette)
        {
            let _ = self
                .engine
                .commands
                .try_send(Command::CloseSurface(palette.id.clone()));
            return;
        }
        cx.set_global(super::menu_catalog::Origin {
            window: window.window_handle(),
            focus: window.focused(cx),
            actions: window
                .available_actions(cx)
                .iter()
                .map(|action| action.name().into())
                .collect(),
        });
        let _ = self.engine.commands.try_send(Command::OpenPalette {
            query: String::new(),
            player: self.sheet_open(),
        });
    }
}
