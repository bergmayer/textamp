//! Browse view — category column (left) + Miller columns for the active
//! category (right).
//!
//! Mirrors the TUI's `render_browse` layout (src/ui/app.rs:141): the
//! leftmost column is the category selector (Library / Playlists / Genres /
//! Folders), and the remaining columns are Miller columns for the active
//! category's navigation state.

use iced::widget::{button, column as iced_column, container, image, scrollable, text, Column, Row, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{NavigationAction, QueueAction};
use crate::app::state::BrowseCategory;
use crate::app::{Action, AppState};
use crate::plex::models::{FolderColumn, FolderItem, FolderItemType, Track};
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::miller_column;
use crate::ui_gui::widgets::transport_bar::primary_action_button;

const CATEGORY_COL_WIDTH: f32 = 160.0;
const TRACK_DETAILS_WIDTH: f32 = 380.0;
const ALPHABET_STRIP_WIDTH: f32 = 30.0;
const ALPHABET_STRIP_FONT: f32 = 15.0;

/// Minimum width a Miller column is allowed to occupy before the layout
/// sheds an older column off the left edge. Chosen to stay readable —
/// artists like "The Brian Jonestown Massacre" truncate past ~14 chars
/// below this width.
const MIN_MILLER_COL_WIDTH: f32 = 260.0;

pub fn view<'a>(
    state: &'a AppState,
    viewport_width_logical: f32,
    scroll_info: impl Fn(usize) -> (f32, f32) + Copy + 'a,
    track_pane_similar: &'a std::collections::HashMap<String, Vec<Track>>,
) -> Element<'a, GuiMessage> {
    // The alphabet strip is conceptually part of the artists column
    // — it scrolls through the alphabetised artist list. Hidden:
    //   - In every category other than Library (no artist list).
    //   - When the artist column is shuffled (the items aren't
    //     alphabetical, so jumping to a letter would land arbitrarily).
    // The strip's letter order is also reversed when the artist
    // column is sorted descending, so a click on "Z" still jumps to
    // the top of the visible list.
    use crate::app::state::ColumnSortMode;
    let artist_col0 = state.artist_nav.columns.first();
    let artist_shuffled = artist_col0.map_or(false, |c| c.sort_mode == ColumnSortMode::Shuffled);
    let artist_descending = artist_col0.map_or(false, |c| !c.sort_ascending);
    let show_strip = matches!(state.browse_category, BrowseCategory::Library) && !artist_shuffled;
    let strip_reserved = if show_strip { ALPHABET_STRIP_WIDTH + 4.0 } else { 0.0 };
    // The track-details pane is purely a function of the currently
    // focused Miller column: when its highlighted row is a Track, we
    // show the pane and feed it that track's full data. Navigating to
    // a non-track column (artists/albums/etc.) hides the pane
    // automatically — no stale "last clicked track" lingering on the
    // right while the left columns moved on.
    let details_track = focused_track(state);
    let details_reserved = if details_track.is_some() { TRACK_DETAILS_WIDTH + 4.0 } else { 0.0 };
    let content_width = (viewport_width_logical - CATEGORY_COL_WIDTH - strip_reserved - details_reserved - 16.0)
        .max(MIN_MILLER_COL_WIDTH);
    let category_col = category_column(state);
    let content = content_columns(state, content_width, scroll_info);

    let mut children: Vec<Element<'a, GuiMessage>> = vec![category_col];
    if show_strip {
        children.push(alphabet_strip(artist_descending));
    }
    children.push(content);
    if let Some(track) = details_track {
        children.push(track_details_pane(track, state, track_pane_similar));
    }

    let body = Row::with_children(children)
        .spacing(4)
        .width(Length::Fill)
        .height(Length::Fill);

    // No outer padding — the alphabet strip is supposed to span the
    // full vertical extent between the menu bar and transport bar.
    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Vertical (% + 0 + A–Z) strip. Symbols sort first, then digits, then
