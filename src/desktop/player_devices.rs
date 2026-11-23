//! Native Mac controls over the shared player-sync job.
use super::*;
use crate::app::player_sync::{self, Command as PlayerCommand, Phase};
use gpui_kit::component::{
    Disableable, Icon, Selectable, Sizable,
    button::ButtonVariants,
    checkbox::Checkbox,
    form::{Field, Form},
    input::{Input, InputEvent, InputState},
    label::Label,
    progress::Progress,
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
};
gpui_kit::actions!(player_devices, [Players]);
pub(super) fn sidebar_icon() -> Icon {
    Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <rect x="5.5" y="2" width="13" height="20" rx="2.5"/>
      <rect x="8" y="4.75" width="8" height="5.5" rx=".75"/>
      <circle cx="12" cy="16" r="3.25"/>
      <circle cx="12" cy="16" r=".8" fill="currentColor" stroke="none"/>
    </svg>"#)
}

pub(super) struct PlayerDevices {
    engine: std::sync::Arc<Engine>,
    model: player_sync::View,
    report_filter: Entity<InputState>,
    _report_filter_subscription: Subscription,
    name: Entity<InputState>,
    playlist: Entity<InputState>,
    playlist_filter: Entity<InputState>,
    folder_filter: Entity<InputState>,
    _folder_filter_subscription: Subscription,
    _filter_subscription: Subscription,
    focus: FocusHandle,
    _updates: Task<()>,
}
use crate::util::format_bytes as bytes;
impl PlayerDevices {
    pub(super) fn new(
        engine: std::sync::Arc<Engine>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = engine.snapshot.borrow().player_sync.clone();
        let report_filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter files")
                .default_value(model.report_query.clone())
        });
        let report_subscription =
            cx.subscribe(&report_filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.send(PlayerCommand::ReportFilter(
                        input.read(cx).value().to_string(),
                    ));
                }
            });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Player name"));
        let playlist = cx.new(|cx| InputState::new(window, cx).placeholder("Sync: <player name>"));
        let playlist_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name"));
        let filter_subscription = cx.subscribe(&playlist_filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let query = model
            .webdav
            .as_ref()
            .map(|webdav| webdav.query.as_str())
            .unwrap_or_default();
        let folder_filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter folders and subfolders")
                .default_value(query.to_owned())
        });

        let folder_filter_subscription = cx.subscribe(
            &folder_filter,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => this.send(PlayerCommand::FilterWebdav(
                    input.read(cx).value().to_string(),
                )),
                InputEvent::PressEnter { .. } => {
                    if let Some(webdav) = &this.model.webdav
                        && let [(path, _)] = webdav.folders.as_slice()
                    {
                        this.send(PlayerCommand::BrowseWebdav(path.clone()));
                    }
                }
                _ => {}
            },
        );
        let focus = cx.focus_handle();
        let mut updates = engine.snapshot.clone();
        let task = cx.spawn_in(window, async move |entity, cx| {
            while updates.changed().await.is_ok() {
                let model = updates.borrow_and_update().player_sync.clone();
                if cx
                    .update(|window, cx| {
                        entity.update(cx, |this, cx| {
                            let changed = this.model.selected != model.selected
                                || this.model.players != model.players;
                            let location = |model: &player_sync::View| {
                                model
                                    .webdav
                                    .as_ref()
                                    .map(|webdav| (webdav.source_id.clone(), webdav.path.clone()))
                            };
                            if location(&this.model) != location(&model) {
                                let query = model
                                    .webdav
                                    .as_ref()
                                    .map(|webdav| webdav.query.clone())
                                    .unwrap_or_default();
                                this.folder_filter
                                    .update(cx, |input, cx| input.set_value(query, window, cx));
                            }
                            if model.report_query.is_empty()
                                && !this.model.report_query.is_empty()
                                && !this.report_filter.read(cx).value().is_empty()
                            {
                                this.report_filter
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            this.model = model;
                            if changed {
                                this.load_name(window, cx);
                            }
                            cx.notify();
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            report_filter,
            _report_filter_subscription: report_subscription,
            engine,
            model,
            name,
            playlist,
            playlist_filter,
            folder_filter,
            _folder_filter_subscription: folder_filter_subscription,
            _filter_subscription: filter_subscription,
            focus,
            _updates: task,
        };
        view.load_name(window, cx);
        view
    }
    fn load_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .model
            .players
            .iter()
            .find(|player| Some(&player.volume_uuid) == self.model.selected.as_ref())
            .map(|player| player.name.as_str())
            .unwrap_or("Audio Player");
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        let playlist = self
            .model
            .players
            .iter()
            .find(|player| Some(&player.volume_uuid) == self.model.selected.as_ref())
            .map(|player| player.playlists.first().map(String::as_str).unwrap_or(""))
            .unwrap_or("");
        self.playlist
            .update(cx, |input, cx| input.set_value(playlist, window, cx));
    }
    fn report(
        &self,
        rows: &std::sync::Arc<crate::services::player_sync::Report>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::{accordion::Accordion, list::ListItem};
        let (title, action) = match &self.model.phase {
            Phase::Review {
                operation, title, ..
            } => (format!("Review Sync · {title}"), Some(*operation)),
            _ => ("On this player".into(), None),
        };
        let header = div()
            .flex()
            .gap_2()
            .items_center()
            .justify_between()
            .child(Label::new(title))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("close-player-report")
                            .label(if action.is_some() { "Cancel" } else { "Done" })
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.send(if action.is_some() {
                                    PlayerCommand::Cancel
                                } else {
                                    PlayerCommand::CloseInventory
                                })
                            })),
                    )
                    .when_some(action, |buttons, operation| {
                        buttons.child(
                            Button::new("apply-player-review")
                                .debug_selector(|| "apply-player-review".into())
                                .primary()
                                .label("Apply Sync")
                                .on_click(cx.listener(move |this, _, _, _| {
                                    this.send(PlayerCommand::ApplyReview(operation))
                                })),
                        )
                    }),
            );
        let mut groups = div()
            .id("player-report-groups")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar();
        let query = self.model.report_query.trim().to_lowercase();
        let mut found = false;
        for (index, group) in player_sync::report_groups(&self.model.phase)
            .into_iter()
            .enumerate()
        {
            let entries = &rows[group];
            let filtered: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    (query.is_empty()
                        || s.to_lowercase().contains(&query)
                        || group.to_lowercase().contains(&query))
                    .then_some(i)
                })
                .collect();
            if filtered.is_empty() && !query.is_empty() {
                continue;
            }
            found = true;
            let expanded = self.model.report_group.as_deref() == Some(group) || !query.is_empty();
            let title = format!("{group} ({})", filtered.len());
            let key = group.to_owned();
            let accordion = Accordion::new(("player-report-section", index))
                .bordered(false)
                .h_auto()
                .small()
                .on_toggle_click(cx.listener(move |this, indices: &[usize], _, _| {
                    if this.model.report_query.is_empty() && indices.contains(&0) != expanded {
                        this.send(PlayerCommand::ReportGroup(key.clone()));
                    }
                }));
            let list = if expanded && !filtered.is_empty() {
                let scroll = window
                    .use_keyed_state(("player-report-scroll", index), cx, |_, _| {
                        UniformListScrollHandle::new()
                    })
                    .read(cx)
                    .clone();
                let report = rows.clone();
                let key = group.to_owned();
                let count = filtered.len();
                let list =
                    uniform_list(("player-report-list", index), count, move |range, _, _| {
                        range
                            .map(|i| {
                                ListItem::new(("player-report-row", i))
                                    .child(Label::new(report[&key][filtered[i]].clone()))
                            })
                            .collect::<Vec<_>>()
                    })
                    .size_full()
                    .track_scroll(&scroll);
                Some(
                    div()
                        .relative()
                        .h(px((count.min(10) * 32) as f32))
                        .child(list)
                        .vertical_scrollbar(&scroll),
                )
            } else {
                None
            };
            groups = groups.child(accordion.item(move |item| {
                item.title(
                    div()
                        .debug_selector(move || format!("player-report-header-{index}"))
                        .child(Label::new(title)),
                )
                .open(expanded)
                .when_some(list, |item, list| item.child(list))
            }));
        }
        if !found {
            groups = groups.child(Label::new(if query.is_empty() {
                "No synced files on this player."
            } else {
                "No matching files."
            }));
        }
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .track_focus(&self.focus)
            .on_action(super::keyboard::next_focus)
            .on_action(super::keyboard::previous_focus)
            .child(header)
            .child(
                Input::new(&self.report_filter)
                    .small()
                    .cleanable(true)
                    .aria_label("Filter files"),
            )
            .child(groups)
            .into_any_element()
    }
    fn webdav_browser(
        &self,
        webdav: &player_sync::WebdavView,
        volume_uuid: &str,
        busy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tabs = TabBar::new("webdav-folder-tabs")
            .segmented()
            .small()
            .menu(false)
            .children(
                [(false, "Browse"), (true, "Synced folders")]
                    .into_iter()
                    .map(|(synced, title)| {
                        Tab::new()
                            .label(title)
                            .selected(webdav.synced_tab == synced)
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.send(PlayerCommand::ShowSyncedFolders(synced))
                            }))
                    }),
            );
        if webdav.synced_tab {
            let scroll = window
                .use_keyed_state("webdav-synced-scroll", cx, |_, _| ScrollHandle::new())
                .read(cx)
                .clone();
            let mut list = div()
                .id("webdav-synced-folders")
                .flex()
                .flex_col()
                .gap_2()
                .max_h(px(250.))
                .pr_2();
            for (path, checked) in &webdav.folders {
                let choose = PlayerCommand::ChooseWebdavFolder {
                    volume_uuid: volume_uuid.to_owned(),
                    source_id: webdav.source_id.clone(),
                    path: path.clone(),
                };
                let label = if path.is_empty() {
                    "Entire library".to_owned()
                } else {
                    path.clone()
                };
                list = list.child(
                    Checkbox::new(SharedString::from(format!("synced:{path}")))
                        .small()
                        .label(label)
                        .checked(*checked)
                        .disabled(busy || webdav.inherited.contains(path))
                        .on_change(cx.listener(move |this, _, _, _| this.send(choose.clone()))),
                );
            }
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .child(tabs)
                .when(webdav.folders.is_empty(), |pane| {
                    pane.child(Label::new("No synced WebDAV folders"))
                })
                .child(
                    list.overflow_y_scroll()
                        .track_scroll(&scroll)
                        .vertical_scrollbar(&scroll),
                )
                .into_any_element();
        }
        let current_title = if webdav.path.is_empty() {
            "Entire library"
        } else {
            "This folder"
        };
        let choose_current = PlayerCommand::ChooseWebdavFolder {
            volume_uuid: volume_uuid.into(),
            source_id: webdav.source_id.clone(),
            path: webdav.path.clone(),
        };
        let current_focus = window
            .use_keyed_state("webdav-current-focus", cx, |_, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone();
        let current_focused = current_focus.is_focused(window);
        let current_click = current_focus.clone();
        let current_select = choose_current.clone();
        let can_go_up = !webdav.path.is_empty();
        let marker = (volume_uuid.to_owned(), webdav.source_id.clone());
        let prefix = if webdav.path.is_empty() {
            String::new()
        } else {
            format!("{}/", webdav.path)
        };
        let scroll = window
            .use_keyed_state(
                SharedString::from(format!("webdav-scroll:{}", webdav.path)),
                cx,
                |_, _| ScrollHandle::new(),
            )
            .read(cx)
            .clone();
        let row_focuses: Vec<_> = webdav
            .folders
            .iter()
            .map(|(path, _)| {
                window
                    .use_keyed_state(
                        SharedString::from(format!("webdav-focus:{path}")),
                        cx,
                        |_, cx| cx.focus_handle().tab_stop(true),
                    )
                    .read(cx)
                    .clone()
            })
            .collect();
        let first_focus = row_focuses.first().cloned();
        let first_reveal = scroll.clone();
        let mut list = div()
            .id("webdav-sync-folders")
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(250.))
            .pr_2();
        for (index, (path, checked)) in webdav.folders.iter().enumerate() {
            let name = path.strip_prefix(&prefix).unwrap_or(path).to_owned();
            let open = PlayerCommand::BrowseWebdav(path.clone());
            let enter = open.clone();
            let opened_focus = current_focus.clone();
            let click_open_focus = current_focus.clone();
            let choose = PlayerCommand::ChooseWebdavFolder {
                volume_uuid: volume_uuid.into(),
                source_id: webdav.source_id.clone(),
                path: path.clone(),
            };
            let id = SharedString::from(path.clone());
            let debug_id = format!("webdav-row:{path}");
            let check_id = format!("webdav-check:{path}");
            let row_focus = &row_focuses[index];
            let previous = index.checked_sub(1).map(|index| row_focuses[index].clone());
            let next = row_focuses.get(index + 1).cloned();
            let top = current_focus.clone();
            let reveal = scroll.clone();
            let focused = row_focus.is_focused(window);
            let select = choose.clone();
            let click_focus = row_focus.clone();
            list = list.child(
                gpui_kit::component::h_flex()
                    .id(id)
                    .debug_selector(|| debug_id)
                    .track_focus(row_focus)
                    .flex_shrink_0()
                    .when(focused, |row| {
                        row.bg(cx.theme().primary.opacity(0.18)).rounded_sm()
                    })
                    .w_full()
                    .gap_2()
                    .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "enter" | "right" => {
                                this.send(enter.clone());
                                opened_focus.focus(window, cx);
                            }
                            "space" if !busy => this.send(select.clone()),
                            "left" | "backspace" if can_go_up => {
                                this.send(PlayerCommand::UpWebdav);
                                top.focus(window, cx);
                            }
                            "up" => {
                                if let Some(previous) = &previous {
                                    previous.focus(window, cx);
                                    reveal.scroll_to_item(index - 1);
                                } else {
                                    top.focus(window, cx);
                                }
                            }
                            "down" => {
                                if let Some(next) = &next {
                                    next.focus(window, cx);
                                    reveal.scroll_to_item(index + 1);
                                }
                            }
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
                    .child(
                        div().flex_shrink_0().debug_selector(|| check_id).child(
                            Checkbox::new("webdav-sync-check")
                                .small()
                                .accessibility_label(if webdav.mixed.contains(path) {
                                    format!("Sync {name}, partially selected")
                                } else {
                                    format!("Sync {name}")
                                })
                                .mixed(webdav.mixed.contains(path))
                                .checked(*checked)
                                .tab_stop(false)
                                .disabled(busy || webdav.inherited.contains(path))
                                .on_change(cx.listener(move |this, _, window, cx| {
                                    click_focus.focus(window, cx);
                                    this.send(choose.clone());
                                })),
                        ),
                    )
                    .child(
                        Button::new("webdav-sync-open")
                            .flex_1()
                            .min_w_0()
                            .justify_start()
                            .small()
                            .ghost()
                            .tab_stop(false)
                            .child(
                                gpui_kit::component::h_flex()
                                    .w_full()
                                    .gap_2()
                                    .justify_between()
                                    .child(Label::new(name.clone()))
                                    .child(
                                        Icon::new(gpui_kit::component::IconName::ChevronRight)
                                            .small(),
                                    ),
                            )
                            .accessibility_label(format!("Open {name}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send(open.clone());
                                click_open_focus.focus(window, cx);
                            })),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(tabs)
            .child(
                gpui_kit::component::h_flex()
                    .justify_between()
                    .child(
                        Label::new(format!("Folders · {}", webdav.name))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(format!("{} selected", webdav.selected_count))
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                gpui_kit::component::h_flex()
                    .gap_2()
                    .child(
                        Button::new("webdav-sync-up")
                            .small()
                            .ghost()
                            .icon(gpui_kit::component::IconName::ChevronLeft)
                            .label("Back")
                            .disabled(webdav.path.is_empty())
                            .on_click(
                                cx.listener(move |this, _, _, _| {
                                    this.send(PlayerCommand::UpWebdav)
                                }),
                            ),
                    )
                    .child(Label::new(if webdav.path.is_empty() {
                        webdav.name.clone()
                    } else {
                        webdav.path.clone()
                    })),
            )
            .child(
                div()
                    .capture_key_down({
                        let focus = current_focus.clone();
                        cx.listener(move |_, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "down" {
                                focus.focus(window, cx);
                                cx.stop_propagation();
                            }
                        })
                    })
                    .child(Input::new(&self.folder_filter).small()),
            )
            .child(
                div()
                    .id("webdav-current-folder")
                    .track_focus(&current_focus)
                    .when(current_focused, |row| {
                        row.bg(cx.theme().primary.opacity(0.18)).rounded_sm()
                    })
                    .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "space" if !busy => this.send(current_select.clone()),
                            "left" | "backspace" if can_go_up => this.send(PlayerCommand::UpWebdav),
                            "up" => window.focus_prev(cx),
                            "down" => {
                                if let Some(first) = &first_focus {
                                    first.focus(window, cx);
                                    first_reveal.scroll_to_item(0);
                                }
                            }
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new("webdav-sync-current")
                            .small()
                            .accessibility_label(current_title)
                            .mixed(webdav.mixed.contains(&webdav.path))
                            .checked(webdav.current_checked)
                            .tab_stop(false)
                            .disabled(busy || webdav.inherited.contains(&webdav.path))
                            .on_change(cx.listener(move |this, _, window, cx| {
                                current_click.focus(window, cx);
                                this.send(choose_current.clone());
                            })),
                    )
                    .child(Label::new(current_title)),
            )
            .child(
                list.overflow_y_scroll()
                    .track_scroll(&scroll)
                    .vertical_scrollbar(&scroll),
            )
            .when(webdav.loading, |pane| {
                pane.child(Label::new("Loading folders…"))
            })
            .when(
                !webdav.loading && webdav.folders.is_empty() && webdav.error.is_none(),
                |pane| {
                    pane.child(Label::new(if webdav.query.is_empty() {
                        "No subfolders"
                    } else {
                        "No matching folders"
                    }))
                },
            )
            .when_some(webdav.error.clone(), |pane, error| {
                pane.child(Label::new(error))
            })
            .child(
                Checkbox::new("webdav-nomedia")
                    .label("Hide from media scanner (.nomedia)")
                    .checked(webdav.nomedia)
                    .disabled(busy)
                    .on_change(cx.listener(move |this, enabled, _, _| {
                        this.send(PlayerCommand::SetWebdavNomedia {
                            volume_uuid: marker.0.clone(),
                            source_id: marker.1.clone(),
                            enabled: *enabled,
                        })
                    })),
            )
            .into_any_element()
    }
    fn send(&self, command: PlayerCommand) {
        let _ = self
            .engine
            .commands
            .try_send(Command::Action(command.into()));
    }
    pub(super) fn open_palette(&self, window: &mut Window, cx: &mut Context<Self>) {
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
            player: false,
        });
    }
}
impl Render for PlayerDevices {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Phase::Inventory { rows, .. } | Phase::Review { rows, .. } = &self.model.phase {
            return self.report(rows, window, cx);
        }
        let player = self.model.configured_player.as_ref();
        let webdav = self.model.webdav.as_ref();
        let ready = player.is_some() || webdav.is_some();
        let selected = self
            .model
            .volumes
            .iter()
            .any(|volume| Some(&volume.uuid) == self.model.selected.as_ref());
        let busy = self.model.busy;
        let mut content = div().flex().flex_col().gap_4().p_5().w_full().child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(Label::new(player_sync::DESCRIPTION))
                .when(selected, |header| {
                    header.child(
                        Button::new("inventory-player")
                            .small()
                            .label("On this player…")
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, _, _| this.send(PlayerCommand::Inventory)),
                            ),
                    )
                })
                .child(
                    Button::new("refresh-players")
                        .label("Refresh")
                        .small()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, _| {
                            this.send(PlayerCommand::Discover { quiet: false })
                        })),
                ),
        );
        if self.model.volumes.is_empty() {
            content = content.child(
                Label::new("Connect your player and enable USB storage.")
                    .text_color(cx.theme().muted_foreground),
            );
        }
        if self.model.volumes.len() > 1 && !selected {
            content = content.child(Label::new("Choose a storage volume before syncing."));
        }
        let ids = self
            .model
            .volumes
            .iter()
            .map(|volume| volume.uuid.clone())
            .collect::<Vec<_>>();
        content = content.child(
            RadioGroup::new("player-storage")
                .layout(Axis::Vertical)
                .disabled(busy)
                .selected_index(
                    self.model
                        .volumes
                        .iter()
                        .position(|volume| Some(&volume.uuid) == self.model.selected.as_ref()),
                )
                .children(self.model.volumes.iter().map(|volume| {
                    Radio::new(volume.uuid.clone()).label(format!(
                        "{} — {} free of {}",
                        volume.name,
                        bytes(volume.available),
                        bytes(volume.capacity)
                    ))
                }))
                .on_change(cx.listener(move |this, index: &usize, _, _| {
                    if let Some(uuid) = ids.get(*index) {
                        this.send(PlayerCommand::Select(uuid.clone()));
                    }
                })),
        );
        let current = player
            .map(|player| player.playlists.as_slice())
            .unwrap_or_default();
        let volume = self
            .model
            .selected
            .clone()
            .filter(|_| selected)
            .or_else(|| {
                webdav
                    .is_none()
                    .then(|| player.map(|player| player.volume_uuid.clone()))
                    .flatten()
            });
        if let (Some(webdav), Some(volume_uuid)) = (webdav, volume.as_deref()) {
            content = content.child(self.webdav_browser(webdav, volume_uuid, busy, window, cx));
        } else if let Some(volume_uuid) = volume {
            let choices: Vec<_> = self
                .model
                .active_source
                .iter()
                .flat_map(|source| {
                    self.model.playlists.iter().map(|title| {
                        (
                            title.clone(),
                            current.iter().any(|name| name.eq_ignore_ascii_case(title)),
                            PlayerCommand::ChoosePlaylist {
                                volume_uuid: volume_uuid.clone(),
                                source_id: source.clone(),
                                title: title.clone(),
                            },
                        )
                    })
                })
                .collect();
            let selected_count = choices.iter().filter(|(_, checked, _)| *checked).count();
            let searchable = choices.len() > 8;
            let query = if searchable {
                self.playlist_filter.read(cx).value().trim().to_lowercase()
            } else {
                String::new()
            };
            let mut list = div()
                .id("player-playlists")
                .flex()
                .flex_col()
                .gap_1()
                .max_h(px(210.))
                .pr_2();
            let mut matches = 0;
            for (title, checked, command) in choices
                .into_iter()
                .filter(|(title, _, _)| title.to_lowercase().contains(&query))
            {
                matches += 1;
                list = list.child(
                    Checkbox::new(SharedString::from(title.clone()))
                        .w_full()
                        .py_1()
                        .small()
                        .label(title.clone())
                        .checked(checked)
                        .disabled(busy)
                        .on_change(cx.listener(move |this, _, _, _| this.send(command.clone()))),
                );
            }
            let mut section = div().flex().flex_col().gap_2().child(
                gpui_kit::component::h_flex()
                    .justify_between()
                    .child(Label::new("Playlists").font_weight(FontWeight::SEMIBOLD))
                    .child(
                        Label::new(format!("{selected_count} selected"))
                            .text_color(cx.theme().muted_foreground),
                    ),
            );
            if searchable {
                section = section.child(Input::new(&self.playlist_filter).small());
            }
            if matches > 0 {
                section = section.child(list.overflow_y_scrollbar());
            } else {
                let message = if !query.is_empty() {
                    "No matches"
                } else if player.is_some() {
                    "Open the linked library to change playlists."
                } else {
                    "Open a Navidrome library with playlists."
                };
                section =
                    section.child(Label::new(message).text_color(cx.theme().muted_foreground));
            }
            if let Some(size) = &self.model.size_label {
                section =
                    section.child(Label::new(size.clone()).text_color(cx.theme().muted_foreground));
            }
            content = content.child(section);
        }
        if selected {
            if !ready {
                content = content.child(
                    Form::vertical()
                        .child(
                            Field::new()
                                .label("Player name")
                                .child(Input::new(&self.name).disabled(busy)),
                        )
                        .child(
                            Field::new()
                                .label("Server playlist")
                                .child(Input::new(&self.playlist).disabled(busy)),
                        ),
                );
            }
            if webdav.is_none()
                && let Some(player) = player
            {
                content = content.child(
                    Checkbox::new("auto-sync-player")
                        .label("Sync when connected")
                        .checked(player.auto_sync)
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, _, _| this.send(PlayerCommand::ToggleAuto)),
                        ),
                );
            } else if !ready {
                content = content.child(
                    Label::new(if self.model.can_link {
                        format!(
                            "Uses the named playlist in {}. Leave the playlist blank for Sync: <player name>. Creates it if needed.",
                            self.model.library
                        )
                    } else {
                        "Choose a Navidrome library to set up this player.".into()
                    })
                    .text_color(cx.theme().muted_foreground),
                );
            }
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .when(!ready, |view| {
                        view.child(
                            Button::new("link-player")
                                .label("Set Up Player")
                                .disabled(busy || !self.model.can_link)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let (Some(volume_uuid), Some(source_id)) = (
                                        this.model.selected.clone(),
                                        this.model.active_source.clone(),
                                    ) {
                                        this.send(PlayerCommand::Link {
                                            name: this.name.read(cx).value().to_string(),
                                            playlist: this.playlist.read(cx).value().to_string(),
                                            volume_uuid,
                                            source_id,
                                        });
                                    }
                                })),
                        )
                    })
                    .when(ready, |view| {
                        view.child(
                            Button::new("sync-player")
                                .debug_selector(|| "sync-player".into())
                                .label("Review Sync…")
                                .primary()
                                .disabled(self.model.sync_disabled.is_some())
                                .on_click(cx.listener(|this, _, _, _| {
                                    this.send(PlayerCommand::Start {
                                        create_playlist: false,
                                    })
                                })),
                        )
                    })
                    .child(
                        Button::new("eject-player")
                            .debug_selector(|| "eject-player".into())
                            .label("Eject")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, _| this.send(PlayerCommand::Eject))),
                    ),
            );
        }
        content = content.child(match &self.model.phase {
            Phase::Discovering { .. } => div().child(Label::new("Checking connected players…")),
            Phase::Review { .. } | Phase::Inventory { .. } => unreachable!("report shown above"),
            Phase::LoadingInventory => div().child(Label::new("Reading player inventory…")),
            Phase::Preparing => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(Label::new("Preparing sync…"))
                .child(
                    Progress::new("player-progress")
                        .loading(true)
                        .accessibility_label("Preparing player sync"),
                ),
            Phase::Ejecting => div().child(Label::new("Ejecting player…")),
            Phase::Copying(progress) => {
                let current = progress
                    .expected
                    .filter(|total| *total > 0)
                    .map(|total| progress.received as f32 / total as f32)
                    .unwrap_or(0.)
                    .clamp(0., 1.);
                let percent = if progress.total == 0 {
                    100.
                } else {
                    (progress.completed as f32 + current).min(progress.total as f32)
                        / progress.total as f32
                        * 100.
                };
                div()
                    .debug_selector(|| "player-sync-progress".into())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Label::new(progress.title.clone()))
                    .child(
                        Progress::new("player-progress")
                            .value(percent)
                            .accessibility_label("Player sync progress"),
                    )
                    .child(Label::new(format!(
                        "{} of {} files · {} copied · {} already on player",
                        progress.completed, progress.total, progress.copied, progress.reused
                    )))
                    .child(
                        Label::new(
                            progress
                                .expected
                                .map(|total| {
                                    format!("{} of {}", bytes(progress.received), bytes(total))
                                })
                                .unwrap_or_else(|| bytes(progress.received)),
                        )
                        .text_color(cx.theme().muted_foreground),
                    )
            }
            Phase::Cancelling => div().child(Label::new("Stopping the transfer…")),
            Phase::Complete(summary) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(Label::new(format!(
                    "Sync complete · {} copied · {} unchanged · {} removed",
                    summary.copied, summary.reused, summary.removed
                )))
                .child(
                    Label::new("Eject the player before disconnecting it.")
                        .text_color(cx.theme().muted_foreground),
                ),
            Phase::Cancelled => div().child(Label::new(
                "Sync stopped. Completed copies will be reused next time.",
            )),
            Phase::Failed(error) | Phase::DiscoveryFailed(error) => div().child(
                gpui_kit::component::alert::Alert::error("player-sync-error", error.clone()),
            ),
            Phase::Idle => div(),
        });
        if self.model.can_cancel {
            content = content.child(
                Button::new("cancel-player-sync")
                    .label("Cancel Sync")
                    .on_click(cx.listener(|this, _, _, _| this.send(PlayerCommand::Cancel))),
            );
        }
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .key_context("TextampPlayerDevices")
            .on_action(super::keyboard::next_focus)
            .on_action(super::keyboard::previous_focus)
            .on_action(
                cx.listener(|this, _: &super::Palette, window, cx| this.open_palette(window, cx)),
            )
            .child(
                div()
                    .id("player-devices-scroll")
                    .debug_selector(|| "player-devices-scroll".into())
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(content),
            )
            .into_any_element()
    }
}
#[cfg(all(test, feature = "desktop-tests"))]
mod tests {
    use super::{Phase, PlayerCommand, PlayerDevices};
    use crate::desktop::{Command, Engine, Snapshot};
    use gpui_kit::gpui::{self, AppContext, TestAppContext, px};
    #[gpui::test]
    fn webdav_row_click_and_space_select_while_enter_opens(cx: &mut TestAppContext) {
        use crate::app::player_sync::WebdavView;
        cx.update(super::super::init);
        let (engine, _, mut commands) = Engine::fixture(Snapshot {
            player_sync: crate::app::player_sync::View {
                selected: Some("player".into()),
                volumes: vec![crate::services::player_sync::Volume {
                    uuid: "player".into(),
                    name: "Player".into(),
                    mount: "/Volumes/Test".into(),
                    identifier: "fixture".into(),
                    available: 1024,
                    capacity: 1024,
                }],
                webdav: Some(WebdavView {
                    source_id: "archive".into(),
                    name: "Archive".into(),
                    folders: vec![
                        ("Concerts/Paris".into(), false),
                        ("Concerts/Rome".into(), false),
                    ],
                    nomedia: true,
                    path: "Concerts".into(),
                    query: String::new(),
                    current_checked: false,
                    mixed: Default::default(),
                    inherited: Default::default(),
                    synced_tab: false,
                    selected_count: 0,
                    loading: false,
                    error: None,
                }),
                ..Default::default()
            },
            ..Default::default()
        });
        let engine = std::sync::Arc::new(engine);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| PlayerDevices::new(engine, window, cx));
            gpui_kit::base::Root::new(view, window, cx)
        });
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        while commands.try_recv().is_ok() {}
        let row = cx
            .debug_bounds("webdav-row:Concerts/Paris")
            .expect("folder row");
        let checkbox = cx
            .debug_bounds("webdav-check:Concerts/Paris")
            .expect("checkbox");
        cx.simulate_click(checkbox.center(), Default::default());
        cx.simulate_keystrokes("space enter");
        cx.simulate_click(row.center(), Default::default());
        cx.simulate_keystrokes("down down space up enter left");
        let actions: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| {
                if let Command::Action(crate::app::Action::PlayerSync(action)) = command {
                    Some(action)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            matches!(&actions[..], [
            PlayerCommand::ChooseWebdavFolder { path: click, .. },
            PlayerCommand::ChooseWebdavFolder { path: space, .. },
            PlayerCommand::BrowseWebdav(enter),
            PlayerCommand::BrowseWebdav(row_click),
            PlayerCommand::ChooseWebdavFolder { path: arrow_space, .. },
            PlayerCommand::BrowseWebdav(arrow_enter),
            PlayerCommand::UpWebdav,
        ] if click == "Concerts/Paris" && click == space && click == enter && click == row_click
            && arrow_space == "Concerts/Rome" && arrow_enter == "Concerts/Paris"),
            "{actions:?}"
        );
    }

    #[gpui::test]
    fn sync_controls_dispatch_and_disable_ejection_during_transfer(cx: &mut TestAppContext) {
        cx.update(super::super::init);
        let mut state = crate::app::AppState::new();
        state.desktop_frontend = true;
        let volume = crate::services::player_sync::Volume {
            uuid: "player".into(),
            name: "Y2 storage".into(),
            mount: "/Volumes/Y2".into(),
            identifier: "fixture".into(),
            available: 1024 * 1024 * 1024,
            capacity: 1024 * 1024 * 2048,
        };
        state.player_sync.volumes = vec![volume];
        state.player_sync.selected = Some("player".into());
        state.player_sync.players = vec![crate::services::player_sync::Player {
            volume_uuid: "player".into(),
            name: "Y2".into(),
            source_id: "fixture".into(),
            playlists: vec![String::new()],
            auto_sync: true,
        }];
        let model = crate::app::player_sync::project(&state);
        let (engine, updates, mut commands) = Engine::fixture(Snapshot {
            player_sync: model,
            ..Default::default()
        });
        let engine = std::sync::Arc::new(engine);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| PlayerDevices::new(engine, window, cx));
            gpui_kit::base::Root::new(view, window, cx)
        });
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        while commands.try_recv().is_ok() {}
        let sync = cx.debug_bounds("sync-player").expect("Sync Now");
        cx.simulate_click(sync.center(), Default::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::Action(crate::app::Action::PlayerSync(PlayerCommand::Start {
                    create_playlist: false
                }))
            ))
        );
        state.player_sync.phase = Phase::Review {
            operation: 7,
            title: "Test player".into(),
            rows: std::sync::Arc::new(
                crate::services::player_sync::CHANGE_GROUPS
                    .into_iter()
                    .map(|g| (g.into(), vec!["Artist/Album/01 Track.flac".into()]))
                    .collect(),
            ),
        };
        updates.send_replace(Snapshot {
            player_sync: crate::app::player_sync::project(&state),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|w, c| w.draw(c).clear(c));
        let disclosure = cx
            .debug_bounds("player-report-header-0")
            .expect("Add disclosure");
        cx.simulate_click(disclosure.center(), Default::default());
        assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|c|matches!(c,Command::Action(crate::app::Action::PlayerSync(PlayerCommand::ReportGroup(ref g))) if g=="Add")));
        let apply = cx
            .debug_bounds("apply-player-review")
            .expect("review can be confirmed");
        cx.simulate_click(apply.center(), Default::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                c,
                Command::Action(crate::app::Action::PlayerSync(PlayerCommand::ApplyReview(
                    7
                )))
            ))
        );
        state.player_sync.phase = Phase::Copying(crate::services::player_sync::Progress {
            completed: 1,
            total: 3,
            copied: 1,
            received: 50,
            expected: Some(100),
            title: "Current song".into(),
            ..Default::default()
        });
        updates.send_replace(Snapshot {
            player_sync: crate::app::player_sync::project(&state),
            ..Default::default()
        });
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let progress = cx
            .debug_bounds("player-sync-progress")
            .expect("transfer progress");
        assert!(progress.size.height > px(20.));
        while commands.try_recv().is_ok() {}
        let eject = cx.debug_bounds("eject-player").expect("Eject");
        cx.simulate_click(eject.center(), Default::default());
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::Action(crate::app::Action::PlayerSync(PlayerCommand::Eject))
            ))
        );
    }
}
