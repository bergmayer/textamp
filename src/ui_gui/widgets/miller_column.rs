//! Single Miller column widget.
//!
//! Renders a scrollable list of selectable items. A row reacts to click
//! (select + focus) and double-click (activate). Focus is drawn as a
//! highlight band matching the active theme.
//!
//! Performance: the list is virtualized. Rows outside the current
//! viewport (± a small buffer) are collapsed into two fixed-height
//! `Space` widgets at the top and bottom. Each visible row has its
//! height pinned to a constant so Iced's layout pass is O(visible),
//! not O(total). Without this, a 10k-row library would hitch for
//! hundreds of ms on every frame while cosmic-text measured every row.

use std::collections::HashMap;

use iced::border::Radius;
use iced::widget::{button, column, container, image, mouse_area, row, scrollable, text, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::Action;
use crate::app::state::{BrowseColumn, BrowseItem};
use crate::ui_gui::images;
use crate::ui_gui::message::GuiMessage;

/// Fixed per-row pixel heights. These MUST match the rendered widget's
/// actual size so the virtualization spacers don't drift from the real
/// content position. Rows force `.height(Length::Fixed(row_h))` so
/// layout is trivial and these values are authoritative.
pub const ROW_H_TEXT: f32 = 26.0;
pub const ROW_H_ART: f32 = 132.0;
pub const ROW_H_ACTION: f32 = 34.0;

const ART_THUMB_SIZE: f32 = 128.0;

/// How many rows of buffer to render above and below the viewport.
/// Large enough to cover one frame of fast-scroll latency between
/// `on_scroll` and the next re-render.
const BUFFER_ROWS: usize = 6;

/// Stable per-column scroll `Id` so `App::update` can snap the scrollable
/// viewport to keep the selected row visible on arrow-key navigation.
pub fn scroll_id_for(column_index: usize) -> scrollable::Id {
    scrollable::Id::new(format!("miller-col-{}", column_index))
}

/// Compute a `RelativeOffset` that keeps `selected_index` in view.
/// Linear mapping `0 → top`, `last → bottom`; the scrollable's own logic
/// clamps within the rendered list.
pub fn scroll_offset_for(selected_index: usize, total_items: usize) -> scrollable::RelativeOffset {
    let total = total_items.max(1) as f32;
    let y = (selected_index as f32 / (total - 1.0).max(1.0)).clamp(0.0, 1.0);
    scrollable::RelativeOffset { x: 0.0, y }
}

/// Per-row click handler ID: (column_index, item_index, activate).
/// `activate` is true for double-click / Enter-equivalent interactions.
#[derive(Debug, Clone, Copy)]
pub struct RowClick {
    pub column_index: usize,
    pub item_index: usize,
    pub activate: bool,
}

/// Height of the row that would render for `item` at the given art mode.
pub fn row_height_for(item: &BrowseItem, show_art: bool) -> f32 {
    if is_action_item(item) {
        ROW_H_ACTION
    } else if show_art && has_art_thumb(item) {
        ROW_H_ART
    } else {
        ROW_H_TEXT
    }
}

fn has_art_thumb(item: &BrowseItem) -> bool {
    matches!(
        item,
        BrowseItem::Album { .. }
            | BrowseItem::Playlist { .. }
            | BrowseItem::CompilationTracks { .. }
            | BrowseItem::AllTracks { .. }
    )
}

/// Build the Element tree for one Miller column.
///
/// `filter_matched` restricts the rows shown to the given original-item
/// indices; when `None`, every item in `col.items` is rendered.
///
/// `scroll_offset_y` / `viewport_h` come from the latest `MillerScroll`
/// message for this column. They drive the virtualization window: only
/// rows whose Y range overlaps `[scroll_offset_y, scroll_offset_y +
/// viewport_h]` (plus `BUFFER_ROWS` on each side) are rendered as real
/// widgets; the rest collapse into two `Space` spacers that preserve
/// the scrollable's content bounds.
pub fn view<'a>(
    column_index: usize,
    col: &'a BrowseColumn,
    is_focused: bool,
    grid_cache: &'a HashMap<String, Vec<u8>>,
    filter_matched: Option<&'a [usize]>,
    scroll_offset_y: f32,
    viewport_h: f32,
    on_row_click: impl Fn(RowClick) -> GuiMessage + 'a + Clone,
) -> Element<'a, GuiMessage> {
    // Header row shows the column title (e.g. "artists", "albums - R.E.M.")
    // along with any column-level action buttons. Both Artist Radio
    // (album column) and Play Album (tracks column) live in the
    // header — that way both buttons sit at the same fixed offset
    // below their title row, so they read as a row of aligned actions
    // when the columns are displayed side-by-side.
    // Title row doubles as a click target — clicking it focuses
    // this column without changing selection or drill state.
    let title_row: Element<'_, GuiMessage> = mouse_area(
        container(text(sanitize(&col.title)).size(12))
            .padding([4, 8])
            .width(Length::Fill),
    )
    .on_press(GuiMessage::FocusMillerColumn { column_index })
    .into();

    // Find an `ArtistRadio` row inside the items list, if present.
    // We hoist it into the header as a button (matching Play Album)
    // and skip it in the row iteration below so it isn't drawn twice.
    let artist_radio_idx = col.items.iter().position(|it| {
        matches!(it, BrowseItem::ArtistRadio { .. })
    });

    let mut header_col = Column::new().spacing(2).push(title_row);
    if let Some(ar_idx) = artist_radio_idx {
        if let Some(BrowseItem::ArtistRadio { artist_name, .. }) = col.items.get(ar_idx) {
            use crate::ui_gui::widgets::transport_bar::primary_action_button;
            let label = format!("Artist Radio - {}", sanitize(artist_name));
            let click = RowClick { column_index, item_index: ar_idx, activate: true };
            let on_click_for_btn = on_row_click.clone();
            let btn = primary_action_button(label, on_click_for_btn(click));
            header_col = header_col.push(
                container(btn)
                    .center_x(Length::Fill)
                    .padding([2, 8]),
            );
        }
    }
    if let Some((album_key, album_title)) = col.play_album.as_ref() {
        use crate::app::action::QueueAction;
        use crate::ui_gui::widgets::transport_bar::primary_action_button;
        let key = album_key.clone();
        let title = album_title.clone();
        let action_btn = primary_action_button(
            "Play Album",
            GuiMessage::Action(crate::app::Action::Queue(
                QueueAction::PlayAlbumNow { rating_key: key, title }
            )),
        );
        header_col = header_col.push(
            container(action_btn)
                .center_x(Length::Fill)
                .padding([2, 8]),
        );
    } else if let Some(label) = col.play_all_label.as_ref() {
        // Virtual "all tracks" columns (artist all-tracks, library
        // all-tracks, compilation tracks) don't have a single album
        // key — queue the column's `tracks` directly.
        use crate::app::action::QueueAction;
        use crate::ui_gui::widgets::transport_bar::primary_action_button;
        let tracks = col.tracks.clone();
        let action_btn = primary_action_button(
            label.clone(),
            GuiMessage::Action(crate::app::Action::Queue(
                QueueAction::PlayTracksNow(tracks)
            )),
        );
        header_col = header_col.push(
            container(action_btn)
                .center_x(Length::Fill)
                .padding([2, 8]),
        );
    }
    let header: Element<'_, _> = container(header_col)
        .width(Length::Fill)
        .into();

    // Resolve the ordered list of source indices that should render.
    // An active filter hides non-matching rows; otherwise every item
    // is a candidate. The hoisted Artist Radio row is dropped here so
    // the scrollable doesn't draw it a second time below the header.
    let indices: Vec<usize> = match filter_matched {
        Some(matched) if matched.is_empty() => {
            // Filter active with no matches — single informative row,
            // no virtualization needed.
            let body = container(text("no matches").size(12))
                .padding([6, 10]);
            return wrap_chrome(column_index, header, body.into(), is_focused);
        }
        Some(matched) => matched.iter().copied().collect(),
        None => (0..col.items.len()).collect(),
    };
    let indices: Vec<usize> = if let Some(ar_idx) = artist_radio_idx {
        indices.into_iter().filter(|&i| i != ar_idx).collect()
    } else {
        indices
    };

    let show_art = col.artwork_visible;
    let selected = col.selected_index;

    // Cumulative Y positions at each row's top. `cum[k]` is the Y of
    // the top of the k-th visible row; `cum[n]` is the total height.
    let mut cum: Vec<f32> = Vec::with_capacity(indices.len() + 1);
    cum.push(0.0);
    for &i in &indices {
        let h = col.items.get(i).map(|it| row_height_for(it, show_art)).unwrap_or(0.0);
        cum.push(cum.last().copied().unwrap_or(0.0) + h);
    }
    let total_h = cum.last().copied().unwrap_or(0.0);

    // Visible window. `first`/`last` are indices into `indices`.
    let view_top = scroll_offset_y.max(0.0);
    let view_bot = if viewport_h > 0.0 {
        scroll_offset_y + viewport_h
    } else {
        // No scroll info yet (first render, before `on_scroll` has
        // fired). Render a generous slice from the top so the initial
        // paint fills any reasonable viewport without blank gaps.
        4000.0
    };

    // First visible: last index where cum[k] <= view_top.
    let first_exact = cum.partition_point(|&p| p <= view_top).saturating_sub(1);
    // Last visible (exclusive): first index where cum[k] >= view_bot.
    let last_exact = cum.partition_point(|&p| p < view_bot);

    let first = first_exact.saturating_sub(BUFFER_ROWS);
    let last = (last_exact + BUFFER_ROWS).min(indices.len());

    let top_spacer_h = cum.get(first).copied().unwrap_or(0.0);
    let bot_spacer_h = (total_h - cum.get(last).copied().unwrap_or(total_h)).max(0.0);

    let mut rendered: Vec<Element<'a, GuiMessage>> = Vec::with_capacity(last.saturating_sub(first) + 2);
    if top_spacer_h > 0.0 {
        rendered.push(Space::with_height(Length::Fixed(top_spacer_h)).into());
    }
    // For Track rows: prefix with the track artist when the column
    // is a multi-artist context (playlist, compilations, all-tracks
    // columns). Single-artist album-tracks columns (where
    // `col.play_album` is set) hide the prefix because the artist is
    // already obvious from the parent column.
    let show_track_artist = col.play_album.is_none();
    for k in first..last {
        let i = indices[k];
        let item = match col.items.get(i) {
            Some(it) => it,
            None => continue,
        };
        let on_click = on_row_click.clone();
        let row_h = row_height_for(item, show_art);
        rendered.push(row_item(
            column_index,
            i,
            item,
            i == selected,
            is_focused,
            show_art,
            grid_cache,
            row_h,
            show_track_artist,
            move |click| on_click(click),
        ));
    }
    if bot_spacer_h > 0.0 {
        rendered.push(Space::with_height(Length::Fixed(bot_spacer_h)).into());
    }

    let col_idx_for_scroll = column_index;
    let body = scrollable(Column::with_children(rendered))
        .id(scroll_id_for(column_index))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .on_scroll(move |v| {
            let off = v.absolute_offset();
            let b = v.bounds();
            let cb = v.content_bounds();
            GuiMessage::MillerScroll {
                column_index: col_idx_for_scroll,
                offset_y: off.y,
                bounds_h: b.height,
                content_h: cb.height,
            }
        })
        .height(Length::Fill);

    wrap_chrome(column_index, header, body.into(), is_focused)
}

