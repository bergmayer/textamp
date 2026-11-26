//! Native terminal surface painted from libghostty-vt render state.
use super::engine::Command;
use crate::{
    app::windowed_presentation::Identity,
    text_mode::{
        Input, PresentedFrame,
        backend::{Appearance, Metrics, Viewport},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use gpui_kit::component::{
    ActiveTheme,
    input::{Copy, Cut, Paste, SelectAll},
};
use gpui_kit::{prelude::*, *};
use std::{collections::HashMap, ops::Range, sync::Arc};

struct Preedit {
    target: crate::app::terminal_text::Target,
    replacement: Range<usize>,
    text: String,
    selection: Range<usize>,
}

struct GlyphSpan {
    origin: Point<Pixels>,
    line: ShapedLine,
}

fn sidebar_glyph(
    frame: &PresentedFrame,
    x: usize,
    y: usize,
) -> Option<(ratatui::layout::Rect, gpui_kit::assets::IconName)> {
    let cell = frame.frame.cells.get(y)?.get(x)?;
    if cell.text.is_empty() || cell.text == " " {
        return None;
    }
    use gpui_kit::assets::IconName;
    let (area, symbol) = frame.sidebar_symbols.iter().find(|(area, symbol)| {
        cell.text == *symbol && area.contains(ratatui::layout::Position::new(x as u16, y as u16))
    })?;
    let glyph = match *symbol {
        "♫" => IconName::ListMusic,
        "◷" => IconName::ClockArrowLeft,
        "◉" => IconName::ListStart,
        "✎" => IconName::ClipboardPenLine,
        "×" => IconName::Trash,
        "⌃" => IconName::ChevronUp,
        "⌄" => IconName::ChevronDown,
        "‹" => IconName::ChevronLeft,
        "◧" => IconName::PanelLeft,
        "≋" => IconName::Radio,
        "⚙" => IconName::Settings2,
        "/" => IconName::Search,
        ":" => IconName::Command,
        "←" => IconName::ChevronLeft,
        _ => return None,
    };
    Some((*area, glyph))
}

fn transport_glyph(
    control: crate::app::transport_controls::TransportControl,
    playing: bool,
    muted: bool,
) -> Option<gpui_kit::assets::IconName> {
    use crate::app::transport_controls::TransportControl;
    use gpui_kit::assets::IconName;
    match control {
        TransportControl::Sidebar => Some(IconName::PanelLeft),
        TransportControl::Clear => Some(IconName::Trash),
        TransportControl::Queue => Some(IconName::ListMusic),
        TransportControl::Previous => Some(IconName::SkipBack),
        TransportControl::Next => Some(IconName::SkipForward),
        TransportControl::Menu => Some(IconName::Ellipsis),
        TransportControl::Lyrics => Some(IconName::MessageSquareQuote),
        TransportControl::Mute => Some(if muted {
            IconName::VolumeX
        } else {
            IconName::Volume2
        }),
        TransportControl::Play => Some(if playing {
            IconName::Pause
        } else {
            IconName::Play
        }),
        _ => None,
    }
}
fn transport_cell(frame: &PresentedFrame, x: usize, y: usize) -> bool {
    frame.transport_controls.iter().any(|(area, control)| {
        transport_glyph(*control, false, false).is_some()
            && area.contains(ratatui::layout::Position::new(x as u16, y as u16))
    })
}

pub(super) struct TextSurface {
    commands: super::engine::CommandPort,
    identity: Identity,
    frame: Option<Arc<PresentedFrame>>,
    pub(super) player: super::player_model::Player,
    focus: FocusHandle,
    bounds: Bounds<Pixels>,
    sent: Option<Viewport>,
    sequence: u64,
    pub(super) geometry: crate::ui::layout::WindowGeometry,
    cell: Size<Pixels>,
    selection: Option<((u16, u16), (u16, u16))>,
    selecting: bool,
    edit_drag: Option<usize>,
    editor: Option<crate::app::terminal_text::Document>,
    composition: Option<Preedit>,
    modifiers: Modifiers,
    repeat: bool,
    images: HashMap<(u32, u64), Arc<RenderImage>>,
    cursor_phase: bool,
    _blink: Task<()>,
    _subscriptions: Vec<Subscription>,
}
impl TextSurface {
    pub fn new(
        commands: super::engine::CommandPort,
        identity: Identity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let subscription = cx.observe_window_activation(window, |this, window, _| {
            this.send(Input::Focus(window.is_window_active()))
        });
        let release = cx.on_release_in(window, |this, window, cx| {
            for (_, image) in this.images.drain() {
                cx.drop_image(image, Some(window));
            }
        });
        let blink = cx.spawn_in(window, async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                if cx
                    .update(|window, cx| {
                        entity.update(cx, |this, cx| {
                            if window.is_window_active()
                                && this.frame.as_ref().is_some_and(|frame| {
                                    frame.frame.cursor.visible != 0
                                        && frame.frame.cursor.blinking != 0
                                })
                            {
                                this.cursor_phase = !this.cursor_phase;
                                cx.notify();
                            }
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            commands,
            cursor_phase: true,
            _blink: blink,
            identity,
            frame: None,
            player: Default::default(),
            focus,
            bounds: Bounds::default(),
            sent: None,
            sequence: 0,
            geometry: Default::default(),
            cell: size(px(9.), px(20.)),
            selection: None,
            selecting: false,
            edit_drag: None,
            editor: None,
            composition: None,
            modifiers: Modifiers::default(),
            repeat: false,
            images: HashMap::new(),
            _subscriptions: vec![subscription, release],
        }
    }
    pub fn apply(
        &mut self,
        frame: Arc<PresentedFrame>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if frame.identity != self.identity {
            return;
        }
        if self
            .frame
            .as_ref()
            .is_some_and(|previous| previous.serial == frame.serial)
        {
            return;
        }
        self.images.retain(|key, image| {
            let live = frame
                .frame
                .images
                .iter()
                .any(|placement| (placement.id, placement.generation) == *key);
            if !live {
                cx.drop_image(image.clone(), Some(window));
            }
            live
        });
        if self.editor.as_ref().map(|editor| &editor.target)
            != frame.editor.as_ref().map(|editor| &editor.target)
        {
            self.edit_drag = None;
            self.composition = None;
        }
        self.editor = frame.editor.clone();
        self.frame = Some(frame);
        cx.notify();
    }
    fn input_region(&self) -> Option<&crate::app::presentation::TextInputRegion> {
        let region = self.frame.as_ref()?.input_region.as_ref()?;
        (self.editor.as_ref()?.target == region.target).then_some(region)
    }
    fn display_document(&self) -> Option<crate::app::terminal_text::Document> {
        let mut document = self.editor.clone()?;
        document.text = self.input_document()?;
        if let Some(preedit) = &self.composition {
            document.selection = preedit.replacement.start + preedit.selection.start
                ..preedit.replacement.start + preedit.selection.end;
        }
        Some(document)
    }
    fn display_region(&self) -> Option<crate::app::presentation::TextInputRegion> {
        let region = self.input_region()?;
        if self.composition.is_none() {
            return Some(region.clone());
        }
        let document = self.display_document()?;
        crate::ui::widgets::text_input::layout(
            &document,
            region.area,
            ratatui::style::Style::default(),
        )
        .map(|(_, region)| region)
    }
    fn input_column(&self, index: usize) -> Option<u16> {
        Some(self.display_region()?.column(index))
    }
    fn select_input(&mut self, anchor: usize, offset: usize) {
        let Some(revision) = self
            .frame
            .as_ref()
            .map(|frame| frame.viewport.revision)
            .filter(|revision| {
                self.sent
                    .is_some_and(|viewport| viewport.revision == *revision)
            })
        else {
            return;
        };
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let range = anchor.min(offset)..anchor.max(offset);
        editor.selection = range.clone();
        let target = editor.target.clone();
        let _ = self.commands.try_send(Command::TextInput {
            identity: self.identity,
            viewport_revision: Some(revision),
            input: Input::Edit(crate::app::terminal_text::Edit::Select { target, range }),
        });
        self.cursor_phase = true;
    }
    pub fn activate(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    pub fn prepare(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
        self.bounds = bounds;
        let font_id = window.text_system().resolve_font(&font("Menlo"));
        let scale = window.scale_factor().max(0.5);
        let cell_width = (f32::from(
            window
                .text_system()
                .em_advance(font_id, px(14.))
                .unwrap_or(px(9.)),
        ) * scale)
            .round()
            .max(1.) as u16;
        let cell_height = (20. * scale).round().max(1.) as u16;
        self.cell = size(
            px(f32::from(cell_width) / scale),
            px(f32::from(cell_height) / scale),
        );
        let rgb = |value: Hsla| {
            let value = value.to_rgb();
            [
                (value.r * 255.).round() as u8,
                (value.g * 255.).round() as u8,
                (value.b * 255.).round() as u8,
            ]
        };
        let metrics = Metrics {
            columns: (f32::from(bounds.size.width) / f32::from(self.cell.width))
                .floor()
                .clamp(1., f32::from(u16::MAX)) as u16,
            rows: (f32::from(bounds.size.height) / f32::from(self.cell.height))
                .floor()
                .clamp(1., f32::from(u16::MAX)) as u16,
            cell_width,
            cell_height,
        };
        let appearance = Appearance {
            system: Some(super::theme::terminal_palette(cx)),
            foreground: rgb(cx.theme().foreground),
            background: rgb(cx.theme().background),
        };
        if self.sent.is_some_and(|old| {
            old.metrics == metrics
                && old.appearance == appearance
                && old.scale_factor == scale
                && old.geometry == self.geometry
        }) {
            return;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let viewport = Viewport {
            metrics,
            appearance,
            geometry: self.geometry,
            scale_factor: scale,
            revision: self.sequence,
        };
        if self
            .commands
            .try_send(Command::TextViewport {
                identity: self.identity,
                viewport,
            })
            .is_ok()
        {
            self.sent = Some(viewport);
        }
    }
    fn send(&self, input: Input) {
        let viewport_revision = if matches!(&input, Input::Mouse(_)) {
            let Some(frame) = self.frame.as_ref() else {
                return;
            };
            if self
                .sent
                .is_none_or(|sent| sent.revision != frame.viewport.revision)
            {
                return;
            }
            Some(frame.viewport.revision)
        } else {
            None
        };
        let _ = self.commands.try_send(Command::TextInput {
            identity: self.identity,
            viewport_revision,
            input,
        });
    }
    fn frame_cell(&self) -> Size<Pixels> {
        self.frame.as_ref().map_or(self.cell, |frame| {
            let scale = frame.viewport.scale_factor.max(0.5);
            size(
                px(f32::from(frame.viewport.metrics.cell_width) / scale),
                px(f32::from(frame.viewport.metrics.cell_height) / scale),
            )
        })
    }
    fn cell_at(&self, position: Point<Pixels>) -> (u16, u16) {
        let cell = self.frame_cell();
        let position = position - self.bounds.origin;
        let cols = self.frame.as_ref().map_or(1, |frame| frame.frame.columns);
        let rows = self.frame.as_ref().map_or(1, |frame| frame.frame.rows);
        (
            ((f32::from(position.x) / f32::from(cell.width))
                .floor()
                .max(0.) as u16)
                .min(cols.saturating_sub(1)),
            ((f32::from(position.y) / f32::from(cell.height))
                .floor()
                .max(0.) as u16)
                .min(rows.saturating_sub(1)),
        )
    }
    fn mouse(&self, kind: MouseEventKind, position: Point<Pixels>, modifiers: Modifiers) {
        let (column, row) = self.cell_at(position);
        self.send(Input::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: key_modifiers(modifiers),
        }));
    }
    fn selected(&self, x: u16, y: u16) -> bool {
        self.selection.is_some_and(|(start, end)| {
            let (start, end) = if (start.1, start.0) <= (end.1, end.0) {
                (start, end)
            } else {
                (end, start)
            };
            (y, x) >= (start.1, start.0) && (y, x) <= (end.1, end.0)
        })
    }
    fn edit(&self, edit: crate::app::terminal_text::Edit) {
        self.send(Input::Edit(edit));
    }
    fn copy_input(&self, cx: &mut App) -> bool {
        let Some(editor) = &self.editor else {
            return false;
        };
        if !editor.secret {
            let units = editor.text.encode_utf16().collect::<Vec<_>>();
            let start = editor.selection.start.min(units.len());
            let end = editor.selection.end.min(units.len()).max(start);
            if start < end {
                cx.write_to_clipboard(ClipboardItem::new_string(String::from_utf16_lossy(
                    &units[start..end],
                )));
            }
        }
        true
    }
    fn input_document(&self) -> Option<String> {
        let editor = self.editor.as_ref()?;
        if let Some(preedit) = self
            .composition
            .as_ref()
            .filter(|preedit| preedit.target == editor.target)
        {
            let range = &preedit.replacement;
            let mut units = editor.text.encode_utf16().collect::<Vec<_>>();
            let start = range.start.min(units.len());
            let end = range.end.min(units.len()).max(start);
            let text = if editor.secret {
                "•".repeat(preedit.text.encode_utf16().count())
            } else {
                preedit.text.clone()
            };
            units.splice(start..end, text.encode_utf16());
            Some(String::from_utf16_lossy(&units))
        } else {
            Some(editor.text.clone())
        }
    }
    fn copy(&self, cx: &mut App) {
        if self.copy_input(cx) {
            return;
        }

        let (Some((start, end)), Some(frame)) = (self.selection, &self.frame) else {
            return;
        };
        let (start, end) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let mut lines = Vec::new();
        for y in start.1..=end.1 {
            let Some(row) = frame.frame.cells.get(usize::from(y)) else {
                continue;
            };
            let first = if y == start.1 { start.0 } else { 0 };
            let last = if y == end.1 {
                end.0
            } else {
                frame.frame.columns.saturating_sub(1)
            };
            let text = row
                .iter()
                .enumerate()
                .filter(|(x, cell)| {
                    *x >= usize::from(first)
                        && *x <= usize::from(last)
                        && cell.wide != 2
                        && cell.wide != 3
                })
                .map(|(_, cell)| {
                    if cell.text.is_empty() {
                        " "
                    } else {
                        cell.text.as_str()
                    }
                })
                .collect::<String>();
            lines.push(text.trim_end().to_owned());
        }
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
    }
    fn shape(&self, bounds: Bounds<Pixels>, window: &mut Window) -> Vec<GlyphSpan> {
        let Some(frame) = &self.frame else {
            return Vec::new();
        };
        let scale = frame.viewport.scale_factor.max(0.5);
        let cell = size(
            px(f32::from(frame.viewport.metrics.cell_width) / scale),
            px(f32::from(frame.viewport.metrics.cell_height) / scale),
        );
        let mut spans = Vec::new();
        for (y, row) in frame.frame.cells.iter().enumerate() {
            let mut x = 0;
            while x < row.len() {
                let item = &row[x];
                let origin = bounds.origin + point(cell.width * x as f32, cell.height * y as f32);
                let foreground = if item.flags & 16 != 0 {
                    item.background
                } else {
                    item.foreground
                };
                if transport_cell(frame, x, y)
                    || sidebar_glyph(frame, x, y).is_some()
                    || super::text_boxdrawing::supports(&item.text)
                {
                    x += 1;
                    continue;
                }
                if item.text.is_empty()
                    || item.text == " "
                    || item.text.starts_with('\u{10eeee}')
                    || item.wide >= 2
                    || item.flags & 32 != 0
                {
                    x += 1;
                    continue;
                }
                let start = x;
                let mut text = String::new();
                if item.wide == 1 {
                    text.push_str(&item.text);
                    x += 1;
                } else {
                    while x < row.len() {
                        let next = &row[x];
                        if next.flags != item.flags
                            || next.foreground != item.foreground
                            || next.background != item.background
                            || next.wide != 0
                            || next.text.starts_with('\u{10eeee}')
                            || transport_cell(frame, x, y)
                            || sidebar_glyph(frame, x, y).is_some()
                            || super::text_boxdrawing::supports(&next.text)
                        {
                            break;
                        }
                        text.push_str(if next.text.is_empty() {
                            " "
                        } else {
                            &next.text
                        });
                        x += 1;
                    }
                }
                if x == start {
                    x += 1;
                    continue;
                }
                let mut font = font("Menlo");
                if item.flags & 1 != 0 {
                    font.weight = FontWeight::BOLD;
                }
                if item.flags & 2 != 0 {
                    font.style = FontStyle::Italic;
                }
                let color =
                    rgb_color(foreground).opacity(if item.flags & 4 != 0 { 0.6 } else { 1. });
                let line = window.text_system().shape_line(
                    text.clone().into(),
                    px(14.),
                    &[TextRun {
                        len: text.len(),
                        font,
                        color,
                        underline: (item.flags & 0x700 != 0).then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(color),
                            wavy: false,
                        }),
                        strikethrough: (item.flags & 64 != 0).then_some(StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(color),
                        }),
                        ..Default::default()
                    }],
                    if item.wide == 1 {
                        None
                    } else {
                        Some(cell.width)
                    },
                );
                spans.push(GlyphSpan { origin, line });
            }
        }
        spans
    }
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        spans: &[GlyphSpan],
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(frame) = self.frame.clone() else {
            return;
        };
        let scale = frame.viewport.scale_factor.max(0.5);
        let cell = size(
            px(f32::from(frame.viewport.metrics.cell_width) / scale),
            px(f32::from(frame.viewport.metrics.cell_height) / scale),
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_layer(bounds, |window| {
                self.paint_images(0, bounds, cell, scale, &frame, window)
            });
            window.paint_layer(bounds, |window| {
                for (y, row) in frame.frame.cells.iter().enumerate() {
                    for (x, item) in row.iter().enumerate() {
                        let mut background = item.background;
                        if item.flags & 16 != 0 {
                            background = item.foreground;
                        }
                        let color = if self.selected(x as u16, y as u16) {
                            cx.theme().selection
                        } else {
                            rgb_color(background)
                        };
                        window.paint_quad(fill(
                            Bounds::new(
                                bounds.origin
                                    + point(cell.width * x as f32, cell.height * y as f32),
                                cell,
                            ),
                            color,
                        ));
                    }
                }
            });
            window.paint_layer(bounds, |window| {
                self.paint_images(1, bounds, cell, scale, &frame, window)
            });
            window.paint_layer(bounds, |window| {
                for (y, row) in frame.frame.cells.iter().enumerate() {
                    for (x, item) in row.iter().enumerate() {
                        let foreground = if item.flags & 16 != 0 {
                            item.background
                        } else {
                            item.foreground
                        };
                        if let Some((area, glyph)) = sidebar_glyph(&frame, x, y) {
                            let extent = cell.height.min(px(14.));
                            let origin = bounds.origin
                                + point(
                                    cell.width * (f32::from(area.x) + f32::from(area.width) / 2.)
                                        - extent / 2.,
                                    cell.height * f32::from(area.y) + (cell.height - extent) / 2.,
                                );
                            let color = rgb_color(foreground).opacity(if item.flags & 4 != 0 {
                                0.6
                            } else {
                                1.
                            });
                            if let Err(error) = window.paint_svg(
                                Bounds::new(origin, size(extent, extent)),
                                glyph.path(),
                                None,
                                Default::default(),
                                color,
                                cx,
                            ) {
                                tracing::warn!(%error, "Terminal queue icon drawing failed");
                            }
                        }
                        if transport_cell(&frame, x, y) {
                            continue;
                        }
                        super::text_boxdrawing::paint(
                            &item.text,
                            Bounds::new(
                                bounds.origin
                                    + point(cell.width * x as f32, cell.height * y as f32),
                                cell,
                            ),
                            rgb_color(foreground),
                            window,
                        );
                    }
                }
                for (area, control) in &frame.transport_controls {
                    let Some(glyph) = transport_glyph(*control, frame.playing, frame.muted) else {
                        continue;
                    };
                    let color = frame
                        .frame
                        .cells
                        .get(usize::from(area.y))
                        .and_then(|row| row.get(usize::from(area.x + area.width / 2)))
                        .map_or(cx.theme().foreground, |cell| rgb_color(cell.foreground));
                    let extent = px(
                        if matches!(
                            control,
                            crate::app::transport_controls::TransportControl::Previous
                                | crate::app::transport_controls::TransportControl::Play
                                | crate::app::transport_controls::TransportControl::Next
                        ) {
                            24.
                        } else {
                            16.
                        },
                    );
                    let origin = bounds.origin
                        + point(
                            cell.width * (f32::from(area.x) + f32::from(area.width) / 2.)
                                - extent / 2.,
                            cell.height * (f32::from(area.y) + f32::from(area.height) / 2.)
                                - extent / 2.,
                        );
                    if let Err(error) = window.paint_svg(
                        Bounds::new(origin, size(extent, extent)),
                        glyph.path(),
                        None,
                        Default::default(),
                        color,
                        cx,
                    ) {
                        tracing::warn!(%error, "Terminal transport icon drawing failed");
                    }
                }
                for span in spans {
                    if let Err(error) =
                        span.line
                            .paint(span.origin, cell.height, TextAlign::Left, None, window, cx)
                    {
                        tracing::warn!(%error, "Terminal glyph drawing failed");
                    }
                }
            });
            window.paint_layer(bounds, |window| {
                self.paint_images(2, bounds, cell, scale, &frame, window)
            });
            if let Some(area) = frame.graphical_visualizer {
                let plot = Bounds::new(
                    bounds.origin
                        + point(
                            cell.width * f32::from(area.x),
                            cell.height * f32::from(area.y),
                        ),
                    size(
                        cell.width * f32::from(area.width),
                        cell.height * f32::from(area.height),
                    ),
                );
                window.paint_layer(plot, |window| {
                    window.paint_quad(fill(plot, rgb_color(frame.viewport.appearance.background)));
                    let position = if frame.duration_ms > 0 {
                        frame.position_ms as f32 / frame.duration_ms as f32
                    } else {
                        0.
                    };
                    super::player::paint(&self.player, position, plot, window);
                    if self.player.mode == 1 && !self.player.waveform.is_empty() {
                        let mut text_style = window.text_style();
                        text_style.font_family = "Menlo".into();
                        text_style.color = cx.theme().muted_foreground;
                        let ruler_y = plot.bottom() - px(20.);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(plot.left(), ruler_y),
                                size(plot.size.width, px(20.)),
                            ),
                            rgb_color(frame.viewport.appearance.background),
                        ));
                        for (fraction, label) in
                            crate::media::time_axis::time_ticks(frame.duration_ms, 6)
                        {
                            let text: SharedString = label.into();
                            let line = window.text_system().shape_line(
                                text.clone(),
                                px(10.),
                                &[text_style.to_run(text.len())],
                                None,
                            );
                            let x = if fraction > 0.9 {
                                plot.right() - line.width
                            } else {
                                plot.left() + plot.size.width * fraction
                            };
                            let _ = line.paint(
                                point(x, ruler_y + px(4.)),
                                px(14.),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    if self.player.mode == 6 {
                        super::meters::paint_labels(plot, cx.theme().muted_foreground, window, cx);
                    }
                });
            }
            let cursor = frame.frame.cursor;
            if cursor.visible != 0 && (cursor.blinking == 0 || self.cursor_phase) {
                let origin = bounds.origin
                    + point(
                        cell.width * f32::from(cursor.x),
                        cell.height * f32::from(cursor.y),
                    );
                let cursor_bounds = match cursor.style {
                    1 => Bounds::new(origin, cell),
                    2 => Bounds::new(
                        origin + point(px(0.), cell.height - px(1.)),
                        size(cell.width, px(1.)),
                    ),
                    _ => Bounds::new(origin, size(px(1.), cell.height)),
                };
                if cursor.style == 3 {
                    window.paint_quad(quad(
                        Bounds::new(origin, cell),
                        px(0.),
                        transparent_black(),
                        px(1.),
                        cx.theme().foreground,
                        BorderStyle::Solid,
                    ));
                } else {
                    window.paint_quad(fill(
                        cursor_bounds,
                        cx.theme()
                            .foreground
                            .opacity(if cursor.style == 1 { 0.5 } else { 1. }),
                    ));
                }
            }
            if let Some(preedit) = &self.composition
                && let (Some(document), Some(input)) =
                    (self.display_document(), self.input_region())
                && let Some((text, region)) = crate::ui::widgets::text_input::layout(
                    &document,
                    input.area,
                    ratatui::style::Style::default(),
                )
            {
                let input_bounds = Bounds::new(
                    bounds.origin
                        + point(
                            cell.width * f32::from(region.area.x),
                            cell.height * f32::from(region.area.y),
                        ),
                    size(cell.width * f32::from(region.area.width), cell.height),
                );
                let background = frame
                    .frame
                    .cells
                    .get(region.area.y as usize)
                    .and_then(|row| row.get(region.area.x as usize))
                    .map_or(cx.theme().background, |cell| rgb_color(cell.background));
                let mut value = String::new();
                let mut runs = Vec::new();
                let marked = preedit.replacement.start
                    ..preedit.replacement.start + preedit.text.encode_utf16().count();
                for (span, pair) in text.spans.iter().zip(region.carets.windows(2)) {
                    value.push_str(&span.content);
                    runs.push(TextRun {
                        len: span.content.len(),
                        font: font("Menlo"),
                        color: cx.theme().foreground,
                        underline: (pair[0].0 < marked.end && pair[1].0 > marked.start).then_some(
                            UnderlineStyle {
                                thickness: px(1.),
                                color: None,
                                wavy: false,
                            },
                        ),
                        ..Default::default()
                    });
                }
                let line = window
                    .text_system()
                    .shape_line(value.into(), px(14.), &runs, None);
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: input_bounds,
                    }),
                    |window| {
                        window.paint_layer(input_bounds, |window| {
                            window.paint_quad(fill(input_bounds, background));
                            let _ = line.paint(
                                input_bounds.origin,
                                cell.height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            let caret = input_bounds.origin
                                + point(
                                    cell.width
                                        * f32::from(
                                            region
                                                .column(document.selection.end)
                                                .saturating_sub(region.area.x),
                                        ),
                                    px(0.),
                                );
                            window.paint_quad(fill(
                                Bounds::new(caret, size(px(1.), cell.height)),
                                cx.theme().foreground,
                            ));
                        });
                    },
                );
            }
        });
    }
    fn paint_images(
        &mut self,
        layer: u8,
        bounds: Bounds<Pixels>,
        cell: Size<Pixels>,
        scale: f32,
        frame: &PresentedFrame,
        window: &mut Window,
    ) {
        for image in frame
            .frame
            .images
            .iter()
            .filter(|image| image.layer == layer)
        {
            let key = (image.id, image.generation);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.images.entry(key) {
                let Some(mut pixels) = image::RgbaImage::from_raw(
                    image.pixels.width,
                    image.pixels.height,
                    image.pixels.rgba.clone(),
                ) else {
                    continue;
                };
                // GPUI textures use BGRA; terminal image storage uses standard RGBA.
                for pixel in pixels.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                entry.insert(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])));
            }
            let Some(texture) = self.images.get(&key) else {
                continue;
            };
            let destination = Bounds::new(
                bounds.origin
                    + point(
                        cell.width * image.column as f32 + px(image.offset[0] as f32 / scale),
                        cell.height * image.row as f32 + px(image.offset[1] as f32 / scale),
                    ),
                size(
                    px(image.size[0] as f32 / scale),
                    px(image.size[1] as f32 / scale),
                ),
            );
            let [sx, sy, sw, sh] = image.source;
            let width = sw.max(1) as f32;
            let height = sh.max(1) as f32;
            let full = Bounds::new(
                destination.origin
                    - point(
                        destination.size.width * (sx as f32 / width),
                        destination.size.height * (sy as f32 / height),
                    ),
                size(
                    destination.size.width * (image.pixels.width as f32 / width),
                    destination.size.height * (image.pixels.height as f32 / height),
                ),
            );
            let _ = window.paint_image(
                destination,
                full,
                Corners::default(),
                texture.clone(),
                0,
                false,
            );
        }
    }
}
impl Focusable for TextSurface {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for TextSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let prepare_owner = owner.clone();
        let paint_owner = owner.clone();
        div()
            .id("embedded-text")
            .debug_selector(|| "embedded-text".into())
            .key_context("TextampText")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.modifiers = event.keystroke.modifiers;
                this.repeat = event.is_held;
                let modifiers = event.keystroke.modifiers;
                if let Some(code) = key_code(&event.keystroke.key, modifiers) {
                    let key = KeyEvent::new(code, key_modifiers(modifiers));
                    if let Some(command) = crate::app::shortcuts::navigation(key) {
                        let _ = this.commands.try_send(Command::Palette(command));
                        window.prevent_default();
                        cx.stop_propagation();
                        return;
                    }
                }
                if (modifiers.control
                    && ((modifiers.shift && matches!(event.keystroke.key.as_str(), "t" | "p"))
                        || (modifiers.alt && event.keystroke.key == "t")))
                    || (modifiers.platform
                        && modifiers.alt
                        && !modifiers.control
                        && !modifiers.shift
                        && event.keystroke.key == "t")
                    || event.prefer_character_input
                    || (this.editor.is_some()
                        && modifiers.alt
                        && !modifiers.control
                        && !modifiers.platform
                        && event.keystroke.key.chars().count() == 1)
                {
                    return;
                }
                if let Some(code) = key_code(&event.keystroke.key, event.keystroke.modifiers) {
                    this.send(Input::Key(KeyEvent::new_with_kind(
                        code,
                        key_modifiers(event.keystroke.modifiers),
                        if event.is_held {
                            KeyEventKind::Repeat
                        } else {
                            KeyEventKind::Press
                        },
                    )));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .on_modifiers_changed(cx.listener(|this, _, window, _| {
                let modifiers = window.modifiers();
                if this.modifiers.shift != modifiers.shift {
                    this.send(Input::Key(KeyEvent::new_with_kind(
                        KeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftShift),
                        key_modifiers(modifiers),
                        if modifiers.shift {
                            KeyEventKind::Press
                        } else {
                            KeyEventKind::Release
                        },
                    )));
                }
                this.modifiers = modifiers;
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if this.composition.is_some() {
                        this.unmark_text(window, cx);
                    }
                    this.focus.focus(window, cx);
                    if event.modifiers.platform {
                        let cell = this.cell_at(event.position);
                        this.selection = Some((cell, cell));
                        this.selecting = true;
                        cx.notify();
                    } else if this.frame.as_ref().is_some_and(|frame| {
                        let cell = this.cell_at(event.position);
                        frame.mini_artwork.iter().any(|r| r.contains(cell.into()))
                    }) {
                        window.dispatch_action(Box::new(super::EnterMiniPlayer), cx);
                    } else if let Some((offset, anchor)) = this.input_region().and_then(|region| {
                        let cell = this.cell_at(event.position);
                        region.area.contains((cell.0, cell.1).into()).then(|| {
                            let offset = region.offset(cell.0);
                            let anchor = if event.modifiers.shift {
                                this.editor
                                    .as_ref()
                                    .map_or(offset, |editor| editor.selection.start)
                            } else {
                                offset
                            };
                            (offset, anchor)
                        })
                    }) {
                        this.selection = None;
                        this.edit_drag = Some(anchor);
                        this.select_input(anchor, offset);
                        cx.notify();
                    } else {
                        this.selection = None;
                        this.mouse(
                            MouseEventKind::Down(crossterm::event::MouseButton::Left),
                            event.position,
                            event.modifiers,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.mouse(
                        MouseEventKind::Down(crossterm::event::MouseButton::Right),
                        event.position,
                        event.modifiers,
                    );
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    if this.selecting {
                        this.selecting = false;
                    } else if this.edit_drag.take().is_some() {
                        cx.notify();
                    } else {
                        this.mouse(
                            MouseEventKind::Up(crossterm::event::MouseButton::Left),
                            event.position,
                            event.modifiers,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    let cell = this.cell_at(event.position);
                    if let Some((_, end)) = &mut this.selection {
                        *end = cell;
                    }
                    cx.notify();
                } else if let Some(anchor) = this.edit_drag {
                    if let Some(region) = this.input_region() {
                        let offset = region.offset(this.cell_at(event.position).0);
                        this.select_input(anchor, offset);
                        cx.notify();
                    }
                } else {
                    this.mouse(
                        if event.pressed_button == Some(MouseButton::Left) {
                            MouseEventKind::Drag(crossterm::event::MouseButton::Left)
                        } else {
                            MouseEventKind::Moved
                        },
                        event.position,
                        event.modifiers,
                    );
                }
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                let delta = event.delta.pixel_delta(this.cell.height);
                let horizontal = f32::from(delta.x).abs() > f32::from(delta.y).abs();
                let amount = if horizontal {
                    f32::from(delta.x)
                } else {
                    f32::from(delta.y)
                };
                if amount != 0. {
                    let kind = match (horizontal, amount > 0.) {
                        (true, true) => MouseEventKind::ScrollLeft,
                        (true, false) => MouseEventKind::ScrollRight,
                        (false, true) => MouseEventKind::ScrollUp,
                        (false, false) => MouseEventKind::ScrollDown,
                    };
                    for _ in 0..((amount.abs() / f32::from(this.cell.height)).ceil() as usize)
                        .clamp(1, 12)
                    {
                        this.mouse(kind, event.position, event.modifiers);
                    }
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &super::Palette, _, cx| {
                this.send(Input::Key(KeyEvent::new(
                    KeyCode::Char('p'),
                    KeyModifiers::CONTROL,
                )));
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &super::Settings, _, cx| {
                this.send(Input::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &super::Library, _, cx| {
                this.send(Input::Key(KeyEvent::new(
                    KeyCode::Char('l'),
                    KeyModifiers::CONTROL,
                )));
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &super::NowPlaying, _, cx| {
                this.send(Input::Key(KeyEvent::new(
                    KeyCode::Char('n'),
                    KeyModifiers::CONTROL,
                )));
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                this.copy(cx);
                if let Some(editor) = &this.editor
                    && !editor.secret
                {
                    this.edit(crate::app::terminal_text::Edit::Replace {
                        target: editor.target.clone(),
                        range: Some(editor.selection.clone()),
                        text: String::new(),
                    });
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    this.send(Input::Paste(text));
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                if let Some(editor) = this.editor.as_mut() {
                    editor.selection = 0..editor.text.encode_utf16().count();
                    let edit = crate::app::terminal_text::Edit::Select {
                        target: editor.target.clone(),
                        range: editor.selection.clone(),
                    };
                    this.edit(edit);
                } else if let Some(frame) = &this.frame {
                    this.selection = Some((
                        (0, 0),
                        (
                            frame.frame.columns.saturating_sub(1),
                            frame.frame.rows.saturating_sub(1),
                        ),
                    ));
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        prepare_owner.update(cx, |this, cx| {
                            this.prepare(bounds, window, cx);
                            this.shape(bounds, window)
                        })
                    },
                    move |bounds, spans, window, cx| {
                        let focus = paint_owner.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_owner.clone()),
                            cx,
                        );
                        paint_owner.update(cx, |this, cx| this.paint(bounds, &spans, window, cx));
                    },
                )
                .size_full(),
            )
    }
}
fn rgb_color([r, g, b]: [u8; 3]) -> Hsla {
    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
}
fn key_modifiers(modifiers: Modifiers) -> KeyModifiers {
    let mut result = KeyModifiers::NONE;
    if modifiers.shift {
        result |= KeyModifiers::SHIFT;
    }
    if modifiers.alt {
        result |= KeyModifiers::ALT;
    }
    if modifiers.control {
        result |= KeyModifiers::CONTROL;
    }
    if modifiers.platform {
        result |= KeyModifiers::SUPER;
    }
    result
}
fn key_code(key: &str, modifiers: Modifiers) -> Option<KeyCode> {
    Some(match key {
        "enter" | "return" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "tab" if modifiers.shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        _ if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => {
            KeyCode::F(key[1..].parse().ok()?)
        }
        _ if (modifiers.control || modifiers.alt) && key.len() == 1 => {
            KeyCode::Char(key.chars().next()?)
        }
        _ => return None,
    })
}
impl EntityInputHandler for TextSurface {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let units = self.input_document()?.encode_utf16().collect::<Vec<_>>();
        let start = range.start.min(units.len());
        let end = range.end.min(units.len()).max(start);
        *adjusted = Some(start..end);
        Some(String::from_utf16_lossy(&units[start..end]))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: if let Some(preedit) = &self.composition {
                preedit.replacement.start + preedit.selection.start
                    ..preedit.replacement.start + preedit.selection.end
            } else {
                self.editor.as_ref()?.selection.clone()
            },
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.composition.as_ref().map(|preedit| {
            preedit.replacement.start
                ..preedit.replacement.start + preedit.text.encode_utf16().count()
        })
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(preedit) = self.composition.take() {
            self.edit(crate::app::terminal_text::Edit::Replace {
                target: preedit.target,
                range: Some(preedit.replacement),
                text: preedit.text,
            });
        }
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preedit = self.composition.take();
        if let Some(editor) = &self.editor {
            let (target, replacement) = preedit.map_or_else(
                || (editor.target.clone(), range),
                |preedit| (preedit.target, Some(preedit.replacement)),
            );
            self.edit(crate::app::terminal_text::Edit::Replace {
                target,
                range: replacement,
                text: text.into(),
            });
        } else if text.chars().count() == 1 {
            if let Some(character) = text.chars().next() {
                let mut modifiers = key_modifiers(window.modifiers());
                if !character.is_ascii() {
                    modifiers.remove(KeyModifiers::ALT);
                }
                self.send(Input::Key(KeyEvent::new_with_kind(
                    KeyCode::Char(character),
                    modifiers,
                    if self.repeat {
                        KeyEventKind::Repeat
                    } else {
                        KeyEventKind::Press
                    },
                )));
            }
        } else {
            self.send(Input::Text(text.into()));
        }
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = &self.editor else {
            return;
        };
        let replacement = self
            .composition
            .as_ref()
            .filter(|preedit| preedit.target == editor.target)
            .map(|preedit| preedit.replacement.clone())
            .unwrap_or_else(|| range.unwrap_or_else(|| editor.selection.clone()));
        let length = text.encode_utf16().count();
        self.composition = Some(Preedit {
            target: editor.target.clone(),
            replacement,
            text: text.into(),
            selection: selection.unwrap_or(length..length),
        });
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if let Some(editor) = self.editor.as_mut() {
            editor.selection = range.clone();
            let target = editor.target.clone();
            self.edit(crate::app::terminal_text::Edit::Select { target, range });
        }
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.input_document()?.encode_utf16().count())
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cell = self.frame_cell();
        if let Some(region) = self.input_region() {
            let start = self.input_column(range.start)?;
            let end = self.input_column(range.end)?;
            return Some(Bounds::new(
                self.bounds.origin
                    + point(
                        cell.width * f32::from(start),
                        cell.height * f32::from(region.area.y),
                    ),
                size(
                    cell.width * f32::from(end.saturating_sub(start).max(1)),
                    cell.height,
                ),
            ));
        }
        let cursor = self.frame.as_ref()?.frame.cursor;
        Some(Bounds::new(
            self.bounds.origin
                + point(
                    cell.width * f32::from(cursor.x),
                    cell.height * f32::from(cursor.y),
                ),
            cell,
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        if let Some(region) = self.display_region() {
            return Some(region.offset(self.cell_at(position).0));
        }
        self.composition
            .as_ref()
            .map(|preedit| preedit.replacement.start + preedit.selection.end)
            .or_else(|| self.editor.as_ref().map(|editor| editor.selection.end))
    }
}

#[cfg(all(test, feature = "desktop-tests"))]
mod tests {
    use super::{Appearance, Arc, Identity, Metrics, PresentedFrame, TextSurface, Viewport};
    use gpui_test_support::{Bounds, TestAppContext, point, px, size};
    #[gpui_test_support::test]
    fn mode_cycle_shortcut_reaches_the_application_in_embedded_text(cx: &mut TestAppContext) {
        let count = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = count.clone();
        cx.update(|cx| {
            super::super::init(cx);
            super::super::keyboard::bind_navigation_shortcuts(cx);
            cx.on_action(move |_: &super::super::CycleApplicationMode, _| {
                observed.set(observed.get() + 1)
            });
        });
        let (engine, _updates, mut commands) =
            super::super::engine::Engine::fixture(Default::default());
        let port = engine.commands.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = TextSurface::new(port, Identity(1), window, cx);
            view.focus.focus(window, cx);
            view
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for (index, binding) in ["cmd-alt-t", "ctrl-alt-t", "cmd-shift-t", "ctrl-shift-t"]
            .into_iter()
            .enumerate()
        {
            cx.simulate_keystrokes(binding);
            assert_eq!(count.get(), index + 1, "{binding}");
        }
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, super::super::engine::Command::TextInput { .. }))
        );
    }
    #[gpui_test_support::test]
    fn queue_shortcuts_dispatch_the_same_command_in_embedded_text(cx: &mut TestAppContext) {
        use super::super::{NativeCommand, engine::Command};
        use crate::app::state::PaletteCommandKind;
        let (engine, _, mut commands) = super::super::engine::Engine::fixture(Default::default());
        let native = engine.commands.clone();
        cx.update(|cx| {
            super::super::init(cx);
            super::super::keyboard::bind_navigation_shortcuts(cx);
            cx.on_action(move |action: &NativeCommand, _| {
                let _ = native.try_send(Command::Palette(action.command()));
            });
        });
        let port = engine.commands.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = TextSurface::new(port, Identity(1), window, cx);
            view.focus.focus(window, cx);
            view
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for binding in ["cmd-2", "ctrl-2", "cmd-u", "ctrl-u"] {
            cx.simulate_keystrokes(binding);
            let pending: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert_eq!(
                pending
                    .iter()
                    .filter(|command| matches!(
                        command,
                        Command::Palette(PaletteCommandKind::GotoQueue)
                    ))
                    .count(),
                1,
                "{binding}"
            );
            assert!(
                !pending
                    .iter()
                    .any(|command| matches!(command, Command::TextInput { .. })),
                "{binding}"
            );
        }
    }
    #[gpui_test_support::test]
    fn embedded_artwork_opens_the_shared_mini_player(cx: &mut TestAppContext) {
        use super::super::{Desktop, WindowRegistry, WindowRole};
        use gpui_kit::{AppContext, Context, Entity};
        cx.update(super::super::init);
        let mut initial = super::super::engine::Snapshot::default();
        initial.presentation = crate::app::windowed_presentation::Presentation::windowed();
        let preparation = initial
            .presentation
            .request(crate::app::launch::WindowedMode::Text)
            .unwrap();
        initial.presentation.commit(preparation);
        let (engine, _updates, _commands) = super::super::engine::Engine::fixture(initial.clone());
        let (host, cx) = cx.add_window_view(move |window, cx: &mut Context<Desktop>| {
            let mut desktop = Desktop::new(
                Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            let port = desktop.engine.commands.clone();
            let surface = cx.new(|cx| TextSurface::new(port, preparation.identity, window, cx));
            desktop.text_surface = Some((preparation.identity, surface));
            desktop.snapshot = initial;
            desktop
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let artwork = cx.update(|window, cx| {
            host.update(cx, |host, cx| {
                let surface: Entity<TextSurface> = host.text_surface.as_ref().unwrap().1.clone();
                surface.update(cx, |view, cx| {
                    let viewport = view.sent.expect("terminal viewport");
                    let mut vt = crate::text_mode::vt::Vt::new(
                        viewport.metrics.columns,
                        viewport.metrics.rows,
                    )
                    .unwrap();
                    view.apply(
                        Arc::new(PresentedFrame {
                            identity: view.identity,
                            serial: 1,
                            viewport,
                            frame: vt.frame().unwrap(),
                            breadcrumbs: vec![],
                            editor: None,
                            mini_artwork: vec![ratatui::layout::Rect::new(2, 2, 3, 2)],
                            sidebar_symbols: vec![],
                            graphical_visualizer: None,
                            transport_controls: vec![],
                            muted: false,
                            playing: false,
                            position_ms: 0,
                            duration_ms: 0,
                            input_region: None,
                        }),
                        window,
                        cx,
                    );
                    view.bounds.origin + point(view.cell.width * 3., view.cell.height * 3.)
                })
            })
        });
        cx.simulate_click(artwork, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let cover = cx
            .debug_bounds("mini-artwork-return")
            .expect("shared mini player");
        cx.simulate_click(cover.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("mini-artwork-return").is_none());
        assert!(host.read_with(cx, |host, _| host.text_surface.is_some()
            && host.mini_return_size.is_none()));
    }
    #[gpui_test_support::test]
    fn wide_graphemes_reserve_two_cells_without_shifting_the_following_text(
        cx: &mut TestAppContext,
    ) {
        cx.update(super::super::init);
        let (engine, _, _) = super::super::engine::Engine::fixture(Default::default());
        let commands = engine.commands.clone();
        let mut vt = crate::text_mode::vt::Vt::new(20, 2).unwrap();
        vt.write("a界e\u{301}b".as_bytes());
        let frame = Arc::new(PresentedFrame {
            identity: Identity(1),
            serial: 1,
            viewport: Viewport {
                metrics: Metrics {
                    columns: 20,
                    rows: 2,
                    cell_width: 18,
                    cell_height: 40,
                },
                scale_factor: 2.,
                revision: 1,
                geometry: Default::default(),
                appearance: Appearance {
                    system: None,
                    foreground: [255; 3],
                    background: [0; 3],
                },
            },
            frame: vt.frame().unwrap(),
            breadcrumbs: Vec::new(),
            editor: None,
            mini_artwork: vec![],
            sidebar_symbols: Vec::new(),
            graphical_visualizer: None,
            transport_controls: Vec::new(),
            muted: false,
            playing: false,
            position_ms: 0,
            duration_ms: 0,
            input_region: None,
        });
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let mut view = TextSurface::new(commands, Identity(1), window, cx);
            view.apply(frame, window, cx);
            view
        });
        let spans = cx.update(|window, cx| {
            view.update(cx, |view, _| {
                view.shape(
                    Bounds::new(point(px(0.), px(0.)), size(px(180.), px(40.))),
                    window,
                )
            })
        });
        let wide = spans
            .iter()
            .find(|span| span.line.text.as_ref() == "界")
            .unwrap();
        assert_eq!(wide.origin.x, px(9.));
        let following = spans
            .iter()
            .find(|span| span.line.text.starts_with("e\u{301}b"))
            .unwrap();
        assert_eq!(following.origin.x, px(27.));
    }
    #[gpui_test_support::test]
    fn native_input_ranges_and_marked_text_use_the_visible_field(cx: &mut TestAppContext) {
        use crate::app::terminal_text::{Document, Target};
        use gpui_kit::EntityInputHandler;
        cx.update(super::super::init);
        let (engine, _, _) = super::super::engine::Engine::fixture(Default::default());
        let mut vt = crate::text_mode::vt::Vt::new(20, 2).unwrap();
        let document = Document {
            target: Target::Palette,
            text: "a界e\u{301}x".into(),
            selection: 0..0,
            secret: false,
        };
        let (_, region) = crate::ui::widgets::text_input::layout(
            &document,
            ratatui::layout::Rect::new(2, 0, 12, 1),
            Default::default(),
        )
        .unwrap();
        let frame = Arc::new(PresentedFrame {
            identity: Identity(1),
            serial: 1,
            viewport: Viewport {
                metrics: Metrics {
                    columns: 20,
                    rows: 2,
                    cell_width: 18,
                    cell_height: 40,
                },
                scale_factor: 2.,
                revision: 1,
                geometry: Default::default(),
                appearance: Appearance {
                    system: None,
                    foreground: [255; 3],
                    background: [0; 3],
                },
            },
            frame: vt.frame().unwrap(),
            breadcrumbs: Vec::new(),
            editor: Some(document),
            mini_artwork: vec![],
            sidebar_symbols: Vec::new(),
            graphical_visualizer: None,
            transport_controls: Vec::new(),
            muted: false,
            playing: false,
            position_ms: 0,
            duration_ms: 0,
            input_region: Some(region),
        });
        let commands = engine.commands.clone();
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let mut view = TextSurface::new(commands, Identity(1), window, cx);
            view.apply(frame, window, cx);
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let bounds = view
                    .bounds_for_range(1..2, Bounds::default(), window, cx)
                    .unwrap();
                assert_eq!(bounds.origin.x, px(27.));
                assert_eq!(bounds.size.width, px(18.));
                assert_eq!(
                    view.character_index_for_point(point(px(45.), px(0.)), window, cx),
                    Some(2)
                );
                view.replace_and_mark_text_in_range(Some(1..2), "かな", Some(1..1), window, cx);
                assert_eq!(view.input_document().as_deref(), Some("aかなe\u{301}x"));
                assert_eq!(
                    view.bounds_for_range(2..2, Bounds::default(), window, cx)
                        .unwrap()
                        .origin
                        .x,
                    px(45.)
                );
                assert_eq!(
                    view.character_index_for_point(point(px(45.), px(0.)), window, cx),
                    Some(2)
                );
                let editor = view.editor.as_mut().unwrap();
                editor.secret = true;
                editor.text = "•••••".into();
                view.replace_and_mark_text_in_range(Some(1..2), "private", Some(7..7), window, cx);
                assert!(
                    view.input_document()
                        .unwrap()
                        .chars()
                        .all(|value| value == '•')
                );
            })
        });
    }
}
