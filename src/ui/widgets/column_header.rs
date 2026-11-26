//! Compact terminal column titles, filter fields and command buttons.
use crate::app::column_controls::{self, ColumnControl};
use crate::ui::{RenderState, theme::theme};
use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};

/// Whether terminal content reserves a row for explicit detail or playback actions.
pub fn has_content_row(column: &crate::app::state::BrowseColumn) -> bool {
    column.has_album_details() || (column.details.is_some() && column.play_all_row.is_some())
}
/// Paint controls over the column's top border and register their exact targets.
pub fn render(frame: &mut Frame, state: &RenderState<'_>) {
    let regions = state.hit_regions.borrow().miller_columns.clone();
    let Some(regions) = regions else {
        return;
    };
    let colors = &theme().colors;
    for region in regions.columns {
        if region.area.width < 6 {
            continue;
        }
        let controls = column_controls::controls(state, region.col_idx);
        let album = state
            .browse_nav()
            .and_then(|nav| nav.columns.get(region.col_idx))
            .is_some_and(|column| column.has_album_details());
        let artist_page = state
            .browse_nav()
            .and_then(|nav| nav.columns.get(region.col_idx))
            .is_some_and(|c| c.artist_header.is_some());
        let in_content = |control| match control {
            ColumnControl::ArtistBio | ColumnControl::Details | ColumnControl::Play => true,
            ColumnControl::Next | ColumnControl::Add => !album,
            _ => false,
        };
        let buttons = controls
            .iter()
            .filter(|control| **control != ColumnControl::Filter && !in_content(**control))
            .count() as u16;
        let area = Rect::new(region.area.x + 1, region.area.y, region.area.width - 2, 1);
        let title_width = area.width.saturating_sub(buttons * 2).max(1);
        frame.render_widget(
            Paragraph::new(" ".repeat(area.width as usize))
                .style(Style::default().bg(colors.bg_primary)),
            area,
        );
        let title_area = Rect::new(area.x, area.y, title_width, 1);
        let focused = state
            .tui_column_header
            .filter(|focus| {
                focus.category == state.browse_category && focus.column == region.col_idx
            })
            .or_else(|| {
                (!state.category_column_focused
                    && state.tui_transport_focus.is_none()
                    && state.tui_footer_focus.is_none()
                    && state.tui_artist_focus.is_none()
                    && !state.tui_description_focused
                    && !state.tui_description_submit_focused
                    && !state.track_pane_focused
                    && state.browse_nav().is_some_and(|nav| {
                        nav.focused_column == region.col_idx
                            && nav.focused().is_some_and(|column| {
                                column.details.is_some()
                                    && column.header_selection
                                        == crate::app::state::ColumnHeaderSelection::Play
                            })
                    }))
                .then_some(crate::app::column_controls::ColumnHeaderFocus {
                    category: state.browse_category,
                    column: region.col_idx,
                    control: ColumnControl::Play,
                })
            });
        let style = |control| {
            if focused.is_some_and(|focus| focus.control == control) {
                colors.focused_row()
            } else {
                Style::default()
                    .fg(colors.fg_secondary)
                    .bg(colors.bg_primary)
            }
        };
        let filtering = state.list_filter.active
            && state.list_filter.category == state.browse_category
            && state.list_filter.column == region.col_idx;
        let title = if filtering {
            let width = title_width.saturating_sub(5) as usize;
            let query: String = state
                .list_filter
                .query
                .chars()
                .rev()
                .take(width)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!(" / {query}")
        } else {
            let title = state
                .browse_nav()
                .and_then(|nav| nav.columns.get(region.col_idx))
                .map(|column| {
                    column
                        .artist_header
                        .as_ref()
                        .map_or(column.title.as_str(), |i| i.artist_name.as_str())
                })
                .or_else(|| {
                    state
                        .folder_state
                        .as_ref()
                        .and_then(|folders| folders.columns.get(region.col_idx))
                        .map(|column| column.title.as_str())
                })
                .unwrap_or("Library");
            let width = title_width.saturating_sub(2) as usize;
            let display = display_title(title, width);
            let now = std::time::Instant::now();
            let elapsed = if unicode_width::UnicodeWidthStr::width(display.as_str()) > width {
                let start = state
                    .app
                    .hit_regions
                    .header_clocks
                    .iter()
                    .find(|(column, text, cells, _)| {
                        *column == region.col_idx && text == &display && *cells == width
                    })
                    .map_or(now, |clock| clock.3);
                state.hit_regions.borrow_mut().header_clocks.push((
                    region.col_idx,
                    display.clone(),
                    width,
                    start,
                ));
                now.duration_since(start)
            } else {
                std::time::Duration::ZERO
            };
            format!(
                " {}",
                crate::util::column_heading::cells(&display, width, elapsed)
            )
        };
        let mut title_style = style(ColumnControl::Filter);
        if artist_page && !filtering {
            title_style = title_style.add_modifier(ratatui::style::Modifier::BOLD);
        }
        frame.render_widget(Paragraph::new(title).style(title_style), title_area);
        if filtering {
            super::text_input::render(
                frame,
                state,
                Rect::new(
                    title_area.x + 3,
                    title_area.y,
                    title_area.width.saturating_sub(5),
                    1,
                ),
                crate::app::terminal_text::Target::Filter {
                    category: state.browse_category,
                    column: region.col_idx,
                },
                style(ColumnControl::Filter),
            );
        }
        let mut targets = if controls.contains(&ColumnControl::Filter) {
            vec![(title_area, ColumnControl::Filter)]
        } else {
            Vec::new()
        };
        if filtering && title_width >= 4 {
            let close = Rect::new(title_area.right() - 2, area.y, 2, 1);
            frame.render_widget(
                Paragraph::new("× ").style(style(ColumnControl::Filter)),
                close,
            );
            // The close field is handled before the rest of the title in pointer input.
            targets.insert(0, (close, ColumnControl::Filter));
        }
        let content: Vec<_> = controls
            .iter()
            .copied()
            .filter(|control| in_content(*control))
            .map(|id| super::control_bar::Button {
                id,
                text: match id {
                    ColumnControl::ArtistBio
                        if state
                            .browse_nav()
                            .and_then(|nav| nav.columns.get(region.col_idx))
                            .is_some_and(|c| {
                                c.artist_header
                                    .as_ref()
                                    .is_some_and(|i| i.needs_web_search())
                                    || crate::app::details::album_bio(state, c)
                                        .is_some_and(|bio| bio.search_provider.is_some())
                            }) =>
                    {
                        state.web_search_provider.mark()
                    }
                    ColumnControl::ArtistBio if artist_page => "…more",
                    ColumnControl::ArtistBio => "Artist Bio",
                    ColumnControl::Details => "Album Info",
                    ColumnControl::Play => {
                        if state
                            .browse_nav()
                            .and_then(|nav| nav.columns.get(region.col_idx))
                            .is_some_and(|column| {
                                matches!(
                                    column.play_all_row,
                                    Some(crate::app::state::PlayAllRow::ArtistRadio { .. })
                                )
                            })
                        {
                            "◉ Artist Mix"
                        } else {
                            "▶ Play"
                        }
                    }
                    ColumnControl::Next => "▹ Play Next",
                    ColumnControl::Add => "+ Add to Queue",
                    _ => "",
                },
                enabled: true,
                active: false,
            })
            .collect();
        if !content.is_empty() && (!artist_page || region.inner.y > region.area.y + 1) {
            let row = if artist_page {
                Rect::new(
                    area.right().saturating_sub(8).max(area.x),
                    region.inner.y.saturating_sub(1).max(region.area.y + 1),
                    area.width.min(8),
                    1,
                )
            } else {
                Rect::new(area.x, region.area.y + 1, area.width, 1)
            }
            .intersection(region.area);
            targets.extend(super::control_bar::render(
                frame,
                row,
                &content,
                super::control_bar::Arrangement::Fill(content.len() as u16),
                focused.map(|focus| focus.control),
            ));
        }
        for (offset, control) in controls
            .into_iter()
            .filter(|control| *control != ColumnControl::Filter && !in_content(*control))
            .enumerate()
        {
            let rect = Rect::new(
                title_area.right() + offset as u16 * 2,
                area.y,
                2.min(
                    area.right()
                        .saturating_sub(title_area.right() + offset as u16 * 2),
                ),
                1,
            );
            let icon = match control {
                ColumnControl::ArtistBio | ColumnControl::Details => "",
                ColumnControl::Next => "▹ ",
                ColumnControl::Play => "▷ ",
                ColumnControl::Add => "+ ",
                ColumnControl::PlaylistEdit => {
                    if state
                        .browse_nav()
                        .and_then(|n| n.columns.get(region.col_idx))
                        .is_some_and(|c| c.playlist_editing)
                    {
                        "✓ "
                    } else {
                        "✎ "
                    }
                }
                ColumnControl::ArtistMix => "◉ ",
                ColumnControl::DownloadAlbum => "↓ ",
                ColumnControl::Options => "≡ ",
                ColumnControl::Close => "× ",
                ColumnControl::Filter => "",
            };
            frame.render_widget(Paragraph::new(icon).style(style(control)), rect);
            targets.push((rect, control));
        }
        if let Some(column) = state
            .hit_regions
            .borrow_mut()
            .miller_columns
            .as_mut()
            .and_then(|regions| {
                regions
                    .columns
                    .iter_mut()
                    .find(|column| column.col_idx == region.col_idx)
            })
        {
            column.view_options = targets
                .iter()
                .find(|(_, control)| *control == ColumnControl::Options)
                .map(|(rect, _)| *rect);
            column.close_x = targets
                .iter()
                .find(|(_, control)| *control == ColumnControl::Close)
                .map(|(rect, _)| *rect);
            column.controls = targets;
        }
    }
}