fn wrap_chrome<'a>(
    _column_index: usize,
    header: Element<'a, GuiMessage>,
    body: Element<'a, GuiMessage>,
    is_focused: bool,
) -> Element<'a, GuiMessage> {
    let chrome = container(column![header, body])
        .padding(4)
        .width(Length::Fill)
        .height(Length::Fill);

    let focused = is_focused;
    chrome
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let border_color = if focused {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            };
            container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                text_color: Some(palette.background.base.text),
                border: Border { color: border_color, width: 1.0, radius: Radius::default() },
                ..container::Style::default()
            }
        })
        .into()
}

fn row_item<'a>(
    column_index: usize,
    row_index: usize,
    item: &'a BrowseItem,
    is_selected: bool,
    is_focused_column: bool,
    show_art: bool,
    grid_cache: &'a HashMap<String, Vec<u8>>,
    row_h: f32,
    show_track_artist: bool,
    on_click: impl Fn(RowClick) -> GuiMessage + 'a,
) -> Element<'a, GuiMessage> {
    // Action-style entries (Artist Radio, Compilations, etc.) are not
    // drillable — render as a popout-button that fires immediately.
    if is_action_item(item) {
        return action_button(column_index, row_index, item, row_h, on_click);
    }

    let label = label_for(item, show_track_artist);
    let activate = is_selected;

    let row_style = move |theme: &Theme, status: button::Status| -> button::Style {
        let palette = theme.extended_palette();
        let (bg, fg) = if is_selected && is_focused_column {
            (palette.primary.strong.color, palette.primary.strong.text)
        } else if is_selected {
            (palette.background.weak.color, palette.background.base.text)
        } else {
            match status {
                button::Status::Hovered => (palette.background.weak.color, palette.background.weak.text),
                _ => (Color::TRANSPARENT, palette.background.base.text),
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border::default(),
            ..button::Style::default()
        }
    };

    if show_art && has_art_thumb(item) {
        let thumb = art_handle_for(item, grid_cache);
        let art_key = art_key_for(item).map(|s| s.to_string());
        let art_thumb_path = art_thumb_path_for(item).map(|s| s.to_string());
        let art_el: Element<'a, GuiMessage> = match thumb {
            Some(h) => {
                let img = image(h)
                    .width(Length::Fixed(ART_THUMB_SIZE))
                    .height(Length::Fixed(ART_THUMB_SIZE));
                if let (Some(key), Some(path)) = (art_key, art_thumb_path) {
                    mouse_area(img)
                        .on_press(GuiMessage::OpenArtPopup { key, thumb_path: path })
                        .into()
                } else {
                    img.into()
                }
            }
            None => Space::with_width(Length::Fixed(ART_THUMB_SIZE)).into(),
        };

        let label_btn = button(text(label).size(13))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([4, 8])
            .on_press_with(move || on_click(RowClick { column_index, item_index: row_index, activate }))
            .style(row_style);

        let label_with_ctx = mouse_area(label_btn)
            .on_right_press(GuiMessage::OpenMillerContextMenu {
                column_index,
                item_index: row_index,
            });

        container(
            row![art_el, label_with_ctx]
                .spacing(6)
                .align_y(Alignment::Center)
                .height(Length::Fill),
        )
        .height(Length::Fixed(row_h))
        .width(Length::Fill)
        .into()
    } else {
        let btn = button(text(label).size(13))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([4, 8])
            .on_press_with(move || on_click(RowClick { column_index, item_index: row_index, activate }))
            .style(row_style);

        container(
            mouse_area(btn)
                .on_right_press(GuiMessage::OpenMillerContextMenu {
                    column_index,
                    item_index: row_index,
                }),
        )
        .height(Length::Fixed(row_h))
        .width(Length::Fill)
        .into()
    }
}

