//! Browse view — category column (left) + Miller columns for the active
//! category (right).
//!
//! Mirrors the TUI's `render_browse` layout (src/ui/app.rs:141): the
//! leftmost column is the category selector (Library / Playlists / Genres /
//! Folders), and the remaining columns are Miller columns for the active
//! category's navigation state.

use iced::widget::{button, column as iced_column, container, image, scrollable, Column, Row, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{NavigationAction, QueueAction};
use crate::app::state::BrowseCategory;
use crate::app::{Action, AppState};
use crate::plex::models::{FolderColumn, FolderItem, FolderItemType, Track};
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::{miller_column, text};
use crate::ui_gui::widgets::transport_bar::primary_action_button;

const ALPHABET_STRIP_WIDTH: f32 = 30.0;
const ALPHABET_STRIP_FONT: f32 = 15.0;

/// Minimum width a Miller column is allowed to occupy before the layout
/// sheds an older column off the left edge. Chosen to stay readable —
/// artists like "The Brian Jonestown Massacre" truncate past ~14 chars
/// below this width.
const MIN_MILLER_COL_WIDTH: f32 = 260.0;

/// Floor on per-column width in scrolling mode. Mirrors the
/// `MIN_COL_WIDTH` floor in the TUI ribbon math — windows smaller
/// than `2 × min` would render unusably narrow columns. Above this
/// floor each column gets exactly half the viewport so two columns
/// fit on screen, matching the TUI's `RIBBON_VISIBLE = 2` rule.
const SCROLL_MILLER_COL_MIN: f32 = 280.0;

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
    let details_track = focused_track(state);

    // Scrolling Miller mode: the entire body becomes a single
    // horizontal ribbon — sections col, every miller col, and the
    // track-details pane all sized to half the viewport, all
    // scrolling together. The alphabet strip is embedded inside the
    // artist column's slot (sharing its half-screen width) so it
    // scrolls with the artist col it belongs to instead of as
    // separate chrome.
    if state.miller_layout == crate::app::state::MillerLayoutMode::Scrolling {
        return view_scrolling(
            state,
            viewport_width_logical,
            scroll_info,
            track_pane_similar,
            show_strip,
            artist_descending,
            details_track,
        );
    }

    let strip_reserved = if show_strip { ALPHABET_STRIP_WIDTH + 4.0 } else { 0.0 };
    // Equal-width column model: cat col, every miller col, and the
    // track-details pane all get the same proportion of the row.
    // Strip is fixed-width chrome. For the visible-window math we
    // reserve MIN_MILLER_COL_WIDTH per non-miller slot (cat + pane)
    // and let `compute_visible_window` cap miller cols against
    // what's left.
    let n_pane: usize = if details_track.is_some() { 1 } else { 0 };
    let reserved_non_miller = (1 + n_pane) as f32 * MIN_MILLER_COL_WIDTH;
    let content_width = (viewport_width_logical - strip_reserved - reserved_non_miller - 16.0)
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

/// Scrolling-mode body: every browse element (sections col, every
/// miller col, the track-details pane) sits at a fixed `col_w =
/// viewport / 2` slot inside one horizontal scrollable. The alphabet
/// strip is embedded inside the artist col's slot rather than living
/// in its own slot, so the strip scrolls with the artist col it
/// belongs to. Mirrors the TUI's Niri-style ribbon layout.
fn view_scrolling<'a>(
    state: &'a AppState,
    viewport_width_logical: f32,
    scroll_info: impl Fn(usize) -> (f32, f32) + Copy + 'a,
    track_pane_similar: &'a std::collections::HashMap<String, Vec<Track>>,
    show_strip: bool,
    artist_descending: bool,
    details_track: Option<&'a Track>,
) -> Element<'a, GuiMessage> {
    use iced::widget::scrollable;
    // Reserve a sliver for the horizontal scrollbar and a touch of
    // breathing room at the right; everything else is split in half.
    let col_w = ((viewport_width_logical - 20.0) / 2.0).max(SCROLL_MILLER_COL_MIN);

    // Slot 0: sections column.
    let mut slots: Vec<Element<'a, GuiMessage>> = vec![
        container(category_column(state))
            .width(Length::Fixed(col_w))
            .height(Length::Fill)
            .into(),
    ];

    // Slots 1..N: miller columns of the active nav. The first slot
    // (Library + artist col 0) hosts the embedded alphabet strip.
    if let Some(nav) = state.browse_nav() {
        let column_offset = if state.browse_category == BrowseCategory::Playlists { 1 } else { 0 };
        if nav.columns.len() > column_offset {
            let other_owns_focus = state.category_column_focused || state.track_pane_focused;
            let focused_logical = nav.focused_column.saturating_sub(column_offset);
            let focused = if other_owns_focus { usize::MAX } else { focused_logical };

            let live_query: Option<&str> = if state.list_filter.active
                && state.list_filter.category == state.browse_category
                && !state.list_filter.query.trim().is_empty()
            {
                Some(state.list_filter.query.trim())
            } else {
                None
            };
            let n_visible_cols = nav.columns.len() - column_offset;
            let mut column_matches: Vec<Option<Vec<usize>>> = if let Some(q) = live_query {
                use crate::services::{filter_with_priority, DEFAULT_MAX_RESULTS};
                (0..n_visible_cols).map(|logical_idx| {
                    let abs_idx = logical_idx + column_offset;
                    let col = &nav.columns[abs_idx];
                    let r = filter_with_priority(&col.items, q, |it| it.title(), DEFAULT_MAX_RESULTS);
                    Some(r.matched_indices)
                }).collect()
            } else {
                (0..n_visible_cols).map(|_| None).collect()
            };

            let grid_cache = &state.artwork.grid_cache;
            for logical_idx in 0..n_visible_cols {
                let abs_idx = logical_idx + column_offset;
                let col = &nav.columns[abs_idx];
                let filter_matched: Option<Vec<usize>> = column_matches[logical_idx].take();
                let (scroll_y, vp_h) = scroll_info(abs_idx);

                // Strip is embedded with the artist col 0 on Library:
                // the strip steals ALPHABET_STRIP_WIDTH from the slot,
                // leaving the rest for the miller column itself.
                let embed_strip = show_strip
                    && state.browse_category == BrowseCategory::Library
                    && logical_idx == 0;
                let inner_col_w = if embed_strip {
                    (col_w - ALPHABET_STRIP_WIDTH - 4.0).max(80.0)
                } else {
                    col_w
                };

                let miller = miller_column::view(
                    abs_idx,
                    col,
                    logical_idx == focused,
                    grid_cache,
                    filter_matched,
                    scroll_y,
                    vp_h,
                    inner_col_w,
                    |click| GuiMessage::MillerSelect {
                        column_index: click.column_index,
                        item_index: click.item_index,
                        activate: click.activate,
                    },
                );

                let slot: Element<'a, GuiMessage> = if embed_strip {
                    let row = Row::new()
                        .spacing(4)
                        .height(Length::Fill)
                        .push(alphabet_strip(artist_descending))
                        .push(
                            container(miller)
                                .width(Length::Fill)
                                .height(Length::Fill),
                        );
                    container(row)
                        .width(Length::Fixed(col_w))
                        .height(Length::Fill)
                        .into()
                } else {
                    container(miller)
                        .width(Length::Fixed(col_w))
                        .height(Length::Fill)
                        .into()
                };
                slots.push(slot);
            }
        }
    }

    // Last slot: track-details pane, when a track is focused.
    if let Some(track) = details_track {
        slots.push(
            container(track_details_pane(track, state, track_pane_similar))
                .width(Length::Fixed(col_w))
                .height(Length::Fill)
                .into(),
        );
    }

    let row = Row::with_children(slots)
        .spacing(4)
        .height(Length::Fill);

    let scroller = scrollable(row)
        .id(browse_h_scroll_id())
        .direction(crate::ui_gui::widgets::fat_horizontal_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .width(Length::Fill)
        .height(Length::Fill);

    container(scroller)
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
    // Order matches `helpers::ALPHABET_STRIP_LETTERS`: % → 0 → a..z → 文.
    // `'文'` is the bucket for non-ASCII first characters (CJK, Cyrillic,
    // Greek, etc.), which sort after `z` in code-point order.
    let mut chars: Vec<char> = std::iter::once('%')
        .chain(std::iter::once('0'))
        .chain('a'..='z')
        .chain(std::iter::once('文'))
        .collect();
    if descending {
        // Sort-descending puts Z-named artists at the top of the
        // list, so the alphabet strip should mirror that order:
        // 文 → Z…A, then 9…0, then % at the bottom (the last sort
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
            // Canonical bucket key in `ALPHABET_STRIP_LETTERS` is `'文'`,
            // but render it as `Ω` to match the TUI strip (which can't
            // fit a double-width CJK glyph in one cell). Greek omega
            // reads as a clear "everything else" marker in either
            // front-end.
            '文' => "Ω".to_string(),
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
fn similar_row<'a>(track: &'a Track, pane_idx: usize, is_selected: bool) -> Element<'a, GuiMessage> {
    use iced::widget::{button, mouse_area};
    let label = format!("{} \u{2014} {}", track.title, track.track_artist());
    // First click selects the row; a second click on the same row
    // (or pressing Enter on it) is what triggers OpenInLibrary —
    // matches the rule used everywhere else in the app. The handler
    // for `SimilarRowClick` lives in `App::update`.
    let body = button(text(label).size(14))
        .width(Length::Fill)
        .padding([3, 8])
        .on_press(GuiMessage::SimilarRowClick { pane_index: pane_idx })
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = if is_selected {
                (p.primary.strong.color, p.primary.strong.text)
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
        });
    // Right-click → standard track context menu, but the "floating"
    // variant — similar-track rows are by definition outside any
    // library drill, so "Open in Library" must appear near the top
    // regardless of the active view category.
    mouse_area(body)
        .on_right_press(GuiMessage::OpenFloatingTrackContextMenu(Box::new(track.clone())))
        .into()
}

