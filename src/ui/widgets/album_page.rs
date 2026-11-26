//! Artist and album headers and recommendations surrounding the terminal catalog list.
use crate::{
    app::{
        details,
        state::{BrowseColumn, PlayAllRow},
    },
    ui::{RenderState, artwork},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Wrap},
};

/// Return the track viewport so rendering, selection and hit testing use the same rows.
pub fn render_artist(
    frame: &mut Frame,
    state: &RenderState<'_>,
    index: usize,
    column: &BrowseColumn,
    area: Rect,
) -> Rect {
    let top = state
        .scroll
        .browse
        .filter(|(col, _)| *col == index)
        .map(|(_, pos)| pos);
    let inline_grid = crate::app::view_options::album_grid(state, index) && area.width >= 12;
    let top = top.or_else(|| {
        (inline_grid && column.selected_index > 0).then(|| {
            state
                .app
                .hit_regions
                .miller_offsets
                .iter()
                .find(|(col, _)| *col == index)
                .map_or(0, |(_, pos)| *pos)
        })
    });
    let past_header = top.map_or(
        column.selected_index >= usize::from(area.height.max(1)),
        |pos| pos > 0,
    ) || column.recommendation_focus.is_some();
    if let Some(info) = &column.artist_header {
        if area.height < 5 || area.width < 8 {
            return area;
        }
        if past_header {
            if inline_grid {
                return area;
            }
            return artist_list_recommendations(
                frame,
                state,
                index,
                column,
                area,
                &info.document.related_artists,
            );
        }
        let height = (area.height / 4).clamp(1, 9).min((area.width / 6).max(1));
        let width = (height * 2).min(area.width / 3);
        let portrait = Rect::new(area.x + (area.width - width) / 2, area.y, width, height);
        if !state
            .artwork
            .grid_cache
            .get(&info.artist_key)
            .is_some_and(|bytes| {
                artwork::render_grid_portrait(frame, portrait, &info.artist_key, bytes)
            })
        {
            artwork::render_placeholder(frame, portrait, artwork::Placeholder::Artist);
        }
        let bio_height = if info.excerpt().trim().is_empty() {
            0
        } else {
            (area.height.saturating_sub(height + 2)).min(2)
        };
        let text = Rect::new(area.x, portrait.bottom(), area.width, bio_height);
        frame.render_widget(
            Paragraph::new(info.excerpt()).wrap(Wrap { trim: false }),
            text,
        );
        let list = Rect::new(
            area.x,
            text.bottom() + 1,
            area.width,
            area.bottom() - text.bottom() - 1,
        );
        if inline_grid {
            return list;
        }
        return artist_list_recommendations(
            frame,
            state,
            index,
            column,
            list,
            &info.document.related_artists,
        );
    }
    area
}

pub(in crate::ui) struct AlbumViewport {
    pub tracks: Rect,
    pub scroll: crate::app::presentation::AlbumScroll,
    pub art_row: (u16, u16),
}