/// Items that read as actions rather than drill targets. Their Miller
/// rows are rendered as distinct buttons.
///
/// Only `ArtistRadio` qualifies: it fires immediately and never opens
/// a child column. Everything else in the artist / album columns
/// (`Compilations`, `AllTracks`, `CompilationTracks`) drills, so it
/// renders as a regular row with the same chrome as a real artist or
/// album entry — keeping the columns visually consistent.
fn is_action_item(item: &BrowseItem) -> bool {
    matches!(item, BrowseItem::ArtistRadio { .. })
}

fn action_button<'a>(
    column_index: usize,
    row_index: usize,
    item: &'a BrowseItem,
    row_h: f32,
    on_click: impl Fn(RowClick) -> GuiMessage + 'a,
) -> Element<'a, GuiMessage> {
    use crate::ui_gui::widgets::transport_bar::primary_action_button;
    let label = action_label_for(item);
    let click = RowClick { column_index, item_index: row_index, activate: true };
    let btn = primary_action_button(&label, on_click(click));
    container(btn)
        .center_x(Length::Fill)
        .height(Length::Fixed(row_h))
        .padding([2, 8])
        .into()
}

fn action_label_for(item: &BrowseItem) -> String {
    match item {
        BrowseItem::ArtistRadio { artist_name, .. } => format!("Artist Radio - {}", sanitize(artist_name)),
        BrowseItem::Compilations => "Compilations".to_string(),
        BrowseItem::CompilationTracks { artist_name, .. } => format!("Compilation Tracks - {}", sanitize(artist_name)),
        _ => String::new(),
    }
}