/// the alphabet — the same order as `helpers::sort_key`. Each glyph is
/// a tight button that dispatches `GuiMessage::AlphabetJump(ch)` so the
/// focused root list scrolls to the first item starting with that
/// character.
fn alphabet_strip<'a>(descending: bool) -> Element<'a, GuiMessage> {
    let mut chars: Vec<char> = std::iter::once('%')
        .chain(std::iter::once('0'))
        .chain('a'..='z')
        .collect();
    if descending {
        // Sort-descending puts Z-named artists at the top of the
        // list, so the alphabet strip should mirror that order:
        // Z…A, then 9…0, then % at the bottom (the last sort
        // key in the natural ordering).
        chars.reverse();
    }

    // The strip fills the full available vertical space; each glyph
    // takes an equal share so the buttons span top-to-bottom edge.
    // Each glyph is centered horizontally and vertically inside its
    // cell so the letters look balanced regardless of cell height.
    let mut col = Column::new().spacing(0).align_x(iced::Alignment::Center).height(Length::Fill);
    for ch in chars {
        let label = match ch {
            '0' => "0".to_string(),
            '%' => "%".to_string(),
            c   => c.to_ascii_uppercase().to_string(),
        };
        let inner = container(text(label).size(ALPHABET_STRIP_FONT))
            .center_x(Length::Fill)
            .center_y(Length::Fill);
        col = col.push(
            button(inner)
                .width(Length::Fill)
                .height(Length::FillPortion(1))
                .padding(0)
                .on_press(GuiMessage::AlphabetJump(ch))
                .style(|theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    let (bg, fg) = match status {
                        button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                        _ => (Color::TRANSPARENT, p.background.base.text),
                    };
                    button::Style {
                        background: Some(Background::Color(bg)),
                        text_color: fg,
                        border: Border::default(),
                        ..button::Style::default()
                    }
                })
        );
    }

    container(col)
        .width(Length::Fixed(ALPHABET_STRIP_WIDTH))
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                text_color: Some(p.background.base.text),
                border: Border { color: p.background.strong.color, width: 1.0, radius: 0.0.into() },
                ..container::Style::default()
            }
        })
        .into()
}

/// Track-details side pane. Shown to the right of the Miller columns
/// whenever the user clicks a track row. Replaces (does not stack)
/// on each new track click — single-pane drill, terminal node.
/// One row in the "Sonically Similar" list. Click → navigate to the
/// track's album in the Library (artist + album drill via the shared
/// `BrowseAction::OpenInLibrary`); the user lands at the album view
/// with the row highlighted. Right-click → standard track context
/// menu (Play / Add to queue / Show Similar / Open in Library / …).
fn similar_row<'a>(track: &'a Track) -> Element<'a, GuiMessage> {
    use crate::app::action::BrowseAction;
    use iced::widget::{button, mouse_area};
    let label = format!("{} \u{2014} {}", track.title, track.track_artist());
    let click_action: Option<GuiMessage> = match (
        track.grandparent_rating_key.clone(),
        track.parent_rating_key.clone(),
    ) {
        (Some(artist_key), album_key) => Some(GuiMessage::Action(Action::Browse(
            BrowseAction::OpenInLibrary {
                artist_key,
                artist_name: track.artist_name().to_string(),
                album_key,
                album_title: track.parent_title.clone(),
            },
        ))),
        _ => None,
    };
    let body = button(text(label).size(12))
        .width(Length::Fill)
        .padding([3, 8])
        .on_press_maybe(click_action)
        .style(|theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = match status {
                button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
                _ => (Color::TRANSPARENT, p.background.base.text),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border::default(),
                ..button::Style::default()
            }
        });
    // Right-click → standard track context menu (Play / Play next /
    // Add to queue / Show Similar Tracks / Show Similar Albums /
    // Related Artists / Sonic Adventure / Show Artist Bio / Open in
    // Library). Same as the menu Browse miller-column track rows and
    // queue rows show.
    mouse_area(body)
        .on_right_press(GuiMessage::OpenStandaloneTrackContextMenu(Box::new(track.clone())))
        .into()
}

/// The track to feed into the details pane: the currently selected row
/// of the focused Miller column, but only if that row is actually a
/// `BrowseItem::Track`. Returns `None` for artist/album/genre/playlist
/// rows, which keeps the pane hidden whenever the user is navigating
/// in a non-track column.
fn focused_track(state: &AppState) -> Option<&Track> {
    use crate::app::state::BrowseItem;
    let nav = state.browse_nav()?;
    let col = nav.focused()?;
    let item = col.items.get(col.selected_index)?;
    if !matches!(item, BrowseItem::Track { .. }) {
        return None;
    }
    col.tracks.get(col.selected_index)
}