/// Preserve the destination before a generic column description when space is limited.
pub(crate) fn compact_title(title: &str, width: usize) -> String {
    crate::util::column_heading::cells(
        &display_title(title, width),
        width,
        std::time::Duration::ZERO,
    )
}

fn display_title(title: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let title = [
        ("artist information", "Artist Details"),
        ("artist info", "Artist Details"),
        ("album information", "Album Details"),
        ("album info", "Album Details"),
        ("track information", "Track Details"),
        ("track info", "Track Details"),
    ]
    .into_iter()
    .find(|(label, _)| {
        title
            .get(..label.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(label))
    })
    .map_or_else(
        || title.to_owned(),
        |(label, replacement)| format!("{replacement}{}", &title[label.len()..]),
    );
    let title = crate::util::column_heading::capitalized(&title);
    if UnicodeWidthStr::width(title.as_str()) <= width {
        return title;
    }
    crate::util::column_heading::destination(&title).into()
}

#[cfg(test)]
mod tests {
    use super::compact_title;

    #[test]
    fn column_heading_preserves_catalog_case_before_compaction() {
        assert_eq!(compact_title("album — DÉJÀ VU", 30), "Album — DÉJÀ VU");
        assert_eq!(compact_title("Album — DÉJÀ VU", 7), "DÉJÀ VU");
    }
}