/// Strip characters that typically don't render in the default GUI font
/// (emoji + other supplementary-plane glyphs) from user-sourced titles.
/// Without this, playlist names like "🎵 Tracks" print as tofu/boxes.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| (*c as u32) < 0x10000 && !is_symbol_emoji(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_symbol_emoji(c: char) -> bool {
    let cp = c as u32;
    // Misc Symbols, Dingbats, Misc Symbols & Pictographs (BMP ranges
    // that `Segoe UI` doesn't cover but appear in playlist titles).
    (0x2600..=0x26FF).contains(&cp)
        || (0x2700..=0x27BF).contains(&cp)
        || (0x2B00..=0x2BFF).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp) // Variation Selectors
}

fn art_handle_for<'a>(
    item: &'a BrowseItem,
    grid_cache: &'a HashMap<String, Vec<u8>>,
) -> Option<iced::widget::image::Handle> {
    let key = art_key_for(item)?;
    images::lookup_grid(grid_cache, key)
}

fn art_key_for(item: &BrowseItem) -> Option<&str> {
    match item {
        BrowseItem::Album { key, .. } => Some(key.as_str()),
        BrowseItem::Playlist { key, .. } => Some(key.as_str()),
        // Compilation Tracks shows the artist's own thumbnail so the
        // row reads as "another album by this artist" in the column.
        BrowseItem::CompilationTracks { artist_key, .. } => Some(artist_key.as_str()),
        // All Tracks is a virtual album for the artist — render it
        // album-style with the artist's thumbnail (preloaded under
        // `artist_key` by `collect_art_to_load`).
        BrowseItem::AllTracks { artist_key, .. } => Some(artist_key.as_str()),
        _ => None,
    }
}