/// The track to feed into the details pane: the currently selected row
/// of the focused Miller column, but only if that row is actually a
/// `BrowseItem::Track`. Returns `None` for artist/album/genre/playlist
/// rows, which keeps the pane hidden whenever the user is navigating
/// in a non-track column. Honors the Ctrl+W "hide pane" suppression
/// via `AppState::pane_track`.
fn focused_track(state: &AppState) -> Option<&Track> {
    state.pane_track()
}

fn track_details_pane<'a>(
    track: &'a Track,
    state: &'a AppState,
    track_pane_similar: &'a std::collections::HashMap<String, Vec<Track>>,
) -> Element<'a, GuiMessage> {
    use crate::ui_gui::images::lookup_grid;

    // Pane has its own close-x affordance (Cmd+W keyboard equivalent)
    // — clicking it flips `track_pane_open` to false so the pane
    // disappears without disturbing Miller-column focus or selection.
    // Play Track button sits between the title and the close-x.
    let track_for_play = track.clone();
    let header = container(
        Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(text(" track ").size(14))
            .push(Space::with_width(Length::Fill))
            .push(primary_action_button(
                "Play Track",
                GuiMessage::Action(Action::Queue(QueueAction::PlayTrack(track_for_play))),
            ))
            .push(crate::ui_gui::widgets::miller_column::close_x_button(
                GuiMessage::CloseMillerColumn { column_index: None },
            )),
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
        None => container(text("(no cover)").size(13))
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

    let title = text(track.title.clone()).size(18);
    let artist = text(track.track_artist().to_string()).size(15);
    let album_year = match (track.parent_title.as_deref(), track.year) {
        (Some(a), Some(y)) => format!("{a}  ({y})"),
        (Some(a), None)    => a.to_string(),
        (None, Some(y))    => y.to_string(),
        (None, None)       => String::new(),
    };
    let album = text(album_year).size(14);
    let duration = {
        let total = track.duration_ms();
        let m = total / 60_000;
        let s = (total / 1000) % 60;
        text(format!("Duration: {m}:{s:02}")).size(14)
    };
    let track_no = track.index.map(|n| text(format!("Track #{n}")).size(14));

    // File path: pulled from the first MediaPart. Show only the
    // basename so the column doesn't have to be widened for long
    // server-side paths; the full path is in the tooltip-equivalent
    // cached data (dispatch can look it up if needed later).
    let filename: Option<Element<'_, GuiMessage>> = track
        .stream_part()
        .and_then(|p| p.file.as_deref())
        .map(|path| {
            let basename = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
            text(format!("File: {basename}")).size(13).into()
        });

    let mut info_col = Column::new()
        .spacing(6)
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
        col = col.push(text("Sonically Similar").size(15));
        match track_pane_similar.get(&track.rating_key) {
            Some(list) if list.is_empty() => col
                .push(text("(no similar tracks found)").size(13))
                .into(),
            Some(list) => {
                for (i, sim) in list.iter().enumerate() {
                    let pane_idx = i + 1; // 0 reserved for the Play button
                    let is_selected = state.track_pane_focused
                        && state.track_pane_index == pane_idx;
                    col = col.push(similar_row(sim, pane_idx, is_selected));
                }
                col.into()
            }
            None => col.push(text("Loading\u{2026}").size(13)).into(),
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

    let pane_focused = state.track_pane_focused;
    let chrome = container(iced_column![header, body].spacing(0))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |theme: &Theme| {
            let p = theme.extended_palette();
            // Mirror the focused-column accent the miller widget
            // applies (`primary.base.color`) so the user can see at
            // a glance which surface is focused. Single-focus rule:
            // exactly one column carries the accent border at any
            // moment.
            let border_color = if pane_focused {
                p.primary.base.color
            } else {
                p.background.strong.color
            };
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                text_color: Some(p.background.base.text),
                border: Border { color: border_color, width: 1.0, radius: 0.0.into() },
                ..container::Style::default()
            }
        });
    // mouse_area on the chrome claims focus when the user clicks
    // anywhere inside the pane that isn't itself a button — so the
    // pane behaves like a real column for focus purposes. The header
    // X / Play Track / similar-row buttons keep their own messages.
    // Wrap in a FillPortion(1) container so the pane gets the same
    // share of horizontal space as every other miller column slot.
    container(
        iced::widget::mouse_area(chrome)
            .on_press(GuiMessage::FocusTrackPane),
    )
    .width(Length::FillPortion(1))
    .height(Length::Fill)
    .into()
}