pub(in crate::ui) fn render_album(
    frame: &mut Frame,
    state: &RenderState<'_>,
    index: usize,
    column: &BrowseColumn,
    area: Rect,
    filter: Option<&crate::app::state::ListFilterResults>,
) -> Option<AlbumViewport> {
    let PlayAllRow::Album { rating_key, .. } = column.play_all_row.as_ref()? else {
        return None;
    };
    let page = details::album_page(state, column)?;
    let height = if area.width >= 4 {
        (area.width / 2)
            .min(area.height.saturating_sub(8) / 2)
            .min(12)
    } else {
        0
    };
    let text_height = area.height.saturating_sub(height + 8).min(4);
    let header_height = usize::from(height + text_height);
    let indices = filter.map(|f| f.matched_indices.as_slice());
    let count = indices.map_or(column.items.len(), <[usize]>::len);
    // An empty list still has a loading, empty, or no-matches row.
    let tracks = count.max(1);
    let art_row = crate::ui::app::compute_art_grid_row(area.width, area.height, state.cell_width());
    let track_height = if column.artwork_visible {
        art_row.0
    } else if state.browse_nav().is_some_and(|nav| {
        crate::app::presentation::is_two_row_browse_column(state, column, index, nav)
    }) {
        2
    } else {
        1
    };
    let cols = crate::app::view_options::terminal_grid_columns(area.width);
    let similar = page.similar.as_ref().filter(|_| filter.is_none());
    let albums = similar.map_or(&[][..], |s| s.tracks.as_slice());
    let message = similar.map_or("", |s| s.message.as_str());
    let artists = page
        .related_artists
        .as_deref()
        .filter(|_| filter.is_none())
        .unwrap_or_default();
    let status = albums.is_empty() && !message.is_empty();
    let album_bands = albums.len().div_ceil(cols) + usize::from(status);
    let artist_start = tracks + album_bands;
    let bands = artist_start + artists.len().div_ceil(cols);
    let tile_height = usize::from(
        area.height
            .saturating_sub(1)
            .clamp(1, crate::ui::layout::ALBUM_TILE_ROWS),
    );
    let album_height = if status { 1 } else { tile_height };
    let heading = usize::from(area.height > 1);
    let band_y = |band: usize| {
        usize::from(band > 0) * header_height
            + band.min(tracks) * usize::from(track_height)
            + band.saturating_sub(tracks).min(album_bands) * album_height
            + usize::from(band > tracks && album_bands > 0) * heading
            + band.saturating_sub(artist_start) * tile_height
            + usize::from(band > artist_start) * heading
    };
    let first_fitting =
        |end| crate::ui::layout::first_fitting_band(end, usize::from(area.height), band_y);
    let selected = match column.recommendation_focus {
        Some((0, i)) if i < albums.len() => tracks + i / cols,
        Some((1, i)) if i < artists.len() => artist_start + i / cols,
        _ => indices
            .map_or(column.selected_index, |v| {
                v.iter()
                    .position(|&i| i == column.selected_index)
                    .unwrap_or(0)
            })
            .min(tracks - 1),
    };
    let max_first = first_fitting(bands);
    let first = state
        .presentation_anchors
        .as_ref()
        .and_then(|anchors| {
            anchors.display_offset(
                state,
                index,
                column.items.iter().map(crate::app::state::BrowseItem::key),
                indices,
            )
        })
        .or_else(|| {
            state
                .scroll
                .browse
                .filter(|(col, _)| *col == index)
                .map(|(_, pos)| pos)
        })
        .unwrap_or_else(|| {
            let previous = state
                .app
                .hit_regions
                .miller_offsets
                .iter()
                .find(|(col, _)| *col == index)
                .map_or(0, |(_, pos)| *pos);
            previous.clamp(first_fitting(selected + 1), selected)
        })
        .min(max_first);
    let scroll = crate::app::presentation::AlbumScroll {
        first,
        max_first,
        bands,
    };
    let mut y = area.y;
    if first == 0 && header_height > 0 {
        let width = height * 2;
        let cover = Rect::new(area.x + (area.width - width) / 2, area.y, width, height);
        if height > 0
            && !state
                .artwork
                .grid_cache
                .get(rating_key)
                .is_some_and(|bytes| artwork::render_grid_image(frame, cover, rating_key, bytes))
        {
            artwork::render_placeholder(frame, cover, artwork::Placeholder::Album);
        }
        let text = Rect::new(area.x, cover.bottom(), area.width, text_height);
        let mut lines = Vec::new();
        if !page.subtitle.is_empty() {
            lines.push(Line::styled(
                format!("Artist: {}", page.subtitle),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        for pair in page.rows.chunks(2) {
            lines.push(Line::from(
                pair.iter()
                    .map(|(label, value)| format!("{label}: {value}"))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: true }),
            text,
        );
        y = text.bottom();
    }
    let track_rows = tracks
        .saturating_sub(first)
        .min(usize::from((area.bottom() - y) / track_height)) as u16;
    let track_area = Rect::new(area.x, y, area.width, track_rows * track_height);
    y = track_area.bottom();
    for band in first.max(tracks)..bands {
        if band_y(band).saturating_sub(band_y(first)) >= usize::from(area.height)
            || y >= area.bottom()
        {
            break;
        }
        let section = usize::from(band >= artist_start);
        let start = if section == 0 { tracks } else { artist_start };
        if band == start && heading != 0 {
            frame.render_widget(
                Paragraph::new(if section == 0 {
                    "Similar Albums"
                } else {
                    "Similar Artists"
                }),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
        }
        if y >= area.bottom() {
            break;
        }
        if section == 0 && status {
            frame.render_widget(Paragraph::new(message), Rect::new(area.x, y, area.width, 1));
            y += 1;
            continue;
        }
        let h = (tile_height as u16).min(area.bottom() - y);
        let size = (tile_height as u16)
            .saturating_sub(3)
            .min((area.width / cols as u16) / 2);
        if h <= size {
            break;
        }
        let width = area.width / cols as u16;
        let base = (band - start) * cols;
        let count = if section == 0 {
            albums.len()
        } else {
            artists.len()
        };
        for item in base..(base + cols).min(count) {
            let tile = Rect::new(area.x + (item - base) as u16 * width, y, width, h);
            let selected = column.recommendation_focus == Some((section, item))
                && !state.category_column_focused
                && !state.track_pane_focused
                && state
                    .browse_nav()
                    .is_some_and(|nav| nav.focused_column == index);
            if section == 1 {
                render_related_artist(frame, state, tile, size, &artists[item], selected);
            } else {
                render_related_album(frame, state, tile, size, &albums[item], selected);
            }
        }
        y += h;
    }
    Some(AlbumViewport {
        tracks: track_area,
        scroll,
        art_row,
    })
}

fn render_related_album(
    frame: &mut Frame,
    state: &RenderState<'_>,
    tile: Rect,
    size: u16,
    track: &crate::library::models::Track,
    selected: bool,
) {
    let style = if selected {
        crate::ui::theme::theme().colors.focused_row()
    } else {
        Style::default()
    };
    frame.render_widget(ratatui::widgets::Block::default().style(style), tile);
    let art = Rect::new(tile.x + (tile.width - size * 2) / 2, tile.y, size * 2, size);
    if size > 0
        && !track.parent_rating_key.as_ref().is_some_and(|key| {
            state
                .artwork
                .grid_cache
                .get(key)
                .is_some_and(|bytes| artwork::render_grid_image(frame, art, key, bytes))
        })
    {
        artwork::render_placeholder(frame, art, artwork::Placeholder::Album);
    }
    frame.render_widget(
        Paragraph::new(crate::app::view_options::wrapped_caption(
            track.album_name(),
            usize::from(tile.width),
        ))
        .alignment(ratatui::layout::Alignment::Center)
        .style(style),
        Rect::new(tile.x, tile.y + size, tile.width, tile.height - size),
    );
    state.hit_regions.borrow_mut().album_related.push((
        tile,
        track.clone(),
        crate::app::library_details::Destination::Album,
    ));
}

fn artist_list_recommendations(
    frame: &mut Frame,
    state: &RenderState<'_>,
    index: usize,
    column: &BrowseColumn,
    mut tracks: Rect,
    artists: &[crate::library::models::Artist],
) -> Rect {
    let area = tracks;
    if artists.is_empty() {
        return tracks;
    }
    let available = if column.recommendation_focus.is_some() {
        tracks.height.saturating_sub(1)
    } else {
        tracks.height.saturating_sub(8)
    };
    let section_h = available.min(if area.width < 48 { 2 } else { 5 });
    if section_h < 2 {
        return tracks;
    }
    tracks.height -= section_h;
    // Follow short catalogs immediately. Long catalogs keep enough room for browsing.
    let two_rows = state.browse_nav().is_some_and(|nav| {
        crate::app::presentation::is_two_row_browse_column(state, column, index, nav)
    });
    let item_rows = column.items.len().max(1) * if two_rows { 2 } else { 1 };
    let first_visible = state
        .scroll
        .browse
        .filter(|(col, _)| *col == index)
        .map(|(_, pos)| pos);
    let remaining = column
        .items
        .len()
        .saturating_sub(first_visible.unwrap_or(column.selected_index));
    let capacity = usize::from(area.height);
    if column.recommendation_focus.is_none() && remaining > capacity {
        return area;
    }
    if !column.artwork_visible {
        tracks.height = tracks
            .height
            .min(u16::try_from(item_rows.saturating_add(1)).unwrap_or(u16::MAX));
    }
    let y = tracks.bottom();
    let capacity = crate::app::view_options::terminal_grid_columns(area.width);
    let count = artists.len();
    let section = 1;
    let title = "Similar Artists";
    let last = count.saturating_sub(1) / capacity;
    let focused = column.recommendation_focus.filter(|(s, _)| *s == section);
    let page = focused
        .map_or(column.recommendation_pages[section], |(_, i)| i / capacity)
        .min(last);
    let title = if last > 0 {
        format!("{title}  {}/{}", page + 1, last + 1)
    } else {
        title.to_owned()
    };
    state.hit_regions.borrow_mut().recommendation_pages.push((
        Rect::new(area.x, y, area.width, section_h),
        index,
        section,
        last,
    ));
    frame.render_widget(Paragraph::new(title), Rect::new(area.x, y, area.width, 1));
    let width = area.width / (capacity as u16);
    let start = page * capacity;
    for i in start..count.min(start + capacity) {
        let tile = Rect::new(
            area.x + (i - start) as u16 * width,
            y + 1,
            width,
            section_h - 1,
        );
        let selected = focused == Some((section, i))
            && !state.category_column_focused
            && !state.track_pane_focused
            && state
                .browse_nav()
                .is_some_and(|nav| nav.focused_column == index);
        let size = tile.height.saturating_sub(1).min(4);
        render_related_artist(frame, state, tile, size, &artists[i], selected);
    }
    tracks
}

pub(in crate::ui) fn render_related_artist(
    frame: &mut Frame,
    state: &RenderState<'_>,
    tile: Rect,
    size: u16,
    artist: &crate::library::models::Artist,
    selected: bool,
) {
    let style = if selected {
        crate::ui::theme::theme().colors.focused_row()
    } else {
        Style::default()
    };
    frame.render_widget(ratatui::widgets::Block::default().style(style), tile);
    let width = tile.width.min(size * 2);
    let art = Rect::new(tile.x + (tile.width - width) / 2, tile.y, width, size);
    let key = &artist.rating_key;
    if !state
        .artwork
        .grid_cache
        .get(key)
        .is_some_and(|bytes| artwork::render_grid_portrait(frame, art, key, bytes))
    {
        artwork::render_placeholder(frame, art, artwork::Placeholder::Artist);
    }
    let caption = Rect::new(tile.x, art.bottom(), tile.width, tile.height - size);
    frame.render_widget(
        Paragraph::new(crate::app::view_options::wrapped_caption(
            &artist.title,
            usize::from(tile.width),
        ))
        .alignment(ratatui::layout::Alignment::Center)
        .style(style),
        caption,
    );
    state.hit_regions.borrow_mut().album_related.push((
        tile,
        crate::library::models::Track {
            grandparent_rating_key: Some(key.clone()),
            grandparent_title: Some(artist.title.clone()),
            ..Default::default()
        },
        crate::app::library_details::Destination::Artist,
    ));
}
