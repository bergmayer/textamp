//! Stock Kit lists, scrollbars and resizers arranged as compact Miller columns.
use super::*;
use gpui_kit::assets::IconName as Glyph;
use gpui_kit::component::Icon;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::status_bar::StatusBar;

impl SurfaceView {
    pub(super) fn radio_browser(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let model = self.model.radio.clone().expect("radio model");
        self.radio_scroll
            .retain(|key, _| model.columns.iter().any(|column| &column.key == key));
        let stale = self.model.stale;
        let theme = self.updates.borrow().theme;
        let single = self.embedded && self.updates.borrow().sheet_browser;
        let mut single_list = None;
        let mut columns = h_resizable("radio-browser-columns");
        for (column_index, column) in model.columns.iter().enumerate() {
            if single && column_index != model.focused_column {
                continue;
            }
            let scroll = self
                .radio_scroll
                .entry(column.key.clone())
                .or_default()
                .clone();
            let owner = cx.entity().downgrade();
            let stations = column.stations.clone();
            let selected = column.selected.clone();
            let list = uniform_list(
                ("radio-stations", column_index),
                stations.len(),
                move |range, _, _| {
                    range
                        .map(|index| {
                            let station = stations[index].clone();
                            let key = station.key.clone();
                            let category = station.is_category();
                            let owner = owner.clone();
                            ListItem::new(("radio-station", index))
                                .when_some(
                                    crate::desktop::theme::aluminum_list_stripe(theme, index),
                                    |row, stripe| row.bg(stripe),
                                )
                                .role(Role::ListItem)
                                .w_full()
                                .text_size(px(13.))
                                .disabled(stale)
                                .debug_selector(move || format!("radio-row-{column_index}-{index}"))
                                .selected(selected.as_ref() == Some(&station.key))
                                .h(px(32.))
                                .px_2()
                                .py_0()
                                .accessibility_label(station.title.clone())
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_ellipsis_middle()
                                        .child(station.title),
                                )
                                .when(category, |item| {
                                    item.suffix(|_, _| Icon::new(Glyph::ChevronRight).size_3())
                                })
                                .on_click(move |_, window, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        if this.model.stale {
                                            return;
                                        }
                                        this.reveal_selection = false;
                                        this.focus.focus(window, cx);
                                        this.send(Operation::RadioSelect {
                                            column: column_index,
                                            key: key.clone(),
                                        });
                                    });
                                })
                                .into_any_element()
                        })
                        .collect()
                },
            )
            .track_scroll(&scroll)
            .h_full();
            let body = div()
                .debug_selector(move || format!("radio-column-{column_index}"))
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(crate::desktop::theme::column_header_height(theme))
                        .flex_none()
                        .px_3()
                        .flex()
                        .items_center()
                        .when(
                            theme != crate::app::theme::ThemeName::BrushedAluminum,
                            |header| header.border_b_1().border_color(cx.theme().border),
                        )
                        .bg(cx.theme().muted.opacity(0.35))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::util::column_heading::capitalized(&column.title)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(div().flex_1().min_w_0().h_full().child(list))
                        .child(
                            div()
                                .w(px(12.))
                                .h_full()
                                .flex_none()
                                .child(Scrollbar::vertical(&scroll).viewport_from_layout()),
                        ),
                );
            if single {
                single_list = Some(body);
            } else {
                columns = columns.child(
                    resizable_panel()
                        .size(px(220.))
                        .size_range(px(160.)..px(340.))
                        .child(body),
                );
            }
        }
        let mut detail = div()
            .when(!single, |v| v.size_full())
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        if let Some(station) = &model.selected {
            detail = detail.child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(station.description_text().to_owned()),
            );
            if !station.is_category() {
                let key = station.key.clone();
                detail = detail.child(
                    Button::new("play-radio-station")
                        .debug_selector(|| "play-radio-station".into())
                        .primary()
                        .small()
                        .self_start()
                        .icon(Icon::new(Glyph::Play))
                        .label("Play Station")
                        .disabled(self.model.stale || model.loading)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.send(Operation::Actions(vec![
                                crate::app::action::RadioAction::PlayStation(key.clone()).into(),
                            ]));
                        })),
                );
            }
        } else if !model.loading {
            use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle};
            detail = detail.child(
                Empty::new().header(
                    EmptyHeader::new()
                        .title(EmptyTitle::new().child("No stations"))
                        .description(
                            EmptyDescription::new().child("Select a library with radio support."),
                        ),
                ),
            );
        }
        if model.loading {
            detail = detail.child(Spinner::new().small());
        }
        let pane = self.updates.borrow().workspace.clone();
        let pane_title = pane
            .as_ref()
            .map(|pane| pane.title.clone())
            .unwrap_or_else(|| {
                model
                    .selected
                    .as_ref()
                    .map_or("Radio", |station| station.title.as_str())
                    .to_owned()
            });
        let show_detail = pane.is_some() || single_list.is_none();
        let detail = if let Some(pane) = pane {
            if self
                .inline_detail
                .as_ref()
                .is_none_or(|view| !view.read(cx).matches(&pane))
            {
                let entity = cx.new(|cx| {
                    Self::new(
                        pane,
                        true,
                        self.commands.clone(),
                        self.updates.clone(),
                        window,
                        cx,
                    )
                });
                entity.update(cx, |view, cx| view.focus_initial(window, cx));
                self.inline_detail = Some(entity);
            }
            self.inline_detail
                .as_ref()
                .unwrap()
                .clone()
                .into_any_element()
        } else {
            self.inline_detail = None;
            detail.into_any_element()
        };
        let content = if single {
            if show_detail {
                detail
            } else {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(single_list.unwrap()))
                    .when(
                        model.selected.as_ref().is_some_and(|s| !s.is_category()),
                        |v| v.child(div().flex_none().child(detail)),
                    )
                    .into_any_element()
            }
        } else {
            columns
                .child(
                    resizable_panel().size_range(px(220.)..Pixels::MAX).child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .debug_selector(|| "radio-detail-header".into())
                                    .h(crate::desktop::theme::column_header_height(theme))
                                    .flex_none()
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .when(
                                        theme != crate::app::theme::ThemeName::BrushedAluminum,
                                        |header| {
                                            header.border_b_1().border_color(cx.theme().border)
                                        },
                                    )
                                    .bg(cx.theme().muted.opacity(0.35))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_ellipsis_middle()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(crate::util::column_heading::capitalized(
                                                &pane_title,
                                            )),
                                    ),
                            )
                            .child(div().flex_1().min_h_0().child(detail)),
                    ),
                )
                .into_any_element()
        };
        let mut footer = StatusBar::new()
            .bg(cx.theme().title_bar)
            .flex_none()
            .min_h(px(36.))
            .border_t_1()
            .border_color(cx.theme().border)
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2();
        for (index, item) in self.model.buttons.iter().enumerate() {
            let operation = item.operation.clone();
            footer = footer.left(
                Button::new(("radio-tool", index))
                    .small()
                    .debug_selector(move || format!("radio-tool-{index}"))
                    .label(item.label.clone())
                    .disabled(self.model.stale)
                    .on_click(cx.listener(move |this, _, _, _| this.send(operation.clone()))),
            );
        }
        div()
            .debug_selector(|| "radio-browser".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .track_focus(&self.focus)
            .key_context("TextampSurface")
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::actions::Cancel, window, cx| {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &super::super::CloseWindow, window, cx| {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.focus.is_focused(window)
                    || event.keystroke.modifiers != Modifiers::default()
                {
                    return;
                }
                let key = match event.keystroke.key.as_str() {
                    "up" => crossterm::event::KeyCode::Up,
                    "down" => crossterm::event::KeyCode::Down,
                    "left" => crossterm::event::KeyCode::Left,
                    "right" => crossterm::event::KeyCode::Right,
                    "enter" => crossterm::event::KeyCode::Enter,
                    _ => return,
                };
                this.reveal_selection = true;
                this.send(Operation::Key(key));
                cx.stop_propagation();
            }))
            .when(!self.embedded, |view| {
                view.child(TitleBar::new().child("Radio"))
            })
            .when(self.model.stale, |view| {
                view.child(
                    gpui_kit::component::alert::Alert::info(
                        "previous-library",
                        "From another library — reopen Radio to browse the active library.",
                    )
                    .small(),
                )
            })
            .child(div().flex_1().min_h_0().child(content))
            .when(!self.model.buttons.is_empty(), |view| {
                view.child(
                    div()
                        .debug_selector(|| "radio-tools-footer".into())
                        .flex_none()
                        .child(footer),
                )
            })
            .into_any_element()
    }
}