fn track_details_pane<'a>(
    track: &'a Track,
    state: &'a AppState,
    track_pane_similar: &'a std::collections::HashMap<String, Vec<Track>>,
) -> Element<'a, GuiMessage> {
    use crate::ui_gui::images::lookup_grid;

    // No "x" close affordance: the pane is now driven entirely by
    // which column is focused. Clicking out of the track column is
    // the way to dismiss it.
    let header = container(
        Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(text(" track ").size(12))
            .push(Space::with_width(Length::Fill)),
    )
    .padding([4, 8])
    .width(Length::Fill);

    let artwork: Element<'_, GuiMessage> = match track.parent_rating_key.as_ref()
        .and_then(|k| lookup_grid(&state.artwork.grid_cache, k))
    {
        Some(handle) => image(handle)
            .width(Length::Fixed(280.0))
            .height(Length::Fixed(280.0))
            .into(),
        None => container(text("(no cover)").size(11))
            .width(Length::Fixed(280.0))
            .height(Length::Fixed(280.0))
            .center_x(Length::Fixed(280.0))
            .center_y(Length::Fixed(280.0))
            .style(|theme: &Theme| {
                let p = theme.extended_palette();
                container::Style {
                    background: Some(Background::Color(p.background.weak.color)),
                    text_color: Some(p.background.weak.text),
                    border: Border { color: p.background.strong.color, width: 1.0, radius: 0.0.into() },
                    ..container::Style::default()
                }
            })
            .into(),
    };

    // Play Track button on TOP of the info section, before any text.
    // Uses the shared `primary_action_button` so it matches Play Album
    // (album-tracks column header) and Artist Radio (action item).
    // Centred via the surrounding container so the lozenge button
    // doesn't stretch the full width of the details pane.
    let track_for_play = track.clone();
    let play_btn: Element<'_, GuiMessage> = container(
        primary_action_button(
            "Play Track",
            GuiMessage::Action(Action::Queue(QueueAction::PlayTrack(track_for_play))),
        ),
    )
    .center_x(Length::Fill)
    .into();

    let title = text(track.title.clone()).size(16);
    let artist = text(track.track_artist().to_string()).size(13);
    let album_year = match (track.parent_title.as_deref(), track.year) {
        (Some(a), Some(y)) => format!("{a}  ({y})"),
        (Some(a), None)    => a.to_string(),
        (None, Some(y))    => y.to_string(),
        (None, None)       => String::new(),
    };
    let album = text(album_year).size(12);
    let duration = {
        let total = track.duration_ms();
        let m = total / 60_000;
        let s = (total / 1000) % 60;
        text(format!("Duration: {m}:{s:02}")).size(12)
    };
    let track_no = track.index.map(|n| text(format!("Track #{n}")).size(12));

    // File path: pulled from the first MediaPart. Show only the
    // basename so the column doesn't have to be widened for long
    // server-side paths; the full path is in the tooltip-equivalent
    // cached data (dispatch can look it up if needed later).
    let filename: Option<Element<'_, GuiMessage>> = track
        .stream_part()
        .and_then(|p| p.file.as_deref())
        .map(|path| {
            let basename = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
            text(format!("File: {basename}")).size(11).into()
        });

    let mut info_col = Column::new()
        .spacing(6)
        .push(play_btn)
        .push(Space::with_height(Length::Fixed(8.0)))
        .push(title)
        .push(artist)
        .push(album)
        .push(duration);
    if let Some(t) = track_no {
        info_col = info_col.push(t);
    }
    if let Some(f) = filename {
        info_col = info_col.push(f);
    }

    // Sonically-similar tracks list, populated lazily by an iced
    // `Task::perform` from the App's Tick handler. While the cache
    // is empty for this track, render a placeholder line; clicking a
    // row navigates to that track's album in the Library.
    let similar_section: Element<'_, GuiMessage> = {
        let mut col = Column::new().spacing(4).padding([8, 0]);
        col = col.push(text("Sonically Similar").size(13));
        match track_pane_similar.get(&track.rating_key) {
            Some(list) if list.is_empty() => col
                .push(text("(no similar tracks found)").size(11))
                .into(),
            Some(list) => {
                for sim in list.iter() {
                    col = col.push(similar_row(sim));
                }
                col.into()
            }
            None => col.push(text("Loading\u{2026}").size(11)).into(),
        }
    };

    let body = scrollable(
        Column::new()
            .spacing(12)
            .padding(12)
            .align_x(Alignment::Center)
            .push(artwork)
            .push(info_col)
            .push(similar_section),
    )
    .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
    .style(crate::ui_gui::widgets::chunky_scrollable_style)
    .height(Length::Fill);

    container(iced_column![header, body].spacing(0))
        .width(Length::Fixed(TRACK_DETAILS_WIDTH))
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                text_color: Some(p.background.base.text),
                border: Border { color: p.background.strong.color, width: 1.0, radius: 0.0.into() },
                ..container::Style::default()
            }
        })
        .into()
}