fn category_column(state: &AppState) -> Element<'_, GuiMessage> {
    let is_focused = state.category_column_focused;
    // Build the visible category list: every BrowseCategory that
    // isn't hidden via Settings, except Playlists (which is rendered
    // individually as one row per playlist below the divider).
    let visible_cats: Vec<(BrowseCategory, &'static str)> = BrowseCategory::all().iter()
        .filter(|c| **c != BrowseCategory::Playlists && !state.hidden_sections.contains(c))
        .map(|c| (*c, c.display_label()))
        .collect();
    let active_cat_idx = visible_cats.iter().position(|(c, _)| *c == state.browse_category);

    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
    for (i, (cat, label)) in visible_cats.iter().copied().enumerate() {
        let is_selected = Some(i) == active_cat_idx
            && cat == state.browse_category
            && state.category_column_focused
            && i == state.category_column_index;
        let highlighted = cat == state.browse_category;
        let row_label = text(label).size(15);

        rows.push(
            button(row_label)
                .width(Length::Fill)
                .padding([4, 8])
                .on_press(GuiMessage::Action(Action::Navigation(
                    NavigationAction::set_category(cat),
                )))
                .style(move |theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    // The B&W theme collapses every "soft" pair to the
                    // same white-on-white as the body, so the
                    // background.weak highlight used here would be
                    // invisible. Use the strong (black-on-white) pair
                    // instead so the active category row reads as a
                    // pressed-in chip.
                    let is_bw = crate::ui_gui::theme::is_monochrome(theme);
                    let (bg, fg) = if is_selected {
                        (p.primary.strong.color, p.primary.strong.text)
                    } else if highlighted {
                        if is_bw {
                            (p.background.strong.color, p.background.strong.text)
                        } else {
                            (p.background.weak.color, p.background.base.text)
                        }
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

    // Per-playlist rows. The pin-and-filter logic lives in
    // `AppState::category_rows()` so the TUI and GUI agree on which
    // playlists appear and in what order; here we just slice the
    // playlist part out and walk it.
    let cat_rows = state.category_rows();
    let pinned_keys = ['\u{2764}', '\u{2661}', '\u{1f90d}', '\u{2665}'];
    let divider_row = || -> Element<'_, GuiMessage> {
        container(Space::with_height(Length::Fixed(1.0)))
            .padding([6, 8])
            .width(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                ..container::Style::default()
            })
            .into()
    };
    if !state.hidden_sections.contains(&BrowseCategory::Playlists) {
        // Divider between categories block and playlists block.
        let has_any_playlist = cat_rows.iter()
            .any(|r| matches!(r, crate::app::state::CategoryRow::Playlist(_)));
        if !visible_cats.is_empty() && has_any_playlist {
            rows.push(divider_row());
        }
        let mut prev_was_pinned: Option<bool> = None;
        for r in &cat_rows {
            let i = match r {
                crate::app::state::CategoryRow::Playlist(i) => *i,
                _ => continue,
            };
            let Some(p) = state.library.playlists.get(i) else { continue };
            let is_pinned = p.title.eq_ignore_ascii_case("recently added")
                || pinned_keys.iter().any(|k| p.title.contains(*k));
            // Insert a divider when transitioning from pinned to unpinned.
            if let Some(prev) = prev_was_pinned {
                if prev && !is_pinned {
                    rows.push(divider_row());
                }
            }
            prev_was_pinned = Some(is_pinned);
            let title = p.title.clone();
            let pkey = p.rating_key.clone();
            let is_active = state.browse_category == BrowseCategory::Playlists
                && state.playlist_nav.columns
                    .first()
                    .and_then(|c| c.items.get(c.selected_index))
                    .map(|it| it.key() == pkey.as_str())
                    .unwrap_or(false);
            // Force text-presentation for heart glyphs so the playlist
            // row paints in the body text color, not the colorful emoji.
            let display = crate::util::force_text_presentation(&title);
            let row_label = text(crate::ui_gui::widgets::safe_text(&display).into_owned()).size(15);
            rows.push(
                button(row_label)
                    .width(Length::Fill)
                    .padding([3, 8])
                    .on_press(GuiMessage::OpenPlaylistFromCategory {
                        playlist_key: pkey,
                        title,
                    })
                    .style(move |theme: &Theme, status: button::Status| {
                        let p = theme.extended_palette();
                        // Same B&W flip as the category rows above:
                        // background.weak is white-on-white in that
                        // theme so we need the strong pair to make
                        // the active playlist row visible at all.
                        let is_bw = crate::ui_gui::theme::is_monochrome(theme);
                        let (bg, fg) = if is_active {
                            if is_bw {
                                (p.background.strong.color, p.background.strong.text)
                            } else {
                                (p.background.weak.color, p.background.base.text)
                            }
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
    }

    let list = scrollable(Column::with_children(rows).spacing(0))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill);

    // Container chrome matches `miller_column::wrap_chrome` so the
    // category column draws the same border + bg as a regular Miller
    // column. No header row — the category names are self-evident.
    container(list)
        .width(Length::FillPortion(1))
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
    // Note: scrolling Miller mode is handled at the top-level `view`
    // function, which builds the entire body as one horizontal
    // ribbon — this path is never entered in that mode.
    if state.browse_category == BrowseCategory::Folders {
        return folder_columns(state, content_width);
    }

    let grid_cache = &state.artwork.grid_cache;
    if let Some(nav) = state.browse_nav() {
        if nav.columns.is_empty() {
            return container(text("Loading\u{2026}").size(16))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        // Playlists are now listed individually in the leftmost
        // category column, so playlist_nav.columns[0] (the root
        // "playlists" list) is a duplicate of the category column
        // and we skip it. The same nav still drives selection /
        // drilling — we just hide the redundant root from the
        // Miller-column area.
        let column_offset = if state.browse_category == BrowseCategory::Playlists { 1 } else { 0 };
        if nav.columns.len() <= column_offset {
            // Either the user just clicked a playlist (load in
            // flight) or no playlist is selected yet. Show a
            // loading indicator vs. the "pick a playlist" prompt
            // accordingly.
            let msg = if nav.loading {
                "Loading\u{2026}"
            } else {
                "Pick a playlist on the left."
            };
            return container(text(msg).size(16))
                .padding(24)
                .width(Length::FillPortion(1))
                .height(Length::Fill)
                .into();
        }
        let visible_total = nav.columns.len() - column_offset;
        let focused_logical = nav.focused_column.saturating_sub(column_offset);
        // Single-focus rule: only one of {cat col, a miller col, the
        // track-details pane} paints as focused at a time. usize::MAX
        // is the "no miller col is focused" sentinel; we use it both
        // when the cat col owns focus AND when the pane does.
        let other_owns_focus = state.category_column_focused || state.track_pane_focused;
        let focused = if other_owns_focus { usize::MAX } else { focused_logical };
        let (start, end, col_width) = compute_visible_window(
            visible_total,
            focused_logical,
            content_width,
        );

        // Quick-filter (transport-bar text input) narrows EVERY visible
        // Miller column on-the-fly. Each column gets its own match-index
        // list — we run the same priority filter over its items here in
        // the render pass instead of relying on the single
        // `state.list_filter.results` precomputed for one specific
        // column. Pressing Enter in the filter input opens the global
        // Search popup (which clears the input as a side effect).
        let live_query: Option<&str> = if state.list_filter.active
            && state.list_filter.category == state.browse_category
            && !state.list_filter.query.trim().is_empty()
        {
            Some(state.list_filter.query.trim())
        } else {
            None
        };

        // Match-index storage. miller_column::view takes ownership of
        // the Vec so we don't have to keep a borrow alive across the
        // returned Element's lifetime. `matches[i]` is for visible
        // column `i` (logical index — already excludes the skipped
        // root column).
        let mut column_matches: Vec<Option<Vec<usize>>> = if let Some(q) = live_query {
            use crate::services::{filter_with_priority, DEFAULT_MAX_RESULTS};
            (start..end).map(|logical_idx| {
                let abs_idx = logical_idx + column_offset;
                let col = &nav.columns[abs_idx];
                let r = filter_with_priority(&col.items, q, |it| it.title(), DEFAULT_MAX_RESULTS);
                Some(r.matched_indices)
            }).collect()
        } else {
            (start..end).map(|_| None).collect()
        };

        let cols = (start..end).map(|logical_idx| {
            let abs_idx = logical_idx + column_offset;
            let col = &nav.columns[abs_idx];
            let local_idx = logical_idx - start;
            let filter_matched: Option<Vec<usize>> = column_matches[local_idx].take();
            let (scroll_y, vp_h) = scroll_info(abs_idx);
            let element = miller_column::view(abs_idx, col, logical_idx == focused, grid_cache, filter_matched, scroll_y, vp_h, col_width, |click| GuiMessage::MillerSelect {
                column_index: click.column_index,
                item_index: click.item_index,
                activate: click.activate,
            });
            // Equal-width: every visible miller col gets the same
            // proportion of the row's space (FillPortion(1) each).
            // The outer Row's width is FillPortion(n_visible) so
            // each col here ends up matching cat / pane widths.
            let element: Element<'_, GuiMessage> =
                container(element).width(Length::FillPortion(1)).height(Length::Fill).into();
            element
        }).collect::<Vec<_>>();
        let visible_n = end.saturating_sub(start).max(1) as u16;
        Row::with_children(cols)
            .spacing(4)
            .width(Length::FillPortion(visible_n))
            .height(Length::Fill)
            .into()
    } else {
        container(text("Loading\u{2026}").size(16))
            .padding(24)
            .width(Length::FillPortion(1))
            .height(Length::Fill)
            .into()
    }
}

/// Stable scrollable Id for the horizontal Miller-column ribbon, so
/// `App::update` can `snap_to` the right edge whenever a new column
/// drills in or focus moves — keeping the focused column pinned to
/// the right of the viewport.
pub fn browse_h_scroll_id() -> iced::widget::scrollable::Id {
    use std::sync::OnceLock;
    static ID: OnceLock<iced::widget::scrollable::Id> = OnceLock::new();
    ID.get_or_init(|| iced::widget::scrollable::Id::new("browse-h-scroll")).clone()
}

fn folder_columns(state: &AppState, content_width: f32) -> Element<'_, GuiMessage> {
    let fs = match state.folder_state.as_ref() {
        Some(fs) => fs,
        None => {
            return container(text("Loading folders\u{2026}").size(16))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
    };
    if fs.columns.is_empty() {
        return container(text("No folders").size(16))
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let other_owns_focus = state.category_column_focused || state.track_pane_focused;
    let focused = if other_owns_focus { usize::MAX } else { fs.focused_column };
    let (start, end, col_width) = compute_visible_window(
        fs.columns.len(),
        fs.focused_column,
        content_width,
    );
    // Quick-filter (transport-bar text input) narrows every visible
    // folder column on-the-fly — same behaviour as the Library /
    // Genres / Playlists Miller columns.
    let live_query: Option<&str> = if state.list_filter.active
        && state.list_filter.category == BrowseCategory::Folders
        && !state.list_filter.query.trim().is_empty()
    {
        Some(state.list_filter.query.trim())
    } else {
        None
    };
    let cols = (start..end).map(|idx| {
        let col = &fs.columns[idx];
        let filter_matched: Option<Vec<usize>> = live_query.map(|q| {
            use crate::services::{filter_with_priority, DEFAULT_MAX_RESULTS};
            filter_with_priority(&col.items, q, |it| it.title.as_str(), DEFAULT_MAX_RESULTS)
                .matched_indices
        });
        let element = folder_column_view(idx, col, idx == focused, filter_matched);
        let element: Element<'_, GuiMessage> =
            container(element).width(Length::FillPortion(1)).height(Length::Fill).into();
        element
    }).collect::<Vec<_>>();
    let visible_n = end.saturating_sub(start).max(1) as u16;
    Row::with_children(cols)
        .spacing(4)
        .width(Length::FillPortion(visible_n))
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


fn folder_column_view(column_index: usize, col: &FolderColumn, is_focused: bool, filter_matched: Option<Vec<usize>>) -> Element<'_, GuiMessage> {
    const FOLDER_ROW_H: f32 = 26.0;

    let header = container(
        Row::new()
            .spacing(4)
            .align_y(Alignment::Center)
            .push(text(&col.title).size(14).width(Length::Fill))
            .push(crate::ui_gui::widgets::miller_column::close_x_button(
                GuiMessage::CloseMillerColumn { column_index: Some(column_index) },
            )),
    )
    .padding([4, 8])
    .width(Length::Fill);

    let selected = col.selected_index;
    let filter_active_no_matches = matches!(&filter_matched, Some(m) if m.is_empty());
    // Pin each row to a fixed pixel height so Iced's layout pass skips
    // cosmic-text measurement for every item. A 10k-entry folder list
    // was the worst offender before this — layout alone took ~300 ms
    // per frame.
    let indices: Vec<usize> = match filter_matched {
        Some(matched) => matched,
        None => (0..col.items.len()).collect(),
    };
    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::with_capacity(indices.len() + 1);
    if filter_active_no_matches {
        rows.push(
            container(text("no results").size(15))
                .padding([6, 12])
                .width(Length::Fill)
                .into(),
        );
    }
    for &i in &indices {
        let Some(it) = col.items.get(i) else { continue };
        rows.push(
            container(folder_row(column_index, it, i, i == selected, is_focused))
                .height(Length::Fixed(FOLDER_ROW_H))
                .width(Length::Fill)
                .into()
        );
    }

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
    let btn = button(text(label).size(15))
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