#[cfg(all(test, feature = "desktop-tests"))]
mod tests {
    use super::{Command, Operation, Snapshot, SurfaceView};
    use gpui_kit::gpui::{self, AppContext, px, size};

    #[gpui::test]
    fn radio_uses_compact_columns_and_requires_explicit_play(cx: &mut gpui::TestAppContext) {
        cx.update(crate::desktop::init);
        let mut state = crate::app::AppState::new();

        let stations = crate::library::station::standard_stations(
            "nav-radio",
            crate::library::capabilities::NAVIDROME,
        );
        let genre_index = stations
            .iter()
            .position(|station| station.identifier.as_deref() == Some("style"))
            .unwrap();
        state
            .station_nav
            .columns
            .push(crate::app::state::StationColumn::new(
                None,
                "Stations".into(),
                stations,
            ));
        let model = crate::desktop::surface::radio_surface(&state);
        let snapshot = Snapshot {
            surface: Some(model.clone()),
            ..Default::default()
        };
        let (engine, _updates, mut commands) = crate::desktop::Engine::fixture(snapshot);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = slot.clone();
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
            capture.replace(Some(view.clone()));
            gpui_kit::base::Root::new(view, window, cx)
        });
        let view = slot.borrow().clone().unwrap();
        cx.simulate_resize(size(px(940.), px(620.)));
        for _ in 0..4 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(cx.debug_bounds("radio-tools-footer").is_none());
        let root = cx.debug_bounds("radio-column-0").expect("station column");
        assert!((px(160.)..=px(340.)).contains(&root.size.width));
        assert!(root.size.height > px(500.));
        let first_row = cx.debug_bounds("radio-row-0-0").unwrap();
        assert!(
            first_row.size.width >= root.size.width - px(20.),
            "selection must fill its column: {first_row:?} {root:?}"
        );
        let selector = format!("radio-row-0-{genre_index}");
        let row = cx
            .debug_bounds(Box::leak(selector.into_boxed_str()))
            .unwrap();
        cx.simulate_click(row.center(), Default::default());
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Surface { operation: Operation::RadioSelect {column:0,key}, .. } if key == "nav-radio/style"
        )));
        crate::desktop::radio::select(&mut state, 0, "nav-radio/style");
        let template = state.station_nav.columns[0].stations()[genre_index].clone();
        let children = (0..300)
            .map(|index| {
                let mut station = template.clone();
                station.key = format!("nav-radio/style?id=Genre{index}");
                station.title = format!("Genre {index}");
                station
            })
            .collect();
        state
            .station_nav
            .columns
            .push(crate::app::state::StationColumn::new(
                Some("nav-radio/style".into()),
                "Genres".into(),
                children,
            ));
        state.station_nav.focused_column = 1;
        let model = crate::desktop::surface::radio_surface(&state);
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.model = model;
                cx.notify();
            })
        });
        for _ in 0..4 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let child = cx.debug_bounds("radio-column-1").expect("genre column");
        assert!(child.left() >= root.left());
        assert!((px(160.)..=px(340.)).contains(&child.size.width));
        assert!(state.queue.tracks.is_empty());
        let play = cx
            .debug_bounds("play-radio-station")
            .expect("explicit Play Station");
        assert!(
            play.size.width < px(220.),
            "Play Station should keep its intrinsic control width: {play:?}"
        );
        while commands.try_recv().is_ok() {}
        cx.simulate_click(play.center(), Default::default());
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Surface {operation: Operation::Actions(actions),..} if matches!(actions.as_slice(),
                [crate::app::Action::Radio(crate::app::action::RadioAction::PlayStation(key))] if key == "nav-radio/style?id=Genre0")
        )));
        state.playback_mode = crate::app::state::PlaybackMode::Radio;
        let model = crate::desktop::surface::radio_surface(&state);
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.model = model;
                cx.notify();
            })
        });
        for _ in 0..4 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(
            cx.debug_bounds("radio-tools-footer").is_some(),
            "active radio keeps its Manual Queue control"
        );
    }
}