fn category_column(state: &AppState) -> Element<'_, GuiMessage> {
    let is_focused = state.category_column_focused;
    let categories = BrowseCategory::all();

    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
    for (i, cat) in categories.iter().copied().enumerate() {
        let is_selected = i == state.category_column_index;
        let label = match cat {
            BrowseCategory::Library => "Library",
            BrowseCategory::Playlists => "Playlists",
            BrowseCategory::Genres => "Genres",
            BrowseCategory::Folders => "Folders",
        };
        let row_label = text(label).size(13);

        // Match the row colour logic used by `miller_column::row_item`
        // verbatim so the category column reads as just another Miller
        // column in selection state.
        rows.push(
            button(row_label)
                .width(Length::Fill)
                .padding([4, 8])
                .on_press(GuiMessage::Action(Action::Navigation(
                    NavigationAction::SetCategory(cat),
                )))
                .style(move |theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    let (bg, fg) = if is_selected && is_focused {
                        (p.primary.strong.color, p.primary.strong.text)
                    } else if is_selected {
                        (p.background.weak.color, p.background.base.text)
                    } else {
                        match status {
                            button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
                            _ => (Color::TRANSPARENT, p.background.base.text),
                        }
                    };
                    button::Style {
                        background: Some(Background::Color(bg)),
                        text_color: fg,
                        border: Border::default(),
                        ..button::Style::default()
                    }
                })
                .into(),
        );
    }

    let list = scrollable(Column::with_children(rows).spacing(0))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill);

    // Container chrome matches `miller_column::wrap_chrome` so the
    // category column draws the same border + bg as a regular Miller
    // column. No header row — the category names are self-evident.
    container(list)
        .width(Length::Fixed(CATEGORY_COL_WIDTH))
        .height(Length::Fill)
        .padding(4)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let border_color = if is_focused {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            };
            container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                text_color: Some(palette.background.base.text),
                border: Border { color: border_color, width: 1.0, radius: 0.0.into() },
                ..container::Style::default()
            }
        })
        .into()
}

