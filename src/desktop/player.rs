//! Music-specific visualizer data and native Now Playing composition.
use super::player_model::Player;
use super::*;
use gpui_kit::assets::IconName as Glyph;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{
    Icon, Selectable, Sizable,
    button::ButtonVariants,
    description_list::DescriptionList,
    menu::{DropdownMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, v_resizable},
    scroll::{ScrollableElement, Scrollbar},
    tab::{Tab, TabBar},
};

/// Below this size the full metadata, visualizer and queue cannot fit comfortably.
pub(super) fn is_mini(viewport: Size<Pixels>) -> bool {
    viewport.width < px(720.) || viewport.height < px(520.)
}

impl Desktop {
    pub(super) fn track_detail_button(
        &self,
        id: &'static str,
        label: String,
        track: crate::library::models::Track,
        destination: crate::app::library_details::Destination,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .debug_selector(move || id.into())
            .ghost()
            .small()
            .p_0()
            .h_auto()
            .max_w_full()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.command(PaletteCommandKind::FromTrackContext {
                    kind: crate::app::track_context::ContextKind::ShowDetails(destination),
                    track: Box::new(track.clone()),
                });
                cx.stop_propagation();
            }))
    }

    fn mini_artist_link(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(track) = self.snapshot.player.track.clone() {
            self.track_detail_button(
                "mini-artist",
                track.track_artist().to_owned(),
                track,
                crate::app::library_details::Destination::Artist,
                cx,
            )
            .text_size(px(11.))
            .into_any_element()
        } else {
            div().child(self.snapshot.artist.clone()).into_any_element()
        }
    }

    fn now_playing_title(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(track) = self.snapshot.player.track.clone() else {
            return div()
                .child(self.snapshot.now_playing.clone())
                .into_any_element();
        };
        Button::new("now-playing-track-title")
            .debug_selector(|| "now-playing-track-title".into())
            .ghost()
            .small()
            .p_0()
            .h_auto()
            .max_w_full()
            .justify_start()
            .text_size(px(18.))
            .font_weight(FontWeight::SEMIBOLD)
            .accessibility_label(format!("Show Track Details: {}", track.title))
            .label(self.snapshot.now_playing.clone())
            .on_click(cx.listener(move |this, _, _, _| {
                this.command(PaletteCommandKind::FromTrackContext {
                    kind: crate::app::track_context::ContextKind::ShowDetails(
                        crate::app::library_details::Destination::Track,
                    ),
                    track: Box::new(track.clone()),
                })
            }))
            .into_any_element()
    }

    pub(super) fn fullscreen_workspace(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let content = if self.snapshot.player.viewer.is_artwork() {
            self.square_artwork(cx)
        } else {
            self.player_content(false, cx)
        };
        div()
            .debug_selector(|| "fullscreen-workspace".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "fullscreen-content".into())
                    .flex_1()
                    .min_h_0()
                    .child(content),
            )
            .when(self.snapshot.player.viewer.has_transport(), |view| {
                view.child(
                    div()
                        .debug_selector(|| "fullscreen-transport".into())
                        .h(px(super::motion::TRANSPORT_HEIGHT))
                        .flex_none()
                        .flex()
                        .child(div().flex_1().min_w_0().child(self.transport(None, cx)))
                        .child(
                            div()
                                .w(px(310.))
                                .flex_none()
                                .bg(cx.theme().title_bar)
                                .child(self.up_next(cx)),
                        ),
                )
            })
            .when(!self.snapshot.separate_player, |view| {
                view.child(
                    div().absolute().right_2().top_2().child(
                        Button::new("close-fullscreen")
                            .debug_selector(|| "close-fullscreen".into())
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Glyph::X))
                            .accessibility_label("Close Full Screen")
                            .on_click(cx.listener(|this, _, _, _| {
                                let _ = this.engine.commands.try_send(Command::ExitPlayer);
                            })),
                    ),
                )
            })
            .into_any_element()
    }

    fn square_artwork(&self, cx: &App) -> AnyElement {
        let cover = self.transport_art.clone();
        let background = cx.theme().muted;
        canvas(
            move |bounds, window, cx| {
                let side = bounds.size.width.min(bounds.size.height).max(px(0.));
                let mut element = div()
                    .size(side)
                    .rounded_lg()
                    .bg(background)
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|el| {
                        if let Some(image) = cover {
                            el.child(img(image).size_full().object_fit(ObjectFit::Contain))
                        } else {
                            el.child(Icon::new(Glyph::Disc3).size(side * 0.45))
                        }
                    })
                    .into_any_element();
                element.layout_as_root(
                    size(
                        AvailableSpace::Definite(side),
                        AvailableSpace::Definite(side),
                    ),
                    window,
                    cx,
                );
                element.prepaint_at(
                    bounds.origin
                        + point(
                            (bounds.size.width - side) / 2.,
                            (bounds.size.height - side) / 2.,
                        ),
                    window,
                    cx,
                );
                element
            },
            |_, mut element, window, cx| element.paint(window, cx),
        )
        .size_full()
        .into_any_element()
    }
    fn now_playing_metadata(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = &self.snapshot.player;
        if model.track.is_none() {
            return super::about::themed_artwork(cx);
        }
        let mut metadata = DescriptionList::horizontal()
            .columns(1)
            .small()
            .bordered(false)
            .label_width(px(80.));
        if let Some(track) = &model.track {
            let rows = crate::app::details::now_playing_rows(track);
            for (label, value) in rows {
                if label == "Artist" || label == "Album" {
                    let target = track.clone();
                    let artist = label == "Artist";
                    metadata = metadata.item(
                        label,
                        Button::new(if artist {
                            "now-playing-artist"
                        } else {
                            "now-playing-album"
                        })
                        .debug_selector(move || {
                            if artist {
                                "now-playing-artist".into()
                            } else {
                                "now-playing-album".into()
                            }
                        })
                        .ghost()
                        .small()
                        .p_0()
                        .h_auto()
                        .justify_start()
                        .accessibility_label(value.clone())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .text_left()
                                .hover(|style| style.text_decoration_1())
                                .child(value),
                        )
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.command(if artist {
                                PaletteCommandKind::JumpToArtist(Box::new(target.clone()))
                            } else {
                                PaletteCommandKind::JumpToAlbum(Box::new(target.clone()))
                            })
                        }))
                        .into_any_element(),
                        1,
                    );
                } else if label == "Genres" {
                    let generation = self.snapshot.generation;
                    metadata = metadata.item(
                        label,
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .children(track.genres.iter().map(|genre| {
                                let genre = genre.clone();
                                Button::new(SharedString::from(format!("genre-{genre}")))
                                    .ghost()
                                    .small()
                                    .p_0()
                                    .h_auto()
                                    .justify_start()
                                    .accessibility_label(genre.clone())
                                    .child(
                                        div()
                                            .hover(|style| style.text_decoration_1())
                                            .child(genre.clone()),
                                    )
                                    .on_click(cx.listener(move |this, _, _, _| {
                                        let _ =
                                            this.engine.commands.try_send(Command::BrowseGenre {
                                                generation,
                                                genre: genre.clone(),
                                            });
                                    }))
                            }))
                            .into_any_element(),
                        1,
                    );
                } else {
                    metadata = metadata.item(
                        label.clone(),
                        div()
                            .debug_selector(move || format!("now-playing-field-{label}"))
                            .text_size(px(12.))
                            .child(value)
                            .into_any_element(),
                        1,
                    );
                }
            }
        }

        metadata.into_any_element()
    }

    pub(super) fn single_player_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::motion::{self, SheetGeometry};
        use gpui_kit::base::motion::{Transition, transition};
        let expanded = self.snapshot.player.active;
        let progress = transition(
            "single-player-sheet",
            if expanded { 1_f32 } else { 0_f32 },
            Transition::new(self.presentation_duration(motion::PLAYER_DURATION)),
            window,
            cx,
        );
        let hero_side = (f32::from(self.sheet_header_height) - motion::TRANSPORT_HEIGHT - 32.)
            .max(90.)
            .min((f32::from(self.sheet_size.width) - 260.).max(90.));
        let mut geometry =
            SheetGeometry::at(progress, f32::from(self.sheet_size.height), hero_side);
        geometry.artwork_x = self.transport_artwork_x + (16. - self.transport_artwork_x) * progress;
        let transport = self.transport(Some(progress), cx).into_any_element();
        let library = self.library_workspace(window, cx);
        let metadata = (progress > 0.).then(|| self.now_playing_metadata(cx));
        let body = (progress > 0.).then(|| self.player_content(false, cx));
        let cover = self.transport_art.clone();
        let owner = cx.entity().downgrade();
        let header_owner = cx.entity().downgrade();
        let header_default = px(crate::ui::layout::player_header(
            f32::from(self.sheet_size.height),
            f32::from(self.sheet_size.width),
        ));
        let header_min = px(motion::TRANSPORT_HEIGHT + 32. + 128.).min(header_default);
        let visualizer_min = (self.sheet_size.height - header_default).max(px(80.));
        let header = resizable_panel()
            .size(header_default)
            .size_range(header_min..header_default)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let changed = header_owner
                            .update(cx, |this, _| {
                                if this.sheet_header_height == bounds.size.height {
                                    false
                                } else {
                                    this.sheet_header_height = bounds.size.height;
                                    true
                                }
                            })
                            .unwrap_or(false);
                        if changed {
                            let owner = header_owner.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = owner.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .size_full(),
            );
        let panels = v_resizable(SharedString::from(format!(
            "single-player-panels-{}",
            self.layout_epoch
        )))
        .child(header)
        .child(
            resizable_panel()
                .size_range(visualizer_min..Pixels::MAX)
                .child(
                    div()
                        .debug_selector(|| "visualizer-pane".into())
                        .size_full()
                        .opacity(progress)
                        .when_some(body, |view, body| view.child(body)),
                ),
        );
        let mut sheet = div()
            .absolute()
            .left_0()
            .bottom_0()
            .w_full()
            .h(px(geometry.height))
            .overflow_hidden()
            .occlude()
            .bg(cx.theme().background)
            .when(progress == 0., |sheet| sheet.bg(cx.theme().title_bar))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top_0()
                    .h(self.sheet_size.height)
                    .child(panels),
            );
        sheet = sheet.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(geometry.controls_y))
                .child(transport),
        );
        if let Some(metadata) = metadata {
            sheet = sheet.child(
                div()
                    .absolute()
                    .left(px(hero_side + 40.))
                    .right(px(48.))
                    .top(px(16.))
                    .h(px(hero_side))
                    .opacity(progress)
                    .debug_selector(|| "now-playing-metadata".into())
                    .when(self.snapshot.player.track.is_none(), |view| {
                        view.flex().flex_col()
                    })
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .mb_2()
                            .flex()
                            .justify_start()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.now_playing_title(cx)),
                    )
                    .child(metadata),
            );
        }

        sheet = sheet.child(
            div()
                .id("player-sheet-artwork")
                .debug_selector(|| "transport-artwork".into())
                .absolute()
                .left(px(geometry.artwork_x))
                .top(px(geometry.artwork_y))
                .size(px(geometry.artwork_side))
                .opacity(progress)
                .rounded(px(5.))
                .bg(cx.theme().muted)
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .when(!expanded, |art| {
                    art.role(Role::Button)
                        .aria_label("Show Now Playing")
                        .cursor_pointer()
                })
                .on_click(cx.listener(|this, _, _, _| {
                    if !this.snapshot.player.active {
                        this.command(PaletteCommandKind::GotoNowPlaying);
                    }
                }))
                .child(
                    cover
                        .map(|cover| {
                            img(cover)
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element()
                        })
                        .unwrap_or_else(|| {
                            Icon::new(Glyph::Music2)
                                .size(px(geometry.artwork_side * 0.45))
                                .text_color(cx.theme().muted_foreground)
                                .into_any_element()
                        }),
                ),
        );

        let sheet = sheet
            .when(expanded, |sheet| {
                sheet.child(
                    div().absolute().right_2().top_2().child(
                        Button::new("collapse-player-sheet")
                            .debug_selector(|| "collapse-player-sheet".into())
                            .ghost()
                            .small()
                            .icon(Icon::new(Glyph::ChevronDown))
                            .accessibility_label("Show Library")
                            .tooltip("Show Library · ⌘1")
                            .on_click(cx.listener(|this, _, _, _| {
                                this.command(PaletteCommandKind::GotoLibrary)
                            })),
                    ),
                )
            })
            .into_any_element();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let changed = owner
                            .update(cx, |this, _| {
                                if this.sheet_size == bounds.size {
                                    false
                                } else {
                                    this.sheet_size = bounds.size;
                                    true
                                }
                            })
                            .unwrap_or(false);
                        if changed {
                            let owner = owner.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = owner.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(div().flex_1().min_h_0().child(library))
            .child(div().flex_none().h(px(motion::TRANSPORT_HEIGHT)))
            .child(sheet)
            .into_any_element()
    }

    pub(super) fn mini_player(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut controls = div().flex().items_center().justify_center().gap_4();
        for (id, icon, label, action) in [
            (
                "mini-previous",
                Glyph::SkipBack,
                "Previous Track",
                PlaybackAction::Previous,
            ),
            (
                "mini-play",
                if self.snapshot.playing {
                    Glyph::Pause
                } else {
                    Glyph::Play
                },
                if self.snapshot.playing {
                    "Pause"
                } else {
                    "Play"
                },
                PlaybackAction::TogglePlayPause,
            ),
            (
                "mini-next",
                Glyph::SkipForward,
                "Next Track",
                PlaybackAction::Next,
            ),
            (
                "mini-lyrics",
                Glyph::MessageSquareQuote,
                "Lyrics",
                PlaybackAction::ShowLyrics,
            ),
        ] {
            controls = controls.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .size(px(36.))
                    .icon(Icon::new(icon).size(px(24.)))
                    .tooltip(label)
                    .on_click(cx.listener(move |this, _, _, _| {
                        let _ = this
                            .engine
                            .commands
                            .try_send(Command::Action(action.clone().into()));
                    })),
            );
        }
        if window.viewport_size().height < px(280.) {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .min_h_0()
                .p_2()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(72.))
                                .flex_none()
                                .child(self.square_artwork(cx)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(self.now_playing_title(cx)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(self.mini_artist_link(cx)),
                                )
                                .child(controls.justify_start().gap_3()),
                        ),
                )
                .child(
                    gpui_kit::component::slider::Slider::new(&self.seek_slider)
                        .disabled(!self.snapshot.seekable),
                )
                .into_any_element();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .gap_2()
            .p_2()
            .child({
                let side = (window.viewport_size().width - px(16.))
                    .min((window.viewport_size().height - px(156.)).max(px(0.)));
                div()
                    .debug_selector(|| "mini-artwork".into())
                    .flex_none()
                    .size(side)
                    .mx_auto()
                    .child(self.square_artwork(cx))
            })
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .text_center()
                    .text_ellipsis()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.now_playing_title(cx)),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .text_center()
                    .text_ellipsis()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.mini_artist_link(cx)),
            )
            .child(controls.flex_none())
            .child(
                gpui_kit::component::slider::Slider::new(&self.seek_slider)
                    .disabled(!self.snapshot.seekable),
            )
            .into_any_element()
    }
    pub(super) fn now_playing(
        &mut self,
        include_information: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let presentation = self.player_content(include_information, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(presentation))
            .when(self.snapshot.player.active, |view| {
                view.child(self.transport(None, cx))
            })
            .into_any_element()
    }

    pub(super) fn queue_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let queue = self.player_queue(cx);
        let queue_owner = cx.entity().downgrade();
        let measure_queue = self.role == WindowRole::Queue;
        let queue = div()
            .debug_selector(|| "now-playing-queue".into())
            .on_prepaint(move |bounds, window, cx| {
                if !measure_queue {
                    return;
                }
                let changed = queue_owner
                    .update(cx, |this, _| {
                        let changed = this.queue_width != bounds.size.width;
                        this.queue_width = bounds.size.width;
                        changed
                    })
                    .unwrap_or(false);
                if changed {
                    let owner = queue_owner.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = owner.update(cx, |_, cx| cx.notify());
                    });
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .debug_selector(|| "queue-heading".into())
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Play Queue"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child({
                                let choices = self.snapshot.queue_modes.clone();
                                let commands = self.engine.commands.clone();
                                let active = self.snapshot.queue_mode != "Manual Queue";
                                Button::new("queue-mode")
                                    .debug_selector(|| "queue-mode".into())
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(Glyph::ListStart))
                                    .selected(active)
                                    .when(active, |button| button.primary())
                                    .accessibility_label("Queue Type")
                                    .tooltip(format!("Queue Type: {}", self.snapshot.queue_mode))
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for choice in &choices {
                                            let commands = commands.clone();
                                            let command = choice.command.clone();
                                            let selected = choice.selected;
                                            menu = menu.item(
                                                PopupMenuItem::new(choice.label.clone())
                                                    .checked(selected)
                                                    .disabled(choice.disabled.is_some())
                                                    .on_click(move |_, _, _| {
                                                        if !selected {
                                                            let _ = commands.try_send(
                                                                Command::Palette(command.clone()),
                                                            );
                                                        }
                                                    }),
                                            );
                                        }
                                        menu
                                    })
                            })
                            .child(self.queue_tools(cx))
                            .when(!self.snapshot.separate_player, |header| {
                                header.child(
                                    Button::new("close-queue")
                                        .ghost()
                                        .small()
                                        .icon(Icon::new(Glyph::ListMusic))
                                        .selected(true)
                                        .primary()
                                        .accessibility_label("Hide Play Queue")
                                        .tooltip("Hide Play Queue")
                                        .on_click(cx.listener(|this, _, _, _| {
                                            let _ =
                                                this.engine.commands.try_send(Command::ToggleQueue);
                                        })),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .flex()
                    .flex_col()
                    .child(queue),
            );
        div()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus.focus(window, cx);
                this.queue_focused = true;
                cx.notify();
            }))
            .child(queue)
            .into_any_element()
    }

    pub(super) fn queue_dock(
        &mut self,
        content: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::base::motion::{Spring, spring};
        let shown = self.snapshot.queue_open && !self.snapshot.separate_player;
        let progress = spring(
            "queue-dock",
            if shown { 1_f32 } else { 0_f32 },
            Spring::new(self.presentation_duration(super::motion::SIDEBAR_DURATION)),
            window,
            cx,
        )
        .clamp(0., 1.);
        self.queue_progress = progress;
        if !shown && progress == 0. {
            self.queue_focused = false;
        }
        let queue = (progress > 0.).then(|| self.queue_panel(cx));
        let width = self.queue_width;
        let settled = shown && progress == 1.;
        let owner = cx.entity().downgrade();
        h_resizable(SharedString::from(format!(
            "queue-dock-panels-{}-{settled}",
            self.layout_epoch
        )))
        .child(
            resizable_panel().size_range(px(350.)..Pixels::MAX).child(
                div()
                    .debug_selector(|| "now-playing-main".into())
                    .size_full()
                    .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.queue_focused = false;
                        cx.notify();
                    }))
                    .child(content),
            ),
        )
        .when_some(queue, |panels, queue| {
            panels.child(
                resizable_panel()
                    .size(width * progress)
                    .fixed()
                    .size_range(if !settled {
                        width * progress..width * progress
                    } else {
                        px(240.)
                            ..(window.viewport_size().width * 0.4)
                                .min(px(480.))
                                .max(px(240.))
                    })
                    .child(
                        div()
                            .debug_selector(|| "queue-dock".into())
                            .relative()
                            .size_full()
                            .overflow_hidden()
                            .border_l_2()
                            .border_color(cx.theme().border)
                            .on_prepaint(move |bounds, window, cx| {
                                if settled {
                                    let changed = owner
                                        .update(cx, |this, _| {
                                            let changed = this.queue_width != bounds.size.width;
                                            this.queue_width = bounds.size.width;
                                            changed
                                        })
                                        .unwrap_or(false);
                                    if changed {
                                        let owner = owner.clone();
                                        window.defer(cx, move |_, cx| {
                                            let _ = owner.update(cx, |_, cx| cx.notify());
                                        });
                                    }
                                }
                            })
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(40.) * (1. - progress))
                                    .opacity(progress)
                                    .w_full()
                                    .h_full()
                                    .child(queue),
                            ),
                    ),
            )
        })
        .into_any_element()
    }

    pub(super) fn queue_tools(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let tools = self.snapshot.queue_tools.clone();
        let commands = self.engine.commands.clone();
        let repeat = self.snapshot.player.repeat;
        let generation = self.snapshot.generation;
        Button::new("queue-tools")
            .debug_selector(|| "queue-tools".into())
            .ghost()
            .small()
            .icon(Icon::new(Glyph::ClipboardPenLine))
            .accessibility_label(crate::app::queue_controls::Menu::Tools.label())
            .tooltip("Queue Tools")
            .dropdown_menu(move |mut menu, _, _| {
                let mut previous = None;
                for tool in &tools {
                    if previous.replace(tool.group).is_some_and(|group| group != tool.group) {
                        menu = menu.separator();
                    }
                    let commands = commands.clone();
                    let selected = tool.tool;
                    menu = menu.item(
                        PopupMenuItem::new(tool.label)
                            .icon(super::queue_icons::glyph(tool.tool, repeat))
                            .checked(tool.active)
                            .disabled(tool.disabled_reason.is_some())
                            .on_click(move |_, _, _| {
                                let _ = commands.try_send(Command::QueueTool {
                                    generation,
                                    tool: selected,
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn player_information(
        &mut self,
        artwork_size: f32,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let artwork = self.square_artwork(cx);
        let metadata = self.now_playing_metadata(cx);
        h_resizable(SharedString::from(format!(
            "player-information-{}-{compact}",
            self.layout_epoch
        )))
        .child(
            resizable_panel()
                .size(px(artwork_size))
                .size_range(px(80.)..px(500.))
                .child(div().size_full().overflow_hidden().child(artwork)),
        )
        .child(
            resizable_panel().child(
                div()
                    .id("now-playing-metadata")
                    .size_full()
                    .pl_4()
                    .pr_3()
                    .pt_2()
                    .debug_selector(|| "now-playing-metadata".into())
                    .when(self.snapshot.player.track.is_none(), |view| {
                        view.flex().flex_col()
                    })
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .mb_2()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.now_playing_title(cx)),
                    )
                    .child(metadata),
            ),
        )
        .into_any_element()
    }

    pub(super) fn player_content(
        &mut self,
        include_information: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let model = self.snapshot.player.clone();
        let compact = !model.active;
        let fullscreen = model.fullscreen;
        let artwork_size = if compact {
            90.
        } else {
            (f32::from(self.sheet_size.height) * 0.65).clamp(176., 500.)
        };
        if self.role == WindowRole::NowPlaying && self.snapshot.separate_player && !fullscreen {
            let information = self.player_information(artwork_size, compact, cx);
            return div()
                .size_full()
                .p_4()
                .child(information)
                .into_any_element();
        }
        let modes = TabBar::new("now-playing-modes")
            .w_full()
            .h(px(28.))
            .min_w_0()
            .track_scroll(&self.visualizer_mode_scroll)
            .menu(false)
            .segmented()
            .small()
            .text_size(px(13.))
            .children(
                crate::app::state::VisualizerTab::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        let mode = index + 1;
                        Tab::new()
                            .label(tab.name())
                            .selected(model.mode == mode)
                            .on_click(cx.listener(move |this, _, _, _| {
                                let _ = this.engine.commands.try_send(Command::PlayerMode(mode));
                            }))
                    }),
            );
        let modes = div()
            .flex()
            .min_w_0()
            .min_h(px(32.))
            .debug_selector(|| "visualizer-mode-row".into())
            .items_center()
            .justify_between()
            .gap_3()
            .when(fullscreen && !self.snapshot.separate_player, |row| {
                row.pr_6()
            })
            .child(
                div()
                    .debug_selector(|| "visualizer-mode-tabs".into())
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(44.))
                    .pb(px(16.))
                    .child(modes)
                    .child(
                        Scrollbar::horizontal(&self.visualizer_mode_scroll).viewport_from_layout(),
                    ),
            )
            .when(!fullscreen && !self.snapshot.separate_player, |row| {
                row.child(
                    Button::new("expand-visualizer")
                        .debug_selector(|| "expand-visualizer".into())
                        .ghost()
                        .small()
                        .flex_none()
                        .accessibility_label("Full-screen Visualizer")
                        .icon(Icon::new(Glyph::Maximize))
                        .tooltip("Full-screen Visualizer · F7")
                        .on_click(cx.listener(|this, _, _, _| {
                            this.command(PaletteCommandKind::ToggleFullscreen)
                        })),
                )
            });
        // The two resizers are independent; fit the cover to their smaller dimension.
        // Kit has no aspect-fit container. Keep this geometry local to music artwork.
        let artwork_mode = if model.mode == 0 {
            model
                .artwork
                .as_ref()
                .and_then(|bytes| image_from_bytes(bytes, self.snapshot.artwork_mode))
        } else {
            None
        };
        let position = if let Some((id, fraction)) = self
            .waveform_scrub
            .filter(|(id, _)| *id == self.snapshot.playback_id)
        {
            let _ = id;
            fraction
        } else if self.snapshot.duration_ms > 0 {
            self.snapshot.position_ms as f32 / self.snapshot.duration_ms as f32
        } else {
            0.
        };
        let meter_summary = if model.mode == 6 && model.meters.available {
            Some(format!(
                "L  RMS {} · Peak {}     R  RMS {} · Peak {} dBFS · Sample peaks · Hold 1.5 s",
                super::meters::label(model.meters.rms[0]),
                super::meters::label(model.meters.peak[0]),
                super::meters::label(model.meters.rms[1]),
                super::meters::label(model.meters.peak[1])
            ))
        } else {
            None
        };
        let is_meter = model.mode == 6;
        let waveform_ticks = (model.mode == 1 && !model.waveform.is_empty())
            .then(|| crate::media::time_axis::time_ticks(self.snapshot.duration_ms, 6));
        let seek_canvas = (model.mode == 1 && !model.waveform.is_empty())
            || (model.mode == 3 && model.spectra.iter().any(|bins| !bins.is_empty()));
        let colors = (cx.theme().primary, cx.theme().muted_foreground);
        let visual = div()
            .id("audio-visualizer")
            .debug_selector(|| "audio-visualizer".into())
            .relative()
            .flex_1()
            .min_h_0()
            .bg(cx.theme().background)
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        paint(&model, position, bounds, window);
                        if is_meter {
                            super::meters::paint_labels(bounds, colors.1, window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .when_some(artwork_mode, |d, image| {
                d.child(
                    img(image)
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
            })
            .when_some(waveform_ticks, |view, ticks| {
                view.child(
                    div()
                        .debug_selector(|| "waveform-time-ruler".into())
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(20.))
                        .bg(cx.theme().background.opacity(0.85))
                        .children(ticks.into_iter().flat_map(|(fraction, label)| {
                            [
                                div()
                                    .absolute()
                                    .left(relative(fraction.min(0.999)))
                                    .h(px(4.))
                                    .border_l_1()
                                    .border_color(cx.theme().muted_foreground)
                                    .into_any_element(),
                                div()
                                    .absolute()
                                    .top(px(4.))
                                    .when(fraction > 0.9, |view| view.right_0())
                                    .when(fraction <= 0.9, |view| view.left(relative(fraction)))
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(label)
                                    .into_any_element(),
                            ]
                        })),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if seek_canvas && this.snapshot.seekable {
                        this.waveform_scrub = Some((
                            this.snapshot.playback_id,
                            this.visualizer_fraction(event.position),
                        ));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some((id, _)) = this.waveform_scrub {
                    this.waveform_scrub = Some((id, this.visualizer_fraction(event.position)));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_waveform_seek(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_waveform_seek(event.position, cx)
                }),
            );
        // A separate canvas records layout in the entity without borrowing it in paint.
        let bounds_owner = cx.entity().downgrade();
        let visual = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = bounds_owner
                            .update(cx, |this, _| this.visualizer_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(visual)
            .when_some(meter_summary, |d, text| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child(text),
                )
            });
        if fullscreen || self.role == WindowRole::Visualizer {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .child(modes)
                .child(div().flex_1().min_h_0().child(visual))
                .into_any_element();
        }
        let information =
            include_information.then(|| self.player_information(artwork_size, compact, cx));
        let presentation = v_resizable(SharedString::from(format!(
            "player-presentation-{}-{compact}-{include_information}",
            self.layout_epoch
        )))
        .when_some(information, |panels, information| {
            panels.child(
                resizable_panel()
                    .size(px(artwork_size))
                    .size_range(px(176.)..px(artwork_size.max(176.)))
                    .child(information),
            )
        })
        .child(
            resizable_panel().size_range(px(100.)..Pixels::MAX).child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().flex_none().min_w_0().child(modes))
                    .child(
                        div()
                            .debug_selector(|| "visualizer-content".into())
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(visual),
                    ),
            ),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p(px(if compact { 8. } else { 16. }))
            .child(div().flex_1().min_h_0().child(presentation))
            .into_any_element()
    }

    fn visualizer_fraction(&self, point: Point<Pixels>) -> f32 {
        self.visualizer_bounds
            .map(|b| {
                (f32::from(point.x - b.origin.x) / f32::from(b.size.width).max(1.)).clamp(0., 1.)
            })
            .unwrap_or(0.)
    }
    fn finish_waveform_seek(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some((playback_id, _)) = self.waveform_scrub.take() {
            let _ = self.engine.commands.try_send(Command::Seek {
                playback_id,
                fraction: self.visualizer_fraction(point),
            });
            cx.notify();
        }
    }
    fn up_next(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::list::ListItem;
        let upcoming = self.snapshot.playback_queue.as_ref().and_then(|queue| {
            let row = queue.playing.map_or(0, |index| index.saturating_add(1));
            Some((
                queue.tracks.get(row)?.clone(),
                queue.target(self.snapshot.generation, browser::PLAYER_QUEUE, row)?,
                queue.tracks.len() - row,
            ))
        });
        let Some((track, target, remaining)) = upcoming else {
            return div()
                .debug_selector(|| "up-next-empty".into())
                .size_full()
                .px_4()
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child("Nothing queued")
                .into_any_element();
        };
        let artwork = track
            .parent_rating_key
            .as_ref()
            .and_then(|key| self.image_cache.get(key))
            .or_else(|| self.image_cache.get(&track.rating_key))
            .cloned();
        let context = crate::app::track_context::track_context_with_policy(
            crate::app::track_context::ContextPolicy {
                queued: true,
                ..self.snapshot.context_policy
            },
            &track,
            false,
        );
        div()
            .debug_selector(|| "up-next-panel".into())
            .size_full()
            .px_4()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pr(px(24.))
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child("Up Next")
                    .child(format!("{remaining} queued")),
            )
            .child(
                ListItem::new("up-next-track")
                    .debug_selector(|| "up-next-track".into())
                    .w_full()
                    .h(px(40.))
                    .px_0()
                    .py_0()
                    .accessibility_label(format!(
                        "Up Next: {} — {}",
                        track.title,
                        track.track_artist()
                    ))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(32.))
                                    .flex_none()
                                    .rounded(px(4.))
                                    .bg(cx.theme().muted)
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        artwork
                                            .map(|image| {
                                                img(image)
                                                    .size_full()
                                                    .object_fit(ObjectFit::Cover)
                                                    .into_any_element()
                                            })
                                            .unwrap_or_else(|| {
                                                Icon::new(Glyph::Music2)
                                                    .size_4()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .into_any_element()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .debug_selector(|| "up-next-title".into())
                                            .w_full()
                                            .text_size(px(12.))
                                            .text_ellipsis()
                                            .child(track.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(|| "up-next-artist".into())
                                            .w_full()
                                            .text_size(px(11.))
                                            .text_color(cx.theme().muted_foreground)
                                            .text_ellipsis()
                                            .child(track.track_artist().to_owned()),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, _| {
                        if this.snapshot.player.fullscreen {
                            let _ = this.engine.commands.try_send(Command::ExitPlayer);
                        }
                        this.command(PaletteCommandKind::GotoNowPlaying)
                    }))
                    .context_menu(move |mut menu, _, _| {
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
                        menu
                    }),
            )
            .into_any_element()
    }

    fn player_queue(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(column) = self
            .snapshot
            .playback_queue
            .as_ref()
            .filter(|column| !column.rows.is_empty())
        else {
            use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle};
            return Empty::new()
                .header(
                    EmptyHeader::new()
                        .title(EmptyTitle::new().child("Queue is empty"))
                        .description(EmptyDescription::new().child("Add music from your library.")),
                )
                .into_any_element();
        };
        let scroll = self
            .scroll_handles
            .entry(column.identity.clone())
            .or_default()
            .clone();
        let len = column.rows.len();
        let owner = cx.entity().downgrade();
        let edge_scroll = scroll.clone();
        let end_target = column.rows.len().checked_sub(1).and_then(|row| {
            column.target(self.snapshot.generation, super::browser::PLAYER_QUEUE, row)
        });
        div()
            .on_drop(cx.listener(move |this, drag: &super::QueueDrag, _, _| {
                if let Some(to) = &end_target {
                    let _ = this.engine.commands.try_send(Command::QueueDrop {
                        from: drag.target.clone(),
                        to: to.clone(),
                        after: true,
                    });
                }
                this.queue_drop_after = None;
            }))
            .on_drag_move::<super::QueueDrag>(move |event, _, _| {
                use gpui_kit::component::scroll::ScrollbarHandle;
                let y = event.event.position.y;
                let bounds = event.bounds;
                let delta = if y < bounds.top() + px(28.) {
                    14.
                } else if y > bounds.bottom() - px(28.) {
                    -14.
                } else {
                    0.
                };
                if delta != 0. {
                    let offset = edge_scroll.offset();
                    edge_scroll.set_offset(point(offset.x, (offset.y + px(delta)).min(px(0.))));
                }
            })
            .flex_1()
            .min_h_0()
            .relative()
            .debug_selector(|| "queue-scroll-viewport".into())
            .flex()
            .overflow_hidden()
            .child(
                div().flex_1().min_w_0().h_full().child(
                    uniform_list("player-queue", len, move |range, _, cx| {
                        owner
                            .update(cx, |this, cx| {
                                range
                                    .map(|row| {
                                        this.browser_row(super::browser::PLAYER_QUEUE, row, cx)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .track_scroll(&scroll)
                    .h_full(),
                ),
            )
            .child(
                div().relative().flex_none().w(px(12.)).h_full().child(
                    Scrollbar::vertical(&scroll)
                        .mode(gpui_kit::component::scroll::ScrollbarMode::Always)
                        .viewport_from_layout(),
                ),
            )
            .into_any_element()
    }
}
pub fn image_from_bytes(
    bytes: &[u8],
    mode: crate::app::state::ArtworkMode,
) -> Option<std::sync::Arc<Image>> {
    use std::hash::{Hash, Hasher};
    struct RenderCache {
        values: std::collections::VecDeque<(u64, std::sync::Arc<Image>, usize)>,
        bytes: usize,
    }
    thread_local! {
        static CACHE: std::cell::RefCell<RenderCache> = const { std::cell::RefCell::new(RenderCache { values: std::collections::VecDeque::new(), bytes: 0 }) };
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    mode.name().hash(&mut hash);
    let key = hash.finish();
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache.values.iter().position(|(id, _, _)| *id == key) {
            let entry = cache.values.remove(index)?;
            let image = entry.1.clone();
            cache.values.push_back(entry);
            return Some(image);
        }
        let (format, data) = if mode == crate::app::state::ArtworkMode::Auto {
            let format = match image::guess_format(bytes).ok()? {
                image::ImageFormat::Png => ImageFormat::Png,
                image::ImageFormat::Jpeg => ImageFormat::Jpeg,
                image::ImageFormat::WebP => ImageFormat::Webp,
                _ => return None,
            };
            (format, bytes.to_vec())
        } else {
            (
                ImageFormat::Svg,
                crate::ui::artwork::character_svg(bytes, mode)?,
            )
        };
        let size = data.len();
        let image = std::sync::Arc::new(Image::from_bytes(format, data));
        const LIMIT: usize = 32 * 1024 * 1024;
        if size <= LIMIT {
            while cache.bytes + size > LIMIT || cache.values.len() >= 256 {
                if let Some((_, _, bytes)) = cache.values.pop_front() {
                    cache.bytes = cache.bytes.saturating_sub(bytes);
                } else {
                    break;
                }
            }
            cache.bytes += size;
            cache.values.push_back((key, image.clone(), size));
        }
        Some(image)
    })
}

fn quad(window: &mut Window, origin: Point<Pixels>, width: f32, height: f32, color: Hsla) {
    if width > 0. && height > 0. {
        window.paint_quad(fill(
            Bounds::new(origin, size(px(width), px(height))),
            color,
        ));
    }
}
fn line(
    window: &mut Window,
    points: impl IntoIterator<Item = Point<Pixels>>,
    color: Hsla,
    width: f32,
) {
    let mut path = PathBuilder::stroke(px(width));
    let mut first = true;
    for point in points {
        if first {
            path.move_to(point);
            first = false;
        } else {
            path.line_to(point);
        }
    }
    if !first && let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
pub(super) fn paint(model: &Player, position: f32, bounds: Bounds<Pixels>, window: &mut Window) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    if !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
        return;
    }
    let x = f32::from(bounds.origin.x);
    let y = f32::from(bounds.origin.y);
    let point = |a: f32, b: f32| point(px(x + a * w), px(y + b * h));
    let blue: Hsla = rgb(0x007aff).into();
    match model.mode {
        1 => {
            let count = model.waveform.len().max(1) as f32;
            for (i, amplitude) in model.waveform.iter().enumerate() {
                let p = i as f32 / count;
                let height = (amplitude * h * 0.9).max(1.);
                quad(
                    window,
                    point(p, 0.5 - height / h / 2.),
                    (w / count).max(1.),
                    height,
                    if p <= position {
                        blue
                    } else {
                        rgb(0x8e8e93).into()
                    },
                );
            }
        }
        2 => {
            if let Some(bins) = model.spectra.first() {
                for (i, bin) in bins.iter().enumerate() {
                    let height = *bin as f32 / 255. * h;
                    quad(
                        window,
                        point(i as f32 / bins.len() as f32, 1. - height / h),
                        (w / bins.len() as f32 - 2.).max(1.),
                        height,
                        spectral(*bin),
                    );
                }
            }
        }
        3 => {
            if !model.spectra.iter().any(|bins| !bins.is_empty()) {
                return;
            }
            for (t, bins) in model.spectra.iter().enumerate() {
                for (f, bin) in bins.iter().enumerate() {
                    quad(
                        window,
                        point(
                            t as f32 / model.spectra.len() as f32,
                            1. - (f + 1) as f32 / bins.len() as f32,
                        ),
                        w / model.spectra.len() as f32 + 0.5,
                        h / bins.len() as f32 + 0.5,
                        spectral(*bin),
                    );
                }
            }
            line(
                window,
                [point(position, 0.), point(position, 1.)],
                gpui_kit::white(),
                1.,
            );
        }
        4 => {
            line(
                window,
                [point(0., 0.5), point(1., 0.5)],
                rgb(0x636366).into(),
                1.,
            );
            line(
                window,
                [point(0.5, 0.), point(0.5, 1.)],
                rgb(0x636366).into(),
                1.,
            );
            line(
                window,
                model
                    .xy
                    .iter()
                    .map(|(l, r)| point(0.5 + (l - r) * 0.24, 0.5 - (l + r) * 0.24)),
                rgb(0x34c759).into(),
                1.,
            );
        }
        5 => {
            for (row, bins) in model.spectra.iter().enumerate() {
                let depth = row as f32 / model.spectra.len().max(1) as f32;
                let spread = 0.14 + depth * 0.8;
                let baseline = 0.14 + depth * 0.8;
                line(
                    window,
                    bins.iter().enumerate().map(|(band, v)| {
                        point(
                            0.5 + (band as f32 / bins.len().max(1) as f32 - 0.5) * spread,
                            baseline - *v as f32 / 255. * (0.08 + depth * 0.2),
                        )
                    }),
                    spectral((80. + depth * 150.) as u8),
                    1.5,
                );
            }
        }
        6 => {
            let (top, span, history_top, history_bottom) = super::meters::geometry(h);
            let norm = super::meters::fraction;
            for db in [-60., -48., -36., -24., -12., -6., 0.] {
                let y = top + span * (-db / 60.);
                line(
                    window,
                    [point(0.26, y), point(0.76, y)],
                    rgb(0x636366).into(),
                    0.5,
                );
            }
            for channel in 0..2 {
                let x = 0.32 + channel as f32 * 0.22;
                // Fixed-color segments communicate the level on the same dBFS
                // scale as the RMS fill; an isolated peak never recolors it all.
                for segment in 0..60 {
                    let threshold = 10_f32.powf((segment as f32 - 60.) / 20.);
                    let active = model.meters.rms[channel] >= threshold;
                    let color = if segment >= 57 {
                        rgb(0xff453a)
                    } else if segment >= 48 {
                        rgb(0xffcc00)
                    } else {
                        rgb(0x34c759)
                    };
                    quad(
                        window,
                        point(x, top + span * (1. - (segment + 1) as f32 / 60.)),
                        w * 0.12,
                        (h * span / 60. - 1.).max(0.6),
                        color.opacity(if active { 0.95 } else { 0.12 }).into(),
                    );
                }
                for (level, color, width) in [
                    (model.meters.peak[channel], rgb(0xffffff), 2.),
                    (model.meters.held[channel], rgb(0xff9f0a), 2.),
                ] {
                    if level > 0.001 {
                        let y = top + span * (1. - norm(level));
                        line(
                            window,
                            [point(x, y), point(x + 0.12, y)],
                            color.into(),
                            width,
                        );
                    }
                }
            }
            for db in [0., -30., -60.] {
                let y = history_top + (history_bottom - history_top) * (-db / 60.);
                line(
                    window,
                    [point(0.26, y), point(0.76, y)],
                    rgb(0x636366).opacity(0.35).into(),
                    0.5,
                );
            }
            line(
                window,
                model.meters.history.iter().enumerate().map(|(index, db)| {
                    point(
                        0.26 + (120 - model.meters.history.len() + index) as f32 / 119. * 0.50,
                        history_bottom - *db as f32 / 60. * (history_bottom - history_top),
                    )
                }),
                blue,
                1.5,
            );
            // Correlation uses the measured stereo cross-energy; silence has no
            // correlation and therefore no invented center marker.
            line(
                window,
                [point(0.2, 0.93), point(0.8, 0.93)],
                rgb(0x636366).into(),
                2.,
            );
            line(
                window,
                [point(0.5, 0.91), point(0.5, 0.95)],
                rgb(0x8e8e93).into(),
                1.,
            );
            if let Some(correlation) = model.meters.correlation {
                let x = 0.5 + correlation * 0.3;
                line(
                    window,
                    [point(x, 0.91), point(x, 0.95)],
                    if correlation < 0. {
                        rgb(0xff9f0a)
                    } else {
                        rgb(0x34c759)
                    }
                    .into(),
                    3.,
                );
            }
        }
        _ => {}
    }
}
fn spectral(value: u8) -> Hsla {
    let value = f32::from(value) / 255.;
    hsla(0.7 - value * 0.7, 0.8, 0.04 + value * 0.6, 1.)
}
