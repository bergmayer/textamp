//! Textamp's terminal artwork as decorative content in the macOS About pane.
use super::*;

/// Render the same themed ANSI cell artwork used by the terminal frontend.
pub(super) fn themed_artwork(cx: &App) -> AnyElement {
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        widgets::{Paragraph, Widget},
    };
    let colors =
        super::theme::terminal_palette(cx).map(|[r, g, b]| ratatui::style::Color::Rgb(r, g, b));
    let lines = crate::ui::widgets::logo::themed(&colors);
    let width = lines
        .iter()
        .map(ratatui::text::Line::width)
        .max()
        .unwrap_or(1) as u16;
    let height = lines.len().max(1) as u16;
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    Paragraph::new(lines)
        .style(
            ratatui::style::Style::new()
                .fg(colors.fg_accent)
                .bg(colors.bg_primary),
        )
        .render(buffer.area, &mut buffer);
    let source = std::sync::Arc::new(buffer);
    let prepare = source.clone();
    let color = |value| match value {
        ratatui::style::Color::Rgb(r, g, b) => {
            gpui_kit::rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
        }
        ratatui::style::Color::White => gpui_kit::white(),
        _ => gpui_kit::black(),
    };
    div()
        .id("now-playing-ansi-logo")
        .debug_selector(|| "now-playing-ansi-logo".into())
        .role(Role::Image)
        .aria_label("Textamp ANSI logo")
        .w_full()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(
            canvas(
                move |bounds, window, _| {
                    let unit = (bounds.size.width / f32::from(width))
                        .min(bounds.size.height / (f32::from(height) * 2.));
                    let cell = size(unit, unit * 2.);
                    let origin = bounds.origin
                        + point(
                            (bounds.size.width - unit * f32::from(width)) / 2.,
                            (bounds.size.height - cell.height * f32::from(height)) / 2.,
                        );
                    let mut glyphs = Vec::new();
                    for y in 0..height {
                        for x in 0..width {
                            let value = &prepare[(x, y)];
                            if value.symbol().trim().is_empty()
                                || super::text_boxdrawing::supports(value.symbol())
                            {
                                continue;
                            }
                            glyphs.push((
                                origin + point(unit * f32::from(x), cell.height * f32::from(y)),
                                window.text_system().shape_line(
                                    value.symbol().to_owned().into(),
                                    unit * 1.65,
                                    &[TextRun {
                                        len: value.symbol().len(),
                                        font: font("Menlo"),
                                        color: color(value.fg),
                                        ..Default::default()
                                    }],
                                    Some(unit),
                                ),
                            ));
                        }
                    }
                    (origin, cell, glyphs)
                },
                move |bounds, (origin, cell, glyphs), window, cx| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        window.paint_layer(bounds, |window| {
                            for y in 0..height {
                                for x in 0..width {
                                    let value = &source[(x, y)];
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            origin
                                                + point(
                                                    cell.width * f32::from(x),
                                                    cell.height * f32::from(y),
                                                ),
                                            cell,
                                        ),
                                        color(value.bg),
                                    ));
                                }
                            }
                        });
                        window.paint_layer(bounds, |window| {
                            for y in 0..height {
                                for x in 0..width {
                                    let value = &source[(x, y)];
                                    super::text_boxdrawing::paint(
                                        value.symbol(),
                                        Bounds::new(
                                            origin
                                                + point(
                                                    cell.width * f32::from(x),
                                                    cell.height * f32::from(y),
                                                ),
                                            cell,
                                        ),
                                        color(value.fg),
                                        window,
                                    );
                                }
                            }
                            for (origin, line) in glyphs {
                                let _ = line.paint(
                                    origin,
                                    cell.height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        });
                    });
                },
            )
            .size_full(),
        )
        .into_any_element()
}

pub fn content(cx: &App) -> AnyElement {
    let rows: Vec<Vec<char>> = include_str!("about-logo.txt")
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .map(|line| line.chars().collect())
        .collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1) as f32;
    let height = rows.len() as f32;
    let artwork = div()
        .id("about-ansi-logo")
        .debug_selector(|| "about-ansi-logo".into())
        .role(Role::Image)
        .aria_label("Textamp ANSI logo")
        .w_full()
        .max_w(px(480.))
        .aspect_ratio(columns / (height * 1.45))
        .flex_none()
        .child(
            canvas(
                move |_, _, _| (),
                move |bounds, _, window, _| {
                    let cell = bounds.size.width / columns;
                    let cell_height = bounds.size.height / height;
                    for (y, row) in rows.iter().enumerate() {
                        let color = rgb(match y {
                            0..=4 => 0x19c7ce,
                            5..=8 => 0x39a8de,
                            _ => 0x7169ed,
                        });
                        for (x, c) in row.iter().enumerate() {
                            if *c == '█' {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        bounds.origin
                                            + point(cell * x as f32, cell_height * y as f32),
                                        size(cell, cell_height),
                                    ),
                                    color,
                                ));
                            }
                        }
                    }
                },
            )
            .size_full(),
        );
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .min_w_0()
        .items_start()
        .child(artwork)
        .child(
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Textamp"),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
        )
        .into_any_element()
}