fn content_columns<'a>(
    state: &'a AppState,
    content_width: f32,
    scroll_info: impl Fn(usize) -> (f32, f32) + Copy + 'a,
) -> Element<'a, GuiMessage> {
    // Folders use `FolderNavigationState`; render via folder_columns.
    if state.browse_category == BrowseCategory::Folders {
        return folder_columns(state, content_width);
    }

    let grid_cache = &state.artwork.grid_cache;
    if let Some(nav) = state.browse_nav() {
        if nav.columns.is_empty() {
            return container(text("Loading\u{2026}").size(14))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        let cat_focused = state.category_column_focused;
        let focused = if cat_focused { usize::MAX } else { nav.focused_column };
        let (start, end, col_width) = compute_visible_window(
            nav.columns.len(),
            nav.focused_column,
            content_width,
        );

        // Inline list filter (Alt+F / quick-filter text input) restricts
        // a single column to items matching the query. When active, the
        // matched_indices slice is handed to `miller_column::view` which
        // only renders those rows.
        let filter_on_col: Option<usize> = if state.list_filter.active
            && state.list_filter.category == state.browse_category
        {
            Some(state.list_filter.column)
        } else {
            None
        };
        let matched: Option<&[usize]> = state
            .list_filter
            .results
            .as_ref()
            .map(|r| r.matched_indices.as_slice());

        let cols = (start..end).map(|idx| {
            let col = &nav.columns[idx];
            let is_last_visible = idx + 1 == end;
            let filter_matched = if filter_on_col == Some(idx) { matched } else { None };
            let (scroll_y, vp_h) = scroll_info(idx);
            let element = miller_column::view(idx, col, idx == focused, grid_cache, filter_matched, scroll_y, vp_h, |click| GuiMessage::MillerSelect {
                column_index: click.column_index,
                item_index: click.item_index,
                activate: click.activate,
            });
            // Fixed-width for all visible columns except the last, which
            // fills remaining space so rounding errors do not leave a gap.
            let element: Element<'_, GuiMessage> = if is_last_visible {
                container(element).width(Length::Fill).height(Length::Fill).into()
            } else {
                container(element).width(Length::Fixed(col_width)).height(Length::Fill).into()
            };
            element
        }).collect::<Vec<_>>();
        Row::with_children(cols)
            .spacing(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        container(text("Loading\u{2026}").size(14))
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn folder_columns(state: &AppState, content_width: f32) -> Element<'_, GuiMessage> {
    let fs = match state.folder_state.as_ref() {
        Some(fs) => fs,
        None => {
            return container(text("Loading folders\u{2026}").size(14))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
    };
    if fs.columns.is_empty() {
        return container(text("No folders").size(14))
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let cat_focused = state.category_column_focused;
    let focused = if cat_focused { usize::MAX } else { fs.focused_column };
    let (start, end, col_width) = compute_visible_window(
        fs.columns.len(),
        fs.focused_column,
        content_width,
    );
    let cols = (start..end).map(|idx| {
        let col = &fs.columns[idx];
        let is_last_visible = idx + 1 == end;
        let element = folder_column_view(idx, col, idx == focused);
        let element: Element<'_, GuiMessage> = if is_last_visible {
            container(element).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(element).width(Length::Fixed(col_width)).height(Length::Fill).into()
        };
        element
    }).collect::<Vec<_>>();
    Row::with_children(cols)
        .spacing(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Given the total number of Miller columns, the focused column index,
/// and the horizontal space available, pick which columns to show and
/// how wide each should be.
///
/// Mirrors the TUI's sliding-window logic (`src/ui/app.rs::render_browse`
/// ~L820–L845): honor `MIN_MILLER_COL_WIDTH`, show at least 1 column,
/// prefer to reveal the rightmost column (active drill depth), and keep
/// the focused column visible.
fn compute_visible_window(total: usize, focused: usize, width: f32) -> (usize, usize, f32) {
    if total == 0 {
        return (0, 0, width.max(MIN_MILLER_COL_WIDTH));
    }
    let max_visible = ((width / MIN_MILLER_COL_WIDTH).floor() as usize).max(1);
    let rightmost = total.saturating_sub(1).max(focused);

    let start = if rightmost + 1 > max_visible {
        let s = rightmost + 1 - max_visible;
        s.min(focused)
    } else {
        0
    };
    let end = (start + max_visible).min(total);
    let visible_n = end - start;
    let col_width = if visible_n == 0 { width } else { width / visible_n as f32 };
    (start, end, col_width.max(MIN_MILLER_COL_WIDTH))
}


fn folder_column_view(column_index: usize, col: &FolderColumn, is_focused: bool) -> Element<'_, GuiMessage> {
    const FOLDER_ROW_H: f32 = 26.0;

    let header = container(text(&col.title).size(12))
        .padding([4, 8])
        .width(Length::Fill);

    let selected = col.selected_index;
    // Pin each row to a fixed pixel height so Iced's layout pass skips
    // cosmic-text measurement for every item. A 10k-entry folder list
    // was the worst offender before this — layout alone took ~300 ms
    // per frame.
    let rows: Vec<Element<'_, GuiMessage>> = col.items.iter().enumerate().map(|(i, it)| {
        container(folder_row(column_index, it, i, i == selected, is_focused))
            .height(Length::Fixed(FOLDER_ROW_H))
            .width(Length::Fill)
            .into()
    }).collect();

    let body = scrollable(Column::with_children(rows))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill);

    container(iced_column![header, body].spacing(0))
        .padding(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let border_color = if is_focused {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            };
            container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                text_color: Some(palette.background.base.text),
                border: Border { color: border_color, width: 1.0, radius: 0.0.into() },
                ..container::Style::default()
            }
        })
        .into()
}

fn folder_row(column_index: usize, item: &FolderItem, row_index: usize, is_selected: bool, is_focused: bool) -> Element<'_, GuiMessage> {
    use iced::widget::mouse_area;

    let label = item.title.clone();
    // Route the click through `FolderRowClick` so the dispatcher can
    // pin focus to this column AND set the row's selection BEFORE
    // navigating. Without that step `push_column` truncates relative
    // to a stale focused column and the Miller stack ends up with
    // siblings of two different parents alive at the same time.
    let is_folder = matches!(item.item_type, FolderItemType::Folder);
    let msg = GuiMessage::FolderRowClick { column_index, row_index, is_folder };
    let btn = button(text(label).size(13))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([4, 8])
        .on_press(msg)
        .style(move |theme: &Theme, status| {
            let palette = theme.extended_palette();
            let (bg, fg) = if is_selected && is_focused {
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
        });

    // Right-click → context menu. Tracks get a full menu (Play /
    // Show Artist Bio / Open in Library); folder rows currently
    // skip the menu — there's no useful per-folder action that
    // isn't already a single click away.
    let is_track = matches!(item.item_type, FolderItemType::Track);
    if is_track {
        mouse_area(btn)
            .on_right_press(GuiMessage::OpenFolderContextMenu { row_index })
            .into()
    } else {
        btn.into()
    }
}