fn art_thumb_path_for(item: &BrowseItem) -> Option<&str> {
    match item {
        BrowseItem::Album { thumb, .. } => thumb.as_deref(),
        // Playlists have no thumb field on the enum; popup will fall
        // back to the cached grid bytes when no thumb_path is known.
        _ => None,
    }
}

fn label_for(item: &BrowseItem, show_track_artist: bool) -> String {
    match item {
        BrowseItem::Artist { title, .. } => sanitize(title),
        BrowseItem::Album { title, artist, year, .. } => {
            let t = sanitize(title);
            let a = sanitize(artist);
            match year {
                Some(y) => format!("{}  ({})  - {}", t, y, a),
                None => format!("{}  - {}", t, a),
            }
        }
        BrowseItem::Track { title, artist_name, track_number, duration_ms, .. } => {
            let n = track_number.map(|n| format!("{:02}. ", n)).unwrap_or_default();
            let m = duration_ms / 60_000;
            let s = (duration_ms / 1000) % 60;
            // In multi-artist columns (playlist tracks, compilation
            // tracks, all-tracks-by-X), prefix with the track artist
            // so the user can tell rows apart at a glance. Single-
            // album columns suppress the prefix because the artist
            // is implied by the parent column.
            let title_str = sanitize(title);
            let body = match (show_track_artist, artist_name.as_deref()) {
                (true, Some(a)) if !a.is_empty() => format!("{} - {}", sanitize(a), title_str),
                _ => title_str,
            };
            format!("{}{}  {:>2}:{:02}", n, body, m, s)
        }
        BrowseItem::Genre { title, .. } => sanitize(title),
        BrowseItem::GenreCategory { title, .. } => sanitize(title),
        BrowseItem::Playlist { title, track_count, .. } => {
            let t = sanitize(title);
            track_count
                .map(|n| format!("{}  ({} tracks)", t, n))
                .unwrap_or(t)
        }
        BrowseItem::AllTracks { artist_name, .. } => format!("All Tracks - {}", sanitize(artist_name)),
        BrowseItem::AllArtists => "All Artists".to_string(),
        BrowseItem::ArtistRadio { artist_name, .. } => format!("Artist Radio - {}", sanitize(artist_name)),
        BrowseItem::Compilations => "Compilations".to_string(),
        BrowseItem::CompilationTracks { artist_name, .. } => format!("Compilation Tracks - {}", sanitize(artist_name)),
    }
}

/// Helper that converts a `RowClick` to the Action(s) the TUI would emit
/// for an equivalent click. Called by the browse screen in response to
/// `on_row_click` messages.
pub fn actions_for_click(_click: RowClick) -> Vec<Action> {
    Vec::new()
}
