//! Root UI rendering (musikcube-style).
//!
//! Layout:
//! ┌──────────────────────────────────────────────────────────────┐
//! │ ┌─ artists ─────────┬─ track list ─────────────────────────┐ │
//! │ │ Artist 1          │ Album Header                         │ │
//! │ │ Artist 2          │   1  Track Name        4:32  Artist  │ │
//! │ │ > Artist 3        │   2  Track Name        3:21  Artist  │ │
//! │ │ Artist 4          │ Album Header                         │ │
//! │ │                   │   1  Track Name        5:02  Artist  │ │
//! │ └───────────────────┴──────────────────────────────────────┘ │
//! ├──────────────────────────────────────────────────────────────┤
//! │ playing Track Name by Artist from Album       vol ─■── 80%   │
//! ├──────────────────────────────────────────────────────────────┤
//! │ ^A artists │ ^P playlists │ ^N queue │ ^S similar │ ? │
//! └──────────────────────────────────────────────────────────────┘

use std::cell::RefCell;

use crate::app::state::{View, BrowseCategory, InputDialog, ConfirmDialog};
use crate::app::AppState;
use crate::services::NavigationService;
use super::artwork::ArtworkRenderer;
use super::layout::{AppLayout, FullScreenLayout, centered_rect};
use super::screens;
use super::theme::theme;
use super::widgets;
use super::widgets::scrollbar::render_scrollbar;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

thread_local! {
    static BIO_ARTWORK_RENDERER: RefCell<ArtworkRenderer> = RefCell::new(ArtworkRenderer::new());
}

/// Initialize the bio popup artwork renderer with a pre-detected picker.
/// Must be called before the event reader task starts consuming stdin.
pub fn init_bio_artwork_renderer(picker: ratatui_image::picker::Picker) {
    BIO_ARTWORK_RENDERER.with(|r| {
        *r.borrow_mut() = ArtworkRenderer::new_with_picker(picker);
    });
}

/// Set the bio artwork renderer mode.
pub fn set_bio_artwork_mode(mode: crate::app::state::ArtworkMode) {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().set_mode(mode));
}

/// Set the bio artwork renderer protocol type.
pub fn set_bio_artwork_protocol_type(protocol_type: ratatui_image::picker::ProtocolType) {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().set_protocol_type(protocol_type));
}

/// Restore the bio artwork renderer's native protocol.
pub fn restore_bio_artwork_native_protocol() {
    BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().restore_native_protocol());
}

/// Render the entire UI.
pub fn render(frame: &mut Frame, state: &AppState) {
    // Clear hit-test registry for this frame
    state.hit_regions.borrow_mut().clear();

    // Fill entire background with theme color
    let t = theme();
    frame.render_widget(Block::default().style(Style::default().bg(t.colors.bg_primary)), frame.area());

    // Tall mode: split the frame vertically — Library on top half,
    // Now Playing on bottom half. Only applies when the user is on a
    // view that would normally be one of those two; popups / Help /
    // Settings / Auth still render full-screen because their layouts
    // assume the whole frame.
    let tall_eligible = matches!(
        state.view,
        View::Browse | View::Queue | View::NowPlaying | View::Settings | View::Help
    );
    if state.tall_mode && tall_eligible {
        use ratatui::layout::{Constraint, Direction, Layout};
        // Tall split is 40 / 60 — Library on top (40%), Now Playing
        // on bottom (60%). Now-playing gets the larger share because
        // its visualizer / queue / artwork need vertical room more
        // than the library's Miller columns do (which use horizontal
        // ribbon scrolling). 1-row separator between halves; only
        // the bottom half paints the transport bar.
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Length(1),       // separator row
                Constraint::Min(5),          // bottom half (60% minus 1 row)
            ])
            .split(frame.area());

        // Register the split for mouse hit-testing — clicks crossing
        // the separator switch the active view (top = Browse, bottom
        // = NowPlaying).
        state.hit_regions.borrow_mut().tall_mode_split = Some(crate::ui::hit_regions::TallModeSplit {
            top: split[0],
            bottom: split[2],
        });

        // Top half: whichever view the user is on. Settings / Help
        // get the top half so the Now Playing visualizer stays visible
        // below; Browse / Queue / NowPlaying behave as before.
        match state.view {
            View::Settings => screens::settings::render(frame, state, split[0]),
            View::Help => screens::help::render(frame, state, split[0]),
            _ => render_browse_in(frame, state, split[0], true),
        }

        // Separator: a horizontal rule across the full width.
        let sep_text = "\u{2500}".repeat(split[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep_text).style(Style::default().fg(t.colors.border)),
            split[1],
        );
        render_queue_and_visualizer_in(frame, state, split[2], false);
    } else {
        match state.view {
            View::Auth => render_auth(frame, state),
            View::Browse => render_browse(frame, state),
            View::Queue => render_queue_and_visualizer(frame, state),
            View::NowPlaying => render_queue_and_visualizer(frame, state),
            View::Search => render_search(frame, state),
            View::Similar => render_similar(frame, state),
            View::Related => render_related(frame, state),
            View::Help => render_help(frame, state),
            View::Settings => render_settings(frame, state),
        }
    }

    // Render search popup if active (floating dialog)
    if state.popups.search_active {
        screens::filter::render(frame, state, frame.area());
    }

    // Render radio launcher popup if active
    if state.popups.radio_launcher.is_some() {
        screens::radio_launcher::render(frame, state, frame.area());
    }

    // Render adventure launcher popup if active
    if state.popups.adventure_launcher.is_some() {
        screens::adventure_launcher::render(frame, state, frame.area());
    }

    // Render artist radio picker popup if active
    if state.popups.artist_radio_picker.is_some() {
        screens::artist_radio_picker::render(frame, state, frame.area());
    }

    // Render library picker popup if active
    if state.popups.library_picker_active {
        render_library_picker(frame, state);
    }

    // Render sort popup if active
    if state.popups.sort.is_some() {
        screens::sort_popup::render(frame, state, frame.area());
    }

    // Render artist bio popup if active. When it's NOT active but the
    // renderer is still holding an image, drop the protocol so the
    // kitty/iterm2 delete-image command fires on the next terminal
    // write — otherwise the artwork (and the bio text the diff treats
    // as `skip` beneath it) lingers after every non-Esc dismissal
    // path (e.g. `popups.close_all()` when another popup opens).
    if state.popups.artist_bio.is_some() {
        render_artist_bio_popup(frame, state);
    } else if BIO_ARTWORK_RENDERER.with(|r| r.borrow().has_image()) {
        BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().clear());
    }

    // Render error popup if present
    if let Some(ref error) = state.notifications.last_error {
        render_error_popup(frame, error);
    }

    // Render input dialog if present
    if let Some(ref dialog) = state.popups.input_dialog {
        render_input_dialog(frame, dialog);
    }

    // Render confirm dialog if present
    if let Some(ref dialog) = state.popups.confirm_dialog {
        render_confirm_dialog(frame, state, dialog);
    }

    // Render toast notification if present (bottom-right, non-blocking)
    if let Some(ref toast) = state.notifications.toast_message {
        render_toast(frame, toast, frame.area());
    }

    // Command-palette overlay rides ABOVE every other layer so the
    // user can summon it from any view (including over a popup).
    if state.palette.open {
        crate::ui::command_palette::render(frame, state, frame.area());
    }
}

fn render_auth(frame: &mut Frame, state: &AppState) {
    screens::auth::render(frame, state, frame.area());
}

fn render_browse(frame: &mut Frame, state: &AppState) {
    render_browse_in(frame, state, frame.area(), false);
}

fn render_browse_in(frame: &mut Frame, state: &AppState, area: Rect, skip_transport: bool) {
    use crate::app::state::BrowseCategory;

    // In tall mode the top half doesn't paint its own transport bar
    // — give those 2 rows back to the content so the column stack
    // gets to use them, and let the caller paint a separator.
    let layout = if skip_transport {
        AppLayout::without_transport(area)
    } else {
        AppLayout::new(area)
    };

    // Full area for all columns (combine left + right panels)
    let full_area = Rect {
        x: layout.left_panel.x,
        y: layout.left_panel.y,
        width: layout.left_panel.width + layout.right_panel.width,
        height: layout.left_panel.height,
    };

    // Virtual column model — visible-only space:
    //   virtual 0 = category selector column
    //   virtual 1..N = meaningful content columns (skips the
    //   `column_offset` prefix that's hidden by contract, e.g.
    //   Playlists hides nav col 0 because the playlists are
    //   already in the cat col).
    let column_offset_for_focus: usize = match state.browse_category {
        BrowseCategory::Playlists => 1,
        _ => 0,
    };
    let content_focused = if state.category_column_focused {
        0
    } else {
        match state.browse_category {
            BrowseCategory::Folders => state.folder_state.as_ref().map_or(0, |fs| fs.focused_column),
            _ => state.browse_nav().map_or(0, |nav| nav.focused_column),
        }
    };
    // Project raw nav.focused_column into visible space by
    // subtracting the hidden prefix. Without this, focusing a
    // Playlist tracks column (raw idx 1, visible idx 0) would
    // make the sliding window think the focus is one slot
    // further right than it really is, sliding the cat col off
    // the screen.
    let virtual_focus = if state.category_column_focused {
        0
    } else {
        1 + content_focused.saturating_sub(column_offset_for_focus)
    };

    // Equal-width column layout. Every visible miller-related
    // column (cat col + each meaningful content miller col) gets
    // the same width: 2 cols → 50/50, 3 → 1/3 each, 4 → quarters,
    // etc. Strip + pane stay as fixed-width chrome that doesn't
    // count toward the column slots. When cols would shrink below
    // MIN_COL_WIDTH the window slides right (focus-anchored),
    // shedding the leftmost slots.
    const MIN_COL_WIDTH: u16 = 18;

    let t = theme();
    let category_rows = state.category_rows();

    let n_meaningful_miller = count_meaningful_miller_cols(state);

    // Track-details pane: counted as a "column slot" so it gets
    // the same width as everything else. Visible whenever the
    // focused Miller row is a Track AND the pane hasn't been
    // hidden via Ctrl+W for that specific track.
    let pane_track: Option<crate::plex::models::Track> = state.pane_track().cloned();
    let n_pane: usize = if pane_track.is_some() { 1 } else { 0 };

    let total_cols_wanted = 1 + n_meaningful_miller + n_pane;

    // Cat col is ALWAYS pinned to the left — minimum 2-col layout
    // (cat + at least one content col) is the rule. When more miller
    // cols + pane exist than fit, the LEFTMOST miller cols slide off
    // (focus-anchored) while the cat stays put. Total wanted slots
    // is at least 2 so even an empty / loading content area still
    // reserves a meaningful col next to cat.
    let strip_wants_to_show = state.alphabet_strip_visible();
    let cat_col_visible = true;
    let resolve_layout = |strip_w: u16| -> usize {
        let usable = full_area.width.saturating_sub(strip_w);
        let max_fit = ((usable / MIN_COL_WIDTH) as usize).max(2);
        total_cols_wanted.max(2).min(max_fit)
    };

    // Niri-style scroll layout: sections col, every Miller col of
    // the active browse category, and the track pane all live in
    // one horizontal ribbon. Each slot is `col_width = full / 2`,
    // two slots fit on screen at a time. The focused slot sits at
    // the right edge; older slots — including the sections col —
    // push off-screen left. The alphabet strip (Library only) is
    // anchored to the root nav slot (ribbon idx 1) and overlays its
    // leftmost cells; when that slot scrolls off, the strip goes
    // with it. Folders are excluded — they use a different render
    // path (`render_folder_view`) so the ribbon model doesn't
    // apply directly there yet.
    let scrolling_browse = state.miller_layout
        == crate::app::state::MillerLayoutMode::Scrolling
        && matches!(
            state.browse_category,
            BrowseCategory::Library | BrowseCategory::Playlists
        ) || (state.miller_layout == crate::app::state::MillerLayoutMode::Scrolling
            && state.browse_category.is_tag_section());

    // Number of leading nav cols the inner Miller renderer always
    // skips for this category (Playlists hides its root col 0
    // because the playlists are listed in the sections column).
    let category_base_offset: usize = match state.browse_category {
        BrowseCategory::Playlists => 1,
        _ => 0,
    };
    let nav_count = state.browse_nav().map(|n| n.columns.len()).unwrap_or(0);
    let visible_nav_cols = nav_count.saturating_sub(category_base_offset);

    const RIBBON_VISIBLE: usize = 2;
    let ribbon_total = if scrolling_browse {
        1 + visible_nav_cols + n_pane
    } else { 0 };
    let ribbon_focused = if scrolling_browse {
        if state.category_column_focused { 0 }
        else if state.track_pane_focused { ribbon_total.saturating_sub(1) }
        else if state.alphabet_strip_focused { 1 }
        else {
            let nav_focused = state.browse_nav().map(|n| n.focused_column).unwrap_or(0);
            1 + nav_focused.saturating_sub(category_base_offset)
        }
    } else { 0 };
    let ribbon_start = if scrolling_browse {
        let focus_anchored = (ribbon_focused + 1).saturating_sub(RIBBON_VISIBLE);
        if state.miller_scroll_manual && ribbon_total > RIBBON_VISIBLE {
            // User has manually positioned the scrollbar. Honour
            // `miller_scroll_col` until the next keystroke clears
            // the manual flag.
            let max_start = ribbon_total.saturating_sub(RIBBON_VISIBLE);
            state.miller_scroll_col.min(max_start)
        } else {
            focus_anchored
        }
    } else { 0 };
    let ribbon_end = if scrolling_browse {
        (ribbon_start + RIBBON_VISIBLE).min(ribbon_total)
    } else { 0 };
    let scroll_sections_visible = scrolling_browse && ribbon_start == 0;
    let scroll_artists_visible = scrolling_browse && ribbon_start <= 1 && ribbon_end >= 2;

    let strip_width: u16 = if strip_wants_to_show && (!scrolling_browse || scroll_artists_visible) {
        3
    } else {
        0
    };
    let (visible_count, col_width) = if scrolling_browse {
        // Two slots visible, each half the screen.
        let cw = (full_area.width / RIBBON_VISIBLE as u16).max(MIN_COL_WIDTH);
        (RIBBON_VISIBLE, cw)
    } else {
        let vc = resolve_layout(strip_width);
        let usable_width = full_area.width.saturating_sub(strip_width);
        let cw = (usable_width / vc as u16).max(MIN_COL_WIDTH);
        (vc, cw)
    };
    let _virtual_start: usize = 0;
    let _ = virtual_focus;

    let show_alphabet_strip = strip_width > 0;

    // In Niri-style scroll mode, the sections col is at ribbon idx 0
    // and disappears when ribbon_start > 0. Suppress its render in
    // that case; the rest of the layout (Miller area, pane, strip)
    // shifts left to claim its slot.
    let cat_col_renders = cat_col_visible && (!scrolling_browse || scroll_sections_visible);

    // Render the category column if visible
    if cat_col_renders {
        let col_area = Rect {
            x: full_area.x,
            y: full_area.y,
            width: col_width,
            height: full_area.height,
        };

        let is_focused = state.category_column_focused;
        let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };
        let block = Block::default()
            .title(" browse ")
            .title_style(Style::default().fg(if is_focused { t.colors.title_focused } else { t.colors.fg_accent }))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        // Register category column hit region
        {
            let mut hr = state.hit_regions.borrow_mut();
            hr.category_column = Some(crate::ui::hit_regions::CategoryColumnRegion {
                area: col_area,
                inner,
                item_count: category_rows.len(),
            });
        }

        // Categories, dividers, and playlists render as one continuous
        // list — the visual y-offset is just the row index.
        for (i, row) in category_rows.iter().enumerate() {
            let y_offset = i as u16;
            if y_offset >= inner.height { break; }

            // Divider rows render a horizontal rule and skip the rest.
            if matches!(row, crate::app::state::CategoryRow::Divider) {
                let sep = "\u{2500}".repeat(inner.width as usize);
                let line_area = Rect { x: inner.x, y: inner.y + y_offset, width: inner.width, height: 1 };
                frame.render_widget(
                    Paragraph::new(sep).style(Style::default().fg(t.colors.border)),
                    line_area,
                );
                continue;
            }

            let (label, is_active) = match row {
                crate::app::state::CategoryRow::Category(cat) => {
                    let label = cat.display_label();
                    let active = state.browse_category == *cat;
                    (label.to_string(), active)
                }
                crate::app::state::CategoryRow::Playlist(idx) => {
                    let p = match state.library.playlists.get(*idx) {
                        Some(p) => p,
                        None => continue,
                    };
                    let active = state.browse_category == BrowseCategory::Playlists
                        && state.playlist_nav.columns.first()
                            .and_then(|c| c.items.get(c.selected_index))
                            .map(|it| it.key() == p.rating_key.as_str())
                            .unwrap_or(false);
                    // Force text-presentation for hearts so the
                    // playlist label paints in the column's foreground
                    // color instead of the colorful emoji glyph.
                    (crate::util::force_text_presentation(&p.title), active)
                }
                crate::app::state::CategoryRow::Divider => unreachable!(),
            };

            let is_selected = i == state.category_column_index;
            let show_cursor = is_focused && is_selected;
            let show_active = !is_focused && is_active;
            // Foreground on selection/highlight rows must use
            // `selection_text`, not `fg_primary` — in the
            // black-and-white theme `fg_primary` is the same colour
            // as `bg_selection`/`bg_highlight`, which makes the row
            // text invisible.
            let style = if show_cursor {
                Style::default().fg(t.colors.selection_text).bg(t.colors.bg_selection)
            } else if show_active {
                Style::default().fg(t.colors.selection_text).bg(t.colors.bg_highlight)
            } else {
                Style::default().fg(t.colors.fg_primary)
            };

            let indicator = if show_cursor { "\u{25b8} " } else { "  " };
            // Mac-style middle truncation when the label is wider
            // than the column. The 2-cell indicator prefix has to
            // fit too — reserve it before truncating.
            let indicator_w = indicator.chars().count();
            let max_label_w = (inner.width as usize).saturating_sub(indicator_w);
            let label_display = if label.chars().count() > max_label_w {
                crate::util::truncate_middle(&label, max_label_w)
            } else {
                label.clone()
            };
            // Pad the row's text to the full inner width so the
            // selection / highlight background fills the row edge to
            // edge — same look as the Miller columns. Without this,
            // ratatui only paints the bg under the literal text cells
            // and the sections column ends up with a thin highlight
            // ribbon while the rest of the app uses chunky bars.
            let text = format!("{}{}", indicator, label_display);
            let display_w = unicode_width::UnicodeWidthStr::width(text.as_str());
            let pad = (inner.width as usize).saturating_sub(display_w);
            let padded = format!("{}{}", text, " ".repeat(pad));
            let line_area = Rect { x: inner.x, y: inner.y + y_offset, width: inner.width, height: 1 };
            frame.render_widget(Paragraph::new(padded).style(style), line_area);
        }
    }

    // Strip rendering in shrinking layout only (between sections and
    // Miller). In Niri scroll mode the strip is painted later, AFTER
    // the Miller cols, as an overlay on the artists slot's leftmost
    // cells.
    if show_alphabet_strip && !scrolling_browse {
        let strip_area = Rect {
            x: full_area.x + col_width,
            y: full_area.y,
            width: strip_width,
            height: full_area.height,
        };
        render_alphabet_strip(frame, state, strip_area);
    }

    let cat_w = if cat_col_renders { col_width } else { 0 };

    // In scroll mode the pane only "claims" a slot when its ribbon
    // index is actually in the visible window; otherwise it's
    // scrolled off and shouldn't push miller cols out of the layout.
    let pane_renders = if scrolling_browse {
        let pane_ribbon_idx = ribbon_total.saturating_sub(1);
        n_pane > 0 && pane_ribbon_idx >= ribbon_start && pane_ribbon_idx < ribbon_end
    } else {
        n_pane > 0
    };
    let pane_w = if pane_renders { col_width } else { 0 };
    let n_visible_miller = visible_count
        .saturating_sub(if cat_col_renders { 1 } else { 0 })
        .saturating_sub(if pane_renders { 1 } else { 0 });
    let miller_w = (n_visible_miller as u16) * col_width;

    // In Niri scroll mode, the Miller area's x position depends on
    // whether the sections col is on screen — when scrolled off, the
    // Miller cols slide left to claim that slot. Strip width offset
    // applies only in shrinking layout (in scrolling mode the strip
    // overlays the artists col's leftmost cells, not its own slot).
    let content_area_x_offset = if scrolling_browse { 0 } else { strip_width };
    let content_area = Rect {
        x: full_area.x + cat_w + content_area_x_offset,
        y: full_area.y,
        width: miller_w,
        height: full_area.height,
    };

    let pane_area = if pane_w > 0 {
        Some(Rect {
            x: content_area.x + miller_w,
            y: full_area.y,
            width: pane_w,
            height: full_area.height,
        })
    } else {
        None
    };


    // Pass content area to existing category renderers.
    // They compute their own internal column layout from the given area.
    // When category column is visible, they get 2/3 of the width (2 content column slots).
    // When it's scrolled off, they get full width (3 content column slots).
    let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

    // When category column is focused, anchor the inner content viewport to
    // column 0 so the root column (e.g. Artists) is always visible next to the
    // category column.
    let content_focus_override = if state.category_column_focused { Some(0) } else { None };

    match state.browse_category {
        BrowseCategory::Library => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Library {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            // In Niri scroll mode, when the sections col (ribbon
            // idx 0) and possibly the artists col (ribbon idx 1)
            // have scrolled off the left, the inner Miller renderer
            // skips that many leading nav cols by way of
            // `column_offset`. Otherwise (shrinking layout) we pass
            // 0 — render every nav col.
            let column_offset = if scrolling_browse { ribbon_start.saturating_sub(1) } else { 0 };
            render_browse_miller_columns(
                frame, state, &state.artist_nav, "artists", current_track_key,
                filter_results, filter_column, false,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override, column_offset,
            );
        }
        BrowseCategory::Playlists => {
            // Skip column 0 of playlist_nav — the playlists are
            // already enumerated in the leftmost browse column, so a
            // second "playlists" list right next to it is redundant.
            // Same fix the GUI got in `content_columns`.
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Playlists {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            // category_base_offset = 1 for Playlists (root col always
            // skipped); scroll mode adds further offset as the user
            // drills past the leftmost slot.
            let column_offset = if scrolling_browse {
                1 + ribbon_start.saturating_sub(1)
            } else { 1 };
            render_browse_miller_columns(
                frame, state, &state.playlist_nav, "playlists", current_track_key,
                filter_results, filter_column, true,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override, column_offset,
            );
        }
        BrowseCategory::Folders => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == BrowseCategory::Folders {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            render_folder_view(frame, state, filter_results, filter_column,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override);
        }
        cat if cat.is_tag_section() => {
            let (filter_results, filter_column) = if state.list_filter.active
                && state.list_filter.category == cat {
                (state.list_filter.results.as_ref(), Some(state.list_filter.column))
            } else { (None, None) };
            let column_offset = if scrolling_browse { ribbon_start.saturating_sub(1) } else { 0 };
            render_browse_miller_columns(
                frame, state, &state.tag_nav, cat.name(), current_track_key,
                filter_results, filter_column, false,
                content_area, Rect { x: 0, y: 0, width: 0, height: 0 },
                Some(col_width), content_focus_override, column_offset,
            );
        }
        _ => {}
    }

    // Track details pane (when focused row is a Track).
    if let (Some(area), Some(track)) = (pane_area, pane_track.as_ref()) {
        render_track_details_pane(frame, state, area, track);
    }

    // In scrolling mode, the strip overlays the artists col's
    // leftmost cells. Painted after the Miller cols so it sits on
    // top, claiming the leftmost `strip_width` cells of the artists
    // slot. Hidden when artists has scrolled off-left.
    if show_alphabet_strip && scrolling_browse && scroll_artists_visible {
        // Artists is at ribbon idx 1. On screen its x = full_area.x +
        // ((1 - ribbon_start) * col_width). The strip overlays its
        // leftmost cells.
        let artists_screen_idx = 1usize.saturating_sub(ribbon_start);
        let strip_area = Rect {
            x: full_area.x + (artists_screen_idx as u16 * col_width),
            y: full_area.y,
            width: strip_width,
            height: full_area.height,
        };
        render_alphabet_strip(frame, state, strip_area);
    }

    // Horizontal scrollbar across the bottom row of the browse band.
    // Half-height: rail uses the "lower one eighth block" glyph, thumb
    // uses the "lower half block" glyph. Both pin to the bottom of
    // the row so they visually merge into a single thin horizontal
    // stripe rather than a chunky full-row bar. Only painted when
    // the ribbon contains more slots than fit on screen.
    if scrolling_browse && ribbon_total > RIBBON_VISIBLE && full_area.height >= 1 {
        let bar_y = full_area.y + full_area.height - 1;
        let total_cells = full_area.width as usize;
        let filled_w = ((RIBBON_VISIBLE * total_cells + ribbon_total / 2) / ribbon_total).max(1);
        let filled_x = (ribbon_start * total_cells + ribbon_total / 2) / ribbon_total;
        // Cap so the filled segment never overshoots the rail.
        let filled_x = filled_x.min(total_cells.saturating_sub(filled_w));

        let rail_style = Style::default().fg(t.colors.border).bg(t.colors.bg_primary);
        let thumb_style = Style::default()
            .fg(t.colors.fg_accent)
            .bg(t.colors.bg_primary);

        // U+2581 LOWER ONE EIGHTH BLOCK: thin line hugging the bottom
        // of the row, leaving the rest of the row empty.
        let rail_text = "\u{2581}".repeat(total_cells);
        let rail_area = Rect { x: full_area.x, y: bar_y, width: full_area.width, height: 1 };
        frame.render_widget(Paragraph::new(rail_text).style(rail_style), rail_area);

        // U+2584 LOWER HALF BLOCK: thumb stands proud above the rail
        // but still in the lower half of the row.
        let thumb_text = "\u{2584}".repeat(filled_w);
        let thumb_area = Rect {
            x: full_area.x + filled_x as u16,
            y: bar_y,
            width: filled_w as u16,
            height: 1,
        };
        frame.render_widget(Paragraph::new(thumb_text).style(thumb_style), thumb_area);

        // Register the rail for click + drag hit-testing. The mouse
        // handler maps clicks anywhere on the rail to a ribbon-slot
        // scroll position; dragging the thumb pans continuously.
        state.hit_regions.borrow_mut().miller_h_scrollbar = Some(crate::ui::hit_regions::MillerHScrollbar {
            rail: rail_area,
            thumb_x: full_area.x + filled_x as u16,
            thumb_w: filled_w as u16,
            total: ribbon_total,
            visible: RIBBON_VISIBLE,
        });
    }

    // Chrome: tab bar, transport, command bar
    if !skip_transport {
        render_transport(frame, state, layout.transport);
    }
}

/// Combined Queue / Now Playing — matches the GUI's layout where the
/// queue list and the visualizer share one screen. Top half is the
/// existing queue-mode renderer (artwork + stations + track list);
/// bottom half is the visualizer panel (waveform / spectrum /
/// spectrogram). View::Queue and View::NowPlaying both render this
/// same screen so users don't have to flip between two screens to
/// see what's playing AND what's coming up.
fn render_queue_and_visualizer(frame: &mut Frame, state: &AppState) {
    render_queue_and_visualizer_in(frame, state, frame.area(), false);
}

fn render_queue_and_visualizer_in(frame: &mut Frame, state: &AppState, area_param: Rect, skip_transport: bool) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let layout = if skip_transport {
        FullScreenLayout::without_transport(area_param)
    } else {
        FullScreenLayout::new(area_param)
    };
    let area = layout.content;

    // 50/50 vertical split: queue/sidebar/artwork on top, visualizer
    // on the bottom. The artwork inside the top section is sized so
    // its box is square in pixels (width = 2 × height in cells) —
    // and `render_artwork` uses `Resize::Crop` so the cover image
    // fills the box even if the cell aspect doesn't quite match.
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    screens::now_playing::render_queue_mode(frame, state, split[0]);
    screens::now_playing::render_visualizer_panel(frame, state, split[1]);
    if !skip_transport {
        render_transport(frame, state, layout.transport);
    }
}

fn render_search(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    // Unified search/filter screen handles all tabs including Global (with 3-column layout)
    screens::filter::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
}

fn render_similar(frame: &mut Frame, state: &AppState) {
    // Render the previous view behind the popup
    let prev = state.previous_view.unwrap_or(View::Browse);
    match prev {
        View::Queue => render_queue_and_visualizer(frame, state),
        View::NowPlaying => render_queue_and_visualizer(frame, state),
        View::Browse => render_browse(frame, state),
        _ => render_browse(frame, state),
    }

    // Overlay the similar popup
    screens::similar::render(frame, state, frame.area());
}

fn render_related(frame: &mut Frame, state: &AppState) {
    // Render the previous view behind the popup
    let prev = state.previous_view.unwrap_or(View::Browse);
    match prev {
        View::Queue => render_queue_and_visualizer(frame, state),
        View::NowPlaying => render_queue_and_visualizer(frame, state),
        View::Browse => render_browse(frame, state),
        _ => render_browse(frame, state),
    }

    // Overlay the related popup
    screens::related::render(frame, state, frame.area());
}

fn render_help(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::help::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
}

fn render_settings(frame: &mut Frame, state: &AppState) {
    let layout = FullScreenLayout::new(frame.area());

    screens::settings::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
}

/// Render folder browsing view (Miller columns style) with lazy/windowed rendering.
/// Truncate a path from the left, keeping the end visible.
/// E.g. "D:\music\artist\album" with max 15 → "…\artist\album"
fn truncate_path_left(path: &str, max_width: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_width {
        return path.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let keep = max_width - 1; // 1 char for "…"
    let skip = char_count - keep;
    let tail: String = path.chars().skip(skip).collect();
    format!("…{}", tail)
}

fn render_folder_view(
    frame: &mut Frame,
    state: &AppState,
    filter_results: Option<&crate::app::state::ListFilterResults>,
    filter_column: Option<usize>,
    left_area: Rect,
    right_area: Rect,
    fixed_col_width: Option<u16>,
    focus_override: Option<usize>,
) {
    use crate::services::FolderItemType;

    let t = theme();

    // Combine left and right panels for folder view
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    if let Some(ref folder_state) = state.folder_state {
        if folder_state.loading {
            let block = Block::default()
                .title(" folders ")
                .title_style(Style::default().fg(t.colors.fg_accent))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.colors.border_focused))
                .style(Style::default().bg(t.colors.bg_primary));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let loading = Paragraph::new("Loading...")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(loading, inner);
            return;
        }

        let num_columns = folder_state.columns.len();
        if num_columns == 0 {
            return;
        }

        // Don't show empty trailing columns
        // Find the last non-empty column (or focused column, whichever is greater)
        let last_meaningful = (0..num_columns)
            .rev()
            .find(|&i| !folder_state.columns[i].items.is_empty() || i <= folder_state.focused_column)
            .unwrap_or(0);
        let effective_columns = (last_meaningful + 1).max(num_columns.min(2));

        // Calculate column width - use fixed width when provided (from outer browse layout)
        let (max_visible, col_width) = if let Some(fixed_w) = fixed_col_width {
            let max_vis = (area.width / fixed_w).max(1) as usize;
            (max_vis, fixed_w)
        } else {
            let max_vis = 3.min(effective_columns).max(2);
            (max_vis, area.width / max_vis as u16)
        };

        // Determine which columns to show.
        // Slide based on deepest column, not focus — prevents jumps when clicking
        // between already-visible columns.
        // When focus_override is provided (category column focused), anchor viewport left.
        let viewport_focus = focus_override.unwrap_or(folder_state.focused_column);
        let rightmost_col = effective_columns.saturating_sub(1).max(viewport_focus);
        let start_col = if rightmost_col + 1 > max_visible {
            let s = rightmost_col + 1 - max_visible;
            s.min(viewport_focus)
        } else {
            0
        };

        // Get currently playing track key once for all columns
        let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

        // Register folder Miller column regions for hit-testing
        {
            let mut column_regions = Vec::new();
            for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
                let col_area = Rect {
                    x: area.x + (vis_idx as u16 * col_width),
                    y: area.y,
                    width: if vis_idx == max_visible - 1 {
                        area.width - (vis_idx as u16 * col_width)
                    } else {
                        col_width
                    },
                    height: area.height,
                };
                let block_tmp = Block::default().borders(Borders::ALL);
                let inner_tmp = block_tmp.inner(col_area);
                let close_x = if col_idx > 0 && col_area.width >= 4 {
                    Some(Rect {
                        x: col_area.x + col_area.width.saturating_sub(3),
                        y: col_area.y,
                        width: 1,
                        height: 1,
                    })
                } else {
                    None
                };
                column_regions.push(crate::ui::hit_regions::MillerColumnRegion {
                    col_idx,
                    area: col_area,
                    inner: inner_tmp,
                    rows_per_item: 1, // Folder items are always 1-row
                    is_art_mode: false,
                    art_row_height: 0,
                    close_x,
                });
            }
            let mut hr = state.hit_regions.borrow_mut();
            hr.miller_columns = Some(crate::ui::hit_regions::MillerRegions {
                area,
                columns: column_regions,
            });
        }

        for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
            let col = &folder_state.columns[col_idx];
            let is_focused = focus_override.is_none() && col_idx == folder_state.focused_column;

            let col_area = Rect {
                x: area.x + (vis_idx as u16 * col_width),
                y: area.y,
                width: if vis_idx == max_visible - 1 {
                    area.width - (vis_idx as u16 * col_width) // Last column gets remaining width
                } else {
                    col_width
                },
                height: area.height,
            };

            use crate::util::truncate_middle;

            let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };
            let is_root = col_idx == 0;

            // Show title for all columns; folder paths truncate from the left
            let max_title_width = col_area.width.saturating_sub(4) as usize; // borders + padding
            let title = if is_root && col.is_shuffled() {
                " folders (shuffled) ".to_string()
            } else if is_root {
                " folders ".to_string()
            } else if col.is_shuffled() {
                let t = truncate_path_left(&col.title, max_title_width);
                format!(" {} (shuffled) ", t)
            } else if !col.title.is_empty() {
                let t = truncate_path_left(&col.title, max_title_width);
                format!(" {} ", t)
            } else {
                String::new()
            };

            let mut block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.colors.bg_primary));

            if !title.is_empty() {
                let title_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
                block = block
                    .title(title)
                    .title_style(Style::default().fg(title_color));
            }

            let inner = block.inner(col_area);
            frame.render_widget(block, col_area);

            // Tiny "x" close affordance for every drilled-in folder
            // column (the root folder column isn't closeable).
            if col_idx > 0 && col_area.width >= 4 {
                let close_x = Rect {
                    x: col_area.x + col_area.width.saturating_sub(3),
                    y: col_area.y,
                    width: 1,
                    height: 1,
                };
                let style = if is_focused {
                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_primary)
                } else {
                    Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_primary)
                };
                frame.render_widget(Paragraph::new("\u{2715}").style(style), close_x);
            }

            if col.items.is_empty() {
                let empty = Paragraph::new("(empty)")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // LAZY LOADING: Only render visible items
                let visible_height = inner.height as usize;
                let selected_idx = col.selected_index;

                // Calculate max width for text (minus prefix and padding)
                let max_text_width = inner.width.saturating_sub(4) as usize;

                // Check if filter is active on this column
                let is_filter_column = filter_column == Some(col_idx);
                let (items_to_show, total_items, filter_active_on_col): (Vec<(usize, &crate::services::FolderItem)>, usize, bool) =
                    if let Some(results) = filter_results.filter(|_| is_filter_column) {
                        if results.matched_indices.is_empty() {
                            (vec![], 0, true)
                        } else {
                            let items: Vec<_> = results.matched_indices.iter()
                                .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
                                .collect();
                            let len = items.len();
                            (items, len, true)
                        }
                    } else {
                        let items: Vec<_> = col.items.iter().enumerate().collect();
                        let len = items.len();
                        (items, len, false)
                    };

                // Calculate scroll offset (needed for both rendering and scrollbar)
                let display_selected_idx = if let Some(results) = filter_results.filter(|_| filter_active_on_col) {
                    results.matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_height, total_items),
                };

                if items_to_show.is_empty() && filter_active_on_col {
                    let empty = Paragraph::new("no matches")
                        .style(Style::default().fg(t.colors.fg_muted));
                    frame.render_widget(empty, inner);
                } else {
                    // Only create ListItems for visible range
                    let visible_items: Vec<ListItem> = items_to_show.into_iter()
                        .skip(scroll_offset)
                        .take(visible_height)
                        .map(|(orig_idx, item)| {
                            let is_selected = orig_idx == selected_idx;

                            // Check if this item is the currently playing track
                            let is_now_playing = matches!(item.item_type, FolderItemType::Track)
                                && current_track_key.map(|k| item.key == k).unwrap_or(false);

                            let prefix = match item.item_type {
                                FolderItemType::Folder => "▸ ",
                                FolderItemType::Track if is_now_playing => "♪ ",
                                FolderItemType::Track => "  ",
                            };

                            // Use middle truncation for long titles
                            let display_title = truncate_middle(&item.title, max_text_width);

                            let style = if is_now_playing {
                                Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_selected && is_focused {
                                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                            } else if is_selected {
                                // Selected row in a non-focused folder
                                // col: dim highlight, not the cursor.
                                Style::default().fg(t.colors.selection_text).bg(t.colors.bg_highlight)
                            } else {
                                Style::default().fg(t.colors.fg_primary)
                            };
                            ListItem::new(format!("{}{}", prefix, display_title)).style(style)
                        })
                        .collect();

                    let list = List::new(visible_items);
                    frame.render_widget(list, inner);
                }

                // Scrollbar + position indicator for long lists
                if total_items > visible_height {
                    render_scrollbar(frame, col_area, total_items, visible_height, scroll_offset, Some(border_color));

                    let footer = format!("{}/{}", selected_idx + 1, total_items);
                    let footer_area = Rect::new(
                        col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
                        col_area.y + col_area.height - 1,
                        footer.len() as u16 + 1,
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                        footer_area,
                    );
                }
            }
        }
    } else {
        let block = Block::default()
            .title(" folders ")
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("Loading folders...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
    }
}

/// Determine if a Miller column should use 2-row display.
///
/// Returns true for:
/// - Special track columns (playlist tracks, all-tracks, compilation tracks, etc.)
/// - Album-grouped columns (TracksByAlbum / TracksByArtist mode)
/// - Album columns in "All Artists" mode
/// - Genre/mood album columns
fn is_two_row_column(
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    col_idx: usize,
    nav: &crate::app::state::BrowseNavigationState,
    _two_row_tracks: bool,
) -> bool {
    use crate::app::state::BrowseItem;

    let first_is_track = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Track { .. }));
    let first_is_album = col.items.first().map_or(false, |item| matches!(item, BrowseItem::Album { .. }));

    // Special track columns always get two-row display
    if first_is_track && state.is_special_track_column(nav, col_idx) {
        return true;
    }

    // Album columns in "All Artists" mode (shows artist on 2nd row)
    if first_is_album && (nav.columns.first()
        .and_then(|c| c.selected_item())
        .map_or(false, |item| matches!(item, BrowseItem::AllArtists))
        || (state.browse_category == crate::app::state::BrowseCategory::Library
            && state.library.library_sub_mode != crate::app::state::LibrarySubMode::Normal
            && col_idx == 0))
    {
        return true;
    }

    // Genre/mood album columns
    if first_is_album && state.browse_category.is_tag_section() {
        return true;
    }

    // Grouped-by-album playlist columns (albums with artist on 2nd row)
    if first_is_album && col.grouped_by_album {
        return true;
    }

    false
}

/// Single source of truth for the art-grid row height (and matching
/// art width). Used by `render_album_art_grid` for layout *and* by
/// the Miller-column hit-region registration so `mouse_input` can
/// read the same value that was rendered. Returns `(row_height,
/// art_width)`, both in terminal cells.
///
/// Sizing logic:
/// - `art_width` is capped at 60% of inner width, leaves at least 8
///   cells for the title/year on the right, and floors at 8 cells.
/// - `art_row_height` is sized so at least `TARGET_ROWS` (3) album
///   rows fit on the canonical screen height (full-screen 15"
///   MacBook Air in Ghostty). Also capped at `art_width / 2` so the
///   cover stays roughly square on narrow columns (cells are ~2:1
///   height:width), and clamped to a 6-cell floor.
pub(crate) fn compute_art_grid_row(inner_width: u16, inner_height: u16) -> (u16, u16) {
    const TARGET_ROWS: u16 = 3;
    // Hard cap on row height — the row count is the limiting factor,
    // not the column width. Without this cap, wide columns settled on
    // rows ~22 cells tall and only 2 albums fit on the canonical
    // screen (full-screen 15" MacBook Air in Ghostty). 11 cells is
    // tuned so 3 covers fit on a typical Ghostty session of ~36
    // inner rows; taller terminals get 4+, narrower ones still get
    // ≥ 1 with the lower bound below.
    const MAX_ROW_H: u16 = 11;
    let max_art = ((inner_width as u32 * 3 / 5) as u16).max(20);
    let art_width = max_art.min(inner_width.saturating_sub(8)).max(8);
    let height_bound = (inner_height / TARGET_ROWS).max(1);
    let art_row_height = height_bound
        .min(art_width / 2)
        .min(MAX_ROW_H)
        .max(6);
    (art_row_height, art_width)
}

/// Render a BrowseNavigationState as dynamic Miller columns.
/// Used for Artists, Playlists, and Genres views.
/// When filter_results is Some, only show items at the matched indices in the filter_column.
/// Count the meaningful miller columns for the active browse
/// category (mirrors the `last_meaningful` logic inside
/// `render_browse_miller_columns`). Returns 0 for Folders.
///
/// When the active nav is *loading* and there's no visible miller
/// col yet (typical state right after the user pressed Enter on a
/// playlist row but before the dispatcher pushed the tracks col),
/// returns 1 so the layout reserves a slot for the animated
/// "Loading..." placeholder. Without this, the equal-width math
/// would slide the cat col off and leave just an empty placeholder.
fn count_meaningful_miller_cols(state: &AppState) -> usize {
    use crate::app::state::BrowseCategory;

    // Folders have their own state shape — count `folder_state.columns`
    // so the equal-width layout includes them as miller slots and the
    // cat col stays anchored to the left.
    if state.browse_category == BrowseCategory::Folders {
        let Some(fs) = state.folder_state.as_ref() else {
            return 1;
        };
        if fs.loading || fs.columns.is_empty() {
            return 1;
        }
        let last_meaningful = (0..fs.columns.len())
            .rev()
            .find(|&i| !fs.columns[i].items.is_empty() || i <= fs.focused_column)
            .unwrap_or(0);
        return last_meaningful + 1;
    }

    let column_offset = match state.browse_category {
        BrowseCategory::Playlists => 1,
        _ => 0,
    };
    let Some(nav) = state.browse_nav() else {
        return 0;
    };
    let num_columns = nav.columns.len();
    if num_columns <= column_offset {
        return if nav.loading { 1 } else { 0 };
    }
    let last_meaningful = (column_offset..num_columns)
        .rev()
        .find(|&i| !nav.columns[i].items.is_empty() || i <= nav.focused_column)
        .unwrap_or(column_offset);
    let effective_columns = last_meaningful + 1;
    let n = effective_columns.saturating_sub(column_offset);
    if n == 0 && nav.loading { 1 } else { n }
}

/// Render the vertical alphabet jump strip.
///
/// 28 letters (`%`, `0`, `a..z`) split into **4 groups of 7**:
/// `% 0 a b c d e | f g h i j k l | m n o p q r s | t u v w x y z`.
/// Within a group the letters are tightly packed (one per row); the
/// 3 inter-group gaps absorb whatever extra height the strip has
/// beyond `28 + 3 = 31` rows so the four groups stay visually even
/// while filling the available height.
///
/// When the strip is shorter than 31 rows we subsample one letter
/// per row. Hit regions cover every row so a click in a gap still
/// resolves to the nearest letter.
fn render_alphabet_strip(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::handlers::helpers::ALPHABET_STRIP_LETTERS;
    let t = theme();

    // Full borders match the other browse boxes (cat col, miller cols)
    // so the strip reads as a peer column rather than a hairline.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Sort-descending puts Z-named artists at the top of the list,
    // so the strip mirrors that order: Z…A, then 9…0, then % at the
    // bottom (the last sort key in the natural ordering).
    let descending = state
        .artist_nav
        .columns
        .first()
        .map_or(false, |c| !c.sort_ascending);

    let n = ALPHABET_STRIP_LETTERS.len();
    let h = inner.height as usize;
    let pad_left = inner.width.saturating_sub(1) / 2;

    let glyph_for = |letter_idx: usize| -> String {
        let ch = ALPHABET_STRIP_LETTERS[letter_idx];
        match ch {
            '0' => "0".to_string(),
            '%' => "%".to_string(),
            // The canonical foreign-character bucket key is `'文'`, but
            // a CJK glyph is double-width and the TUI strip is only one
            // cell wide internally — `'文'` would render as a clipped /
            // invisible half-cell. Use Greek capital omega instead: a
            // single-column glyph that's still clearly non-Latin and is
            // commonly used as a "miscellaneous / everything-else"
            // marker. The GUI strip has more room and renders `'文'`
            // directly there.
            '文' => "Ω".to_string(),
            c => c.to_ascii_uppercase().to_string(),
        }
    };
    let style_for = |letter_idx: usize| -> Style {
        let is_selected = state.alphabet_strip_focused && state.alphabet_strip_index == letter_idx;
        if is_selected {
            Style::default()
                .fg(t.colors.selection_text)
                .bg(t.colors.bg_selection)
        } else {
            Style::default().fg(t.colors.fg_muted)
        }
    };

    // 4 even groups × 7 letters each cover indices 0..28 (% 0 a..z),
    // plus a separate foreign-character bucket (`'文'`, index 28) sitting
    // a gap below the last group.
    const N_GROUPS: usize = 4;
    const GROUP_SIZE: usize = 7;
    const N_GAPS: usize = N_GROUPS; // gaps between groups + one before 文
    const MAIN_LETTERS: usize = N_GROUPS * GROUP_SIZE; // 28
    const EXTRA_LETTERS: usize = 1; // 文
    const MIN_HEIGHT: usize = MAIN_LETTERS + EXTRA_LETTERS + N_GAPS; // 33

    // Compute the row for each visual letter index. Returns a
    // `Vec<usize>` of length `n` mapping visual_idx → row when there
    // is room for the 4-group + foreign-bucket layout.
    let letter_rows: Option<Vec<usize>> = if h >= MIN_HEIGHT {
        let extra = h - MIN_HEIGHT;
        // Distribute extra rows: half to inter-group gaps (so the
        // groups visibly separate on tall strips), half to top/bottom
        // padding (so the strip doesn't pin to top + bottom edges
        // on very tall windows). When the extra is small, every
        // available row goes to the gaps first.
        let cap_gap_extra = extra.min(extra / 2 + (extra % 2));
        let pad_total = extra - cap_gap_extra;
        let pad_top = pad_total / 2;

        let gap_avg = 1 + cap_gap_extra / N_GAPS;
        let gap_extra = cap_gap_extra % N_GAPS;

        let mut rows = Vec::with_capacity(n);
        let mut row = pad_top;
        for group in 0..N_GROUPS {
            for _ in 0..GROUP_SIZE {
                rows.push(row);
                row += 1;
            }
            // After each group, leave a gap (including after the last
            // group, which separates the 文 foreign bucket).
            row += gap_avg + (if group < gap_extra { 1 } else { 0 });
        }
        // The lone foreign-bucket letter (文) sits below the gap.
        rows.push(row);
        Some(rows)
    } else {
        None
    };

    let mut letters_hit: Vec<(Rect, usize)> = Vec::with_capacity(h);

    if let Some(rows) = letter_rows {
        // Render each letter at its target row.
        for visual_idx in 0..n {
            let letter_idx = if descending { n - 1 - visual_idx } else { visual_idx };
            let row = rows[visual_idx];
            if row < h {
                let cell = Rect {
                    x: inner.x,
                    y: inner.y + row as u16,
                    width: inner.width,
                    height: 1,
                };
                let line = format!("{}{}", " ".repeat(pad_left as usize), glyph_for(letter_idx));
                frame.render_widget(Paragraph::new(line).style(style_for(letter_idx)), cell);
            }
        }

        // Build hit regions for every row by snapping each row to the
        // nearest letter row. Avoids a binary search by walking
        // forward through `rows` (which is monotonic).
        let mut next_idx = 0usize;
        for row in 0..h {
            // Advance `next_idx` while the next letter is closer to
            // `row` than the current one.
            while next_idx + 1 < n {
                let cur = rows[next_idx];
                let nxt = rows[next_idx + 1];
                let cur_dist = if row >= cur { row - cur } else { cur - row };
                let nxt_dist = if row >= nxt { row - nxt } else { nxt - row };
                if nxt_dist < cur_dist {
                    next_idx += 1;
                } else {
                    break;
                }
            }
            let visual_idx = next_idx;
            let letter_idx = if descending { n - 1 - visual_idx } else { visual_idx };
            letters_hit.push((
                Rect {
                    x: inner.x,
                    y: inner.y + row as u16,
                    width: inner.width,
                    height: 1,
                },
                letter_idx,
            ));
        }
    } else {
        // Strip is shorter than the 4-group minimum — subsample one
        // letter per row. Each row both renders a glyph and is its
        // own hit region.
        for row in 0..h {
            let visual_idx = (row * n) / h;
            let letter_idx = if descending { n - 1 - visual_idx } else { visual_idx };
            let cell = Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            };
            let line = format!("{}{}", " ".repeat(pad_left as usize), glyph_for(letter_idx));
            frame.render_widget(Paragraph::new(line).style(style_for(letter_idx)), cell);
            letters_hit.push((cell, letter_idx));
        }
    }

    let mut hr = state.hit_regions.borrow_mut();
    hr.alphabet_strip = Some(crate::ui::hit_regions::AlphabetStripRegions {
        area,
        letters: letters_hit,
    });
}

/// Right-side track details pane. Mirrors the GUI's
/// `track_details_pane`: header with " track " label, a clickable
/// Play Track button, square album artwork (or placeholder), then
/// track metadata — title, artist, album + year, duration, track #,
/// file basename — and a Sonically-Similar list at the bottom.
/// Renders focus highlights (Play button or similar row) when the
/// pane has keyboard focus.
fn render_track_details_pane(
    frame: &mut Frame,
    state: &AppState,
    area: Rect,
    track: &crate::plex::models::Track,
) {
    use crate::util::{format_duration, truncate_middle};
    let t = theme();

    let pane_focused = state.track_pane_focused;
    let pane_idx = state.track_pane_index;

    let block = Block::default()
        .title(" track ")
        .title_style(Style::default().fg(if pane_focused {
            t.colors.title_focused
        } else {
            t.colors.fg_accent
        }))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if pane_focused {
            t.colors.title_focused
        } else {
            t.colors.border
        }))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Tiny "x" close glyph in the top-right corner — same affordance
    // as the Miller columns. The pane is the rightmost "column" in
    // the browse layout so it gets the same treatment.
    let close_x_rect: Option<Rect> = if area.width >= 4 {
        let r = Rect {
            x: area.x + area.width.saturating_sub(3),
            y: area.y,
            width: 1,
            height: 1,
        };
        let style = if pane_focused {
            Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_primary)
        } else {
            Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_primary)
        };
        frame.render_widget(Paragraph::new("\u{2715}").style(style), r);
        Some(r)
    } else {
        None
    };

    // Top row: clickable Play Track button. Highlighted when the
    // pane has keyboard focus and `track_pane_index == 0`. Anchored
    // to the right side of the row so it lines up with the GUI's
    // primary-action button on its track-details pane.
    //
    // The Play row is pinned at the top of the pane; everything else
    // (artwork, metadata, similar list) renders in the scrollable
    // region below it. This keeps the primary action one keystroke
    // away even when the user has scrolled deep into similar tracks.
    let play_label = " ▶  Play Track ";
    let play_w = (play_label.chars().count() as u16).min(inner.width);
    let play_x = inner.x + inner.width.saturating_sub(play_w);
    let play_area = Rect {
        x: play_x,
        y: inner.y,
        width: play_w,
        height: 1,
    };
    let play_focused = pane_focused && pane_idx == 0;
    let play_style = if play_focused {
        Style::default()
            .fg(t.colors.selection_text)
            .bg(t.colors.bg_selection)
            .add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        Style::default()
            .fg(t.colors.fg_accent)
            .add_modifier(ratatui::style::Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(play_label).style(play_style),
        play_area,
    );

    // Scrollable body region below the pinned Play row. Reserve the
    // rightmost column for the scrollbar so content doesn't bleed
    // under it.
    let body_y = inner.y + 1;
    let body_h = inner.height.saturating_sub(1);
    if body_h == 0 {
        // No room for a body; register hit regions and return.
        let mut hr = state.hit_regions.borrow_mut();
        hr.track_pane = Some(crate::ui::hit_regions::TrackPaneRegions {
            outer: area,
            play_button: play_area,
            similar_rows: Vec::new(),
            close_x: close_x_rect,
        });
        return;
    }
    let scrollbar_w: u16 = if body_h >= 4 { 1 } else { 0 };
    let body_w = inner.width.saturating_sub(scrollbar_w);

    // Artwork: scale to roughly half the body height so similar
    // tracks have room below it. In tall mode the body is short
    // (~6-10 rows), so a quarter of that gives a small but
    // recognisable thumbnail. Cap at 12 cells on tall panes so the
    // artwork never dominates.
    let art_max_h_by_body = (body_h / 2).max(2);
    let art_max_h_cap: u16 = 12;
    let art_max_h = art_max_h_by_body.min(art_max_h_cap);
    let art_max_w = body_w;
    let art_w = art_max_w.min(art_max_h.saturating_mul(2)).max(4);
    let art_h = (art_w / 2).max(2).min(art_max_h);

    // Build the list of content rows. Each entry is `(content_y,
    // render_fn)` where `content_y` is the row offset inside the
    // virtual body buffer (0 = first row of body, just below Play).
    // Total content height = `content_h`. Items whose `content_y`
    // is outside [scroll, scroll + body_h) are skipped.
    //
    // Layout (content coords):
    //   row 0       : (blank — separates Play from artwork)
    //   1..1+art_h  : artwork
    //   1+art_h     : (blank)
    //   2+art_h..   : metadata lines
    //   ...         : (blank)
    //   ...         : "Sonically Similar" header
    //   ...         : similar rows

    let max_w = body_w as usize;
    let mut lines: Vec<(Style, String)> = Vec::new();

    // Title (bold/accent).
    lines.push((
        Style::default()
            .fg(t.colors.fg_primary)
            .add_modifier(ratatui::style::Modifier::BOLD),
        truncate_middle(&track.title, max_w),
    ));

    // Track artist.
    lines.push((
        Style::default().fg(t.colors.fg_primary),
        truncate_middle(track.track_artist(), max_w),
    ));

    // Album + (year).
    let album_year = match (track.parent_title.as_deref(), track.year) {
        (Some(a), Some(y)) => format!("{}  ({})", a, y),
        (Some(a), None)    => a.to_string(),
        (None, Some(y))    => y.to_string(),
        (None, None)       => String::new(),
    };
    if !album_year.is_empty() {
        lines.push((
            Style::default().fg(t.colors.fg_muted),
            truncate_middle(&album_year, max_w),
        ));
    }

    // Blank spacer.
    lines.push((Style::default(), String::new()));

    // Duration.
    let dur = track.duration_ms();
    if dur > 0 {
        lines.push((
            Style::default().fg(t.colors.fg_muted),
            format!("Duration: {}", format_duration(dur)),
        ));
    }

    // Track number.
    if let Some(n) = track.index {
        lines.push((
            Style::default().fg(t.colors.fg_muted),
            format!("Track #{}", n),
        ));
    }

    // File basename.
    if let Some(fname) = track.file_name() {
        lines.push((
            Style::default().fg(t.colors.fg_muted),
            truncate_middle(&format!("File: {}", fname), max_w),
        ));
    }

    // ── Compute virtual content layout ──────────────────────────────
    // Content rows are addressed in body-relative coordinates (0 = the
    // first row of the body region, just below the pinned Play row).
    // Layout: blank, artwork, blank, metadata lines, blank, "Sonically
    // Similar" header, similar rows (or placeholder).
    let art_top: u16 = 1;                      // 1-row gap after Play
    let art_bot: u16 = art_top + art_h;        // exclusive
    let info_top: u16 = art_bot + 1;           // 1-row gap after art
    let info_bot: u16 = info_top + lines.len() as u16;
    let header_y: u16 = info_bot + 1;          // 1-row gap after info
    let list_y: u16 = header_y + 1;
    let similar_data = state.track_pane_similar.get(&track.rating_key);
    let similar_count: u16 = similar_data
        .map(|v| if v.is_empty() { 1 } else { v.len() as u16 })
        .unwrap_or(1); // "Loading…" placeholder occupies 1 row
    let content_h: u16 = list_y + similar_count;

    // Auto-scroll: when the pane has focus and the user has highlighted
    // a similar row that lives outside the visible window, slide the
    // window so the row is on screen. Selection at index 0 (Play) is
    // pinned at the top and never affects scroll.
    let selected_content_y: Option<u16> = if pane_focused && pane_idx > 0 {
        // similar_y(i) = list_y + i (where i = pane_idx - 1)
        Some(list_y + (pane_idx as u16 - 1))
    } else {
        None
    };
    let max_scroll = content_h.saturating_sub(body_h);
    let scroll: u16 = match selected_content_y {
        Some(target) if target < 0_u16.saturating_add(0) => 0,
        Some(target) if target >= body_h => {
            let want = target.saturating_sub(body_h - 1);
            want.min(max_scroll)
        }
        _ => 0,
    };

    // Helper: convert content_y → screen_y, returning None if the row
    // is clipped (above or below the body window).
    let to_screen = |cy: u16| -> Option<u16> {
        if cy < scroll { return None; }
        let off = cy - scroll;
        if off >= body_h { return None; }
        Some(body_y + off)
    };

    // Artwork: render only if any of its rows fall in the visible
    // window. ratatui-image draws into a single sub-rect, so we just
    // clamp the rect to the visible band.
    let art_x = inner.x + body_w.saturating_sub(art_w) / 2;
    let visible_art_top = art_top.max(scroll);
    let visible_art_bot = art_bot.min(scroll + body_h);
    if visible_art_top < visible_art_bot {
        let screen_top = body_y + (visible_art_top - scroll);
        // Only call the image renderer when the full artwork fits;
        // partial slices look ugly with cell-quantised glyphs. Show
        // the (no cover) placeholder when partially clipped.
        let mut rendered_art = false;
        if visible_art_top == art_top && visible_art_bot == art_bot {
            let art_area = Rect {
                x: art_x,
                y: screen_top,
                width: art_w,
                height: art_h,
            };
            if let Some(album_key) = track.parent_rating_key.as_deref() {
                if let Some(data) = state.artwork.grid_cache.get(album_key) {
                    rendered_art = super::artwork::render_grid_image(frame, art_area, album_key, data);
                }
            }
            if !rendered_art {
                let label = "(no cover)";
                let lw = label.chars().count() as u16;
                let cx = art_area.x + art_area.width.saturating_sub(lw) / 2;
                let cy = art_area.y + art_area.height / 2;
                frame.render_widget(
                    Paragraph::new(label).style(Style::default().fg(t.colors.fg_muted)),
                    Rect { x: cx, y: cy, width: lw, height: 1 },
                );
            }
        } else {
            // Partially clipped — paint a blank-ish slice so any
            // previous frame's glyphs don't bleed through.
            for r in visible_art_top..visible_art_bot {
                let sy = body_y + (r - scroll);
                frame.render_widget(
                    Paragraph::new("").style(Style::default().bg(t.colors.bg_primary)),
                    Rect { x: inner.x, y: sy, width: body_w, height: 1 },
                );
            }
        }
    }

    // Metadata lines.
    for (i, (style, text)) in lines.iter().enumerate() {
        let cy = info_top + i as u16;
        if let Some(sy) = to_screen(cy) {
            frame.render_widget(
                Paragraph::new(text.as_str()).style(*style),
                Rect { x: inner.x, y: sy, width: body_w, height: 1 },
            );
        }
    }

    // "Sonically Similar" header.
    if let Some(sy) = to_screen(header_y) {
        frame.render_widget(
            Paragraph::new("Sonically Similar").style(
                Style::default()
                    .fg(t.colors.fg_accent)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Rect { x: inner.x, y: sy, width: body_w, height: 1 },
        );
    }

    // Similar rows (or placeholder).
    let mut similar_rows: Vec<(Rect, usize)> = Vec::new();
    match similar_data {
        Some(list) if list.is_empty() => {
            if let Some(sy) = to_screen(list_y) {
                frame.render_widget(
                    Paragraph::new("(no similar tracks found)")
                        .style(Style::default().fg(t.colors.fg_muted)),
                    Rect { x: inner.x, y: sy, width: body_w, height: 1 },
                );
            }
        }
        Some(list) => {
            for (i, sim) in list.iter().enumerate() {
                let cy = list_y + i as u16;
                let sy = match to_screen(cy) { Some(v) => v, None => continue };
                let label = format!(
                    "\u{2022} {} \u{2014} {}",
                    sim.title,
                    sim.track_artist(),
                );
                let truncated = truncate_middle(&label, max_w);
                let row_focused = pane_focused && pane_idx == i + 1;
                let style = if row_focused {
                    Style::default()
                        .fg(t.colors.selection_text)
                        .bg(t.colors.bg_selection)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                let row_area = Rect {
                    x: inner.x,
                    y: sy,
                    width: body_w,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(truncated).style(style),
                    row_area,
                );
                similar_rows.push((row_area, i));
            }
        }
        None => {
            if let Some(sy) = to_screen(list_y) {
                frame.render_widget(
                    Paragraph::new("Loading\u{2026}")
                        .style(Style::default().fg(t.colors.fg_muted)),
                    Rect { x: inner.x, y: sy, width: body_w, height: 1 },
                );
            }
        }
    }

    // Vertical scrollbar on the right edge of the body when content
    // overflows the visible region. Mirrors the column scrollbar
    // style used elsewhere in the TUI.
    if scrollbar_w > 0 && content_h > body_h {
        use crate::ui::widgets::scrollbar::calc_thumb;
        let (thumb_size, thumb_pos) = calc_thumb(
            content_h as usize,
            body_h as usize,
            scroll as usize,
            body_h as usize,
        );
        let bar_x = inner.x + body_w;
        for r in 0..body_h {
            let sy = body_y + r;
            let in_thumb = (r as usize) >= thumb_pos
                && (r as usize) < thumb_pos + thumb_size;
            let glyph = if in_thumb { "\u{2588}" } else { "\u{2502}" };
            let style = if in_thumb {
                Style::default().fg(t.colors.fg_accent)
            } else {
                Style::default().fg(t.colors.border)
            };
            frame.render_widget(
                Paragraph::new(glyph).style(style),
                Rect { x: bar_x, y: sy, width: 1, height: 1 },
            );
        }
    }

    // Register click regions so the mouse handler can dispatch
    // Play / drill-into-similar / close-pane.
    let mut hr = state.hit_regions.borrow_mut();
    hr.track_pane = Some(crate::ui::hit_regions::TrackPaneRegions {
        outer: area,
        play_button: play_area,
        similar_rows,
        close_x: close_x_rect,
    });
}

fn render_browse_miller_columns(
    frame: &mut Frame,
    state: &AppState,
    nav: &crate::app::state::BrowseNavigationState,
    root_title: &str,
    current_track_key: Option<&str>,
    filter_results: Option<&crate::app::state::ListFilterResults>,
    filter_column: Option<usize>,
    two_row_tracks: bool,
    left_area: Rect,
    right_area: Rect,
    fixed_col_width: Option<u16>,
    focus_override: Option<usize>,
    // Number of leading columns to hide. Used by the Playlists
    // category to suppress the redundant root "playlists" column —
    // the playlists are already enumerated in the leftmost browse
    // column, so showing them again next door is duplicate chrome.
    column_offset: usize,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;

    let t = theme();

    // Combine left and right panels for full-width Miller columns
    let area = Rect {
        x: left_area.x,
        y: left_area.y,
        width: left_area.width + right_area.width,
        height: left_area.height,
    };

    // Loading with no columns yet: show full loading state
    if nav.loading && nav.columns.is_empty() {
        let block = Block::default()
            .title(format!(" {} ", root_title))
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(loading, inner);
        return;
    }

    let num_columns = nav.columns.len();
    // After hiding the leading `column_offset` columns, do we have
    // anything left to show? If not, render a placeholder so the
    // user sees "their click registered, but there's nothing here
    // until you pick a playlist".
    if num_columns <= column_offset {
        let block = Block::default()
            .title(format!(" {} ", root_title))
            .title_style(Style::default().fg(t.colors.fg_accent))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused))
            .style(Style::default().bg(t.colors.bg_primary));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = if column_offset > 0 {
            "Pick a playlist on the left."
        } else {
            "No items"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(t.colors.fg_muted)),
            inner,
        );
        return;
    }

    // Find last non-empty column (or focused column, whichever is
    // greater), restricted to the visible range [column_offset, len).
    let last_meaningful = (column_offset..num_columns)
        .rev()
        .find(|&i| !nav.columns[i].items.is_empty() || i <= nav.focused_column)
        .unwrap_or(column_offset);
    let effective_columns = last_meaningful + 1;
    // When loading with existing columns, reserve space for a loading indicator column
    let layout_columns = if nav.loading { effective_columns + 1 } else { effective_columns };

    // Pick the column width.
    //
    // In `Scrolling` layout (Library only) the *outer* `render_browse`
    // already sized `area` and `fixed_col_width` so every Miller col
    // = col_width and only 2 cols fit. We just adopt the outer's
    // numbers here. The first visible col may overlap the alphabet
    // strip in its leftmost cells (`render_browse` paints the strip
    // on top after we render). That's OK — the artists col's items
    // are inset by 2 cells ("▸ name") so no item glyph collides.
    let scrolling = state.miller_layout == crate::app::state::MillerLayoutMode::Scrolling
        && state.browse_category == BrowseCategory::Library;
    let (max_visible, col_width) = if let Some(fixed_w) = fixed_col_width {
        let max_vis = (area.width / fixed_w).max(1) as usize;
        (max_vis, fixed_w)
    } else {
        let max_vis = 3.min(layout_columns).max(2);
        (max_vis, area.width / max_vis as u16)
    };

    // Spatial-coherence sliding: window anchors to the focused
    // column. The focused column sits at the right edge of the
    // window unless that would push the start past column_offset.
    // Drilling deeper slides one column right; backing out left
    // slides one column left so parents return to view. Columns
    // deeper than focus stay loaded but scroll off the right edge.
    let viewport_focus = focus_override.unwrap_or(nav.focused_column).max(column_offset);
    let start_col = (viewport_focus + 1).saturating_sub(max_visible).max(column_offset);

    // Register Miller column regions for hit-testing
    {
        let mut column_regions = Vec::new();
        for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
            let col = &nav.columns[col_idx];
            // In scrolling mode every column is locked at exactly
            // `col_width` — never extend the last visible column to
            // mop up leftover space, since that would make it wider
            // than half the screen and the user explicitly wants
            // every column at the same fixed half-screen width. In
            // shrinking mode keep the historic "last col absorbs
            // the rounding remainder" behavior so 3 cols always
            // sum to area.width exactly.
            let col_area = Rect {
                x: area.x + (vis_idx as u16 * col_width),
                y: area.y,
                width: if !scrolling && vis_idx == max_visible - 1 {
                    area.width - (vis_idx as u16 * col_width)
                } else {
                    col_width.min(area.width.saturating_sub(vis_idx as u16 * col_width))
                },
                height: area.height,
            };
            let block_tmp = Block::default().borders(Borders::ALL);
            let inner_tmp = block_tmp.inner(col_area);
            let is_two_row = is_two_row_column(state, col, col_idx, nav, two_row_tracks);
            // Top-right "x" close glyph hit area. Skip the root col
            // (which isn't closeable). The glyph lives inside the
            // border, one cell wide, on the top border row.
            let close_x = if col_idx > column_offset && col_area.width >= 4 {
                Some(Rect {
                    x: col_area.x + col_area.width.saturating_sub(3),
                    y: col_area.y,
                    width: 1,
                    height: 1,
                })
            } else {
                None
            };
            let art_row_height = if col.artwork_visible {
                compute_art_grid_row(inner_tmp.width, inner_tmp.height).0
            } else {
                0
            };
            column_regions.push(crate::ui::hit_regions::MillerColumnRegion {
                col_idx,
                area: col_area,
                inner: inner_tmp,
                rows_per_item: if is_two_row { 2 } else { 1 },
                is_art_mode: col.artwork_visible,
                art_row_height,
                close_x,
            });
        }
        let mut hr = state.hit_regions.borrow_mut();
        hr.miller_columns = Some(crate::ui::hit_regions::MillerRegions {
            area,
            columns: column_regions,
        });
    }

    for (vis_idx, col_idx) in (start_col..effective_columns.min(start_col + max_visible)).enumerate() {
        let col = &nav.columns[col_idx];
        // Track-details pane is treated as the rightmost column for
        // focus purposes: when it's focused, no miller column should
        // also paint as focused, otherwise the user sees two
        // simultaneously highlighted "selected" rows (the pane on the
        // right and the track row in the tracks column behind it).
        let is_focused = focus_override.is_none()
            && col_idx == nav.focused_column
            && !state.track_pane_focused;
        let is_root = col_idx == 0;

        let col_area = Rect {
            x: area.x + (vis_idx as u16 * col_width),
            y: area.y,
            // Mirror the registration-loop sizing rule above — in
            // scrolling mode columns stay exactly `col_width`; in
            // shrinking mode the last column absorbs the remainder.
            width: if !scrolling && vis_idx == max_visible - 1 {
                area.width - (vis_idx as u16 * col_width) // Last column gets remaining width
            } else {
                col_width.min(area.width.saturating_sub(vis_idx as u16 * col_width))
            },
            height: area.height,
        };

        let border_color = if is_focused { t.colors.title_focused } else { t.colors.border };

        // Show title for all columns with sort suffix
        let sort_suffix = {
            let suffix = col.sort_mode.header_suffix(!col.sort_ascending);
            if suffix.is_empty() { String::new() } else { format!(" ({})", suffix) }
        };

        let title = if is_root {
            format!(" {}{} ", root_title, sort_suffix)
        } else if !col.title.is_empty() {
            if col.grouped_by_album {
                format!(" albums - {}{} ", col.title, sort_suffix)
            } else {
                format!(" {}{} ", col.title, sort_suffix)
            }
        } else {
            String::new()
        };
        // In scroll mode the alphabet strip overlays the leftmost
        // 3 cells of the artist root col (Library only). Without
        // padding the title — which ratatui anchors to the top-left
        // border — gets eaten by the strip. Reserve those leading
        // cells with non-breaking spaces so the title text starts
        // past the strip.
        let strip_overlap_w: usize = if scrolling
            && state.browse_category == BrowseCategory::Library
            && state.alphabet_strip_visible()
            && col_idx == column_offset
            && vis_idx == 0
        {
            3
        } else {
            0
        };
        let title = if !title.is_empty() && strip_overlap_w > 0 {
            // Trim leading space, prepend `strip_overlap_w` spaces so
            // the title slides right past the overlay.
            let inner = title.trim_start();
            format!("{}{}", " ".repeat(strip_overlap_w), inner)
        } else {
            title
        };
        // Block titles overflow past the column edge when they're
        // longer than the column is wide — Mac-style middle-truncate
        // so both ends stay readable. Reserve 4 cells for the corner
        // border characters and a leading/trailing space.
        let title = if !title.is_empty() {
            let max_title_w = (col_area.width as usize).saturating_sub(4);
            if title.chars().count() > max_title_w {
                let inner = title.trim();
                let inner_max = max_title_w.saturating_sub(2);
                format!(" {} ", truncate_middle(inner, inner_max))
            } else {
                title
            }
        } else {
            title
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_primary));

        if !title.is_empty() {
            let title_color = if is_focused { t.colors.title_focused } else { t.colors.fg_accent };
            block = block
                .title(title)
                .title_style(Style::default().fg(title_color));
        }

        let full_inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        // Tiny "x" close affordance in the top-right corner of every
        // closeable column (everything past the hidden root prefix).
        // Painted directly over the border so it rides on top of the
        // line-drawing glyph that ratatui's `Block` already drew.
        if col_idx > column_offset && col_area.width >= 4 {
            let close_x = Rect {
                x: col_area.x + col_area.width.saturating_sub(3),
                y: col_area.y,
                width: 1,
                height: 1,
            };
            let style = if is_focused {
                Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_primary)
            } else {
                Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_primary)
            };
            frame.render_widget(Paragraph::new("\u{2715}").style(style), close_x);
        }

        // Synthetic "▶ Play album / playlist" row pinned above the
        // real items on any tracks column flagged with `play_all_row`.
        // Eats the top inner line; the remainder of the column
        // renders into `inner` below as before. The cursor parks on
        // this row by default (`col.on_play_row`); ↓ drops it to
        // `items[0]`, ↑ from `items[0]` brings it back.
        let inner = if col.play_all_row.is_some() && full_inner.height > 0 {
            let play_row_rect = Rect {
                x: full_inner.x,
                y: full_inner.y,
                width: full_inner.width,
                height: 1,
            };
            let label = col.play_all_row.as_ref().map(|p| p.label()).unwrap_or("Play");
            // Highlight when the column is focused AND the cursor is
            // on the play row. Otherwise paint as a regular header
            // affordance — still legible, just not selected.
            let highlighted = is_focused && col.on_play_row;
            let style = if highlighted {
                Style::default()
                    .bg(t.colors.bg_selection)
                    .fg(t.colors.selection_text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.colors.fg_accent)
            };
            let max_w = play_row_rect.width.saturating_sub(3) as usize;
            let shown = if label.chars().count() > max_w {
                let truncated: String = label.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", truncated)
            } else {
                label.to_string()
            };
            frame.render_widget(
                Paragraph::new(format!("\u{25B6} {}", shown)).style(style),
                play_row_rect,
            );
            Rect {
                x: full_inner.x,
                y: full_inner.y + 1,
                width: full_inner.width,
                height: full_inner.height - 1,
            }
        } else {
            full_inner
        };

        if col.items.is_empty() {
            let empty = Paragraph::new("(empty)")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            continue;
        }

        // Quick-filter (transport-bar text input) narrows EVERY
        // visible Miller column on-the-fly — same behaviour as the
        // GUI. The original `filter_results` precomputed for ONE
        // column is ignored for rendering; we run
        // `filter_with_priority` against this column's items
        // inline instead.
        let live_query: Option<&str> = if state.list_filter.active
            && state.list_filter.category == state.browse_category
            && !state.list_filter.query.trim().is_empty()
        {
            Some(state.list_filter.query.trim())
        } else {
            None
        };
        let per_col_matches: Option<Vec<usize>> = live_query.map(|q| {
            use crate::services::{filter_with_priority, DEFAULT_MAX_RESULTS};
            filter_with_priority(&col.items, q, |it| it.title(), DEFAULT_MAX_RESULTS).matched_indices
        });
        // For the historic single-column filter (used by FilteredList*
        // selection actions in the dispatcher), only the column the
        // user is anchored on gets keyboard-driven selection helpers.
        let is_filter_column = filter_column == Some(col_idx);

        if col.artwork_visible {
            // Album-art grids still need filter results; for live
            // multi-column filtering we synthesize a fake
            // ListFilterResults so the grid path keeps working.
            let synth = per_col_matches.as_ref().map(|m| crate::app::state::ListFilterResults {
                matched_indices: m.clone(),
                total_matches: m.len(),
                has_more: false,
            });
            let col_filter_owned = synth.or_else(|| {
                if is_filter_column { filter_results.cloned() } else { None }
            });
            render_album_art_grid(
                frame, state, col, is_focused, inner, col_area, col_idx,
                col_filter_owned.as_ref(),
            );
            continue;
        }

        {
            // Calculate visible range for lazy loading
            let visible_height = inner.height as usize;
            let selected_idx = col.selected_index;

            // Calculate max width for text (minus prefix and padding)
            let max_text_width = inner.width.saturating_sub(4) as usize;

            let is_two_row = is_two_row_column(state, col, col_idx, nav, two_row_tracks);
            let rows_per_item = if is_two_row { 2 } else { 1 };
            let visible_item_count = visible_height / rows_per_item;

            // Pick which row indices to show. Per-column live filter
            // first (every column), then the legacy single-column
            // filter_results as a fallback for anything that still
            // expects it.
            let (items_to_show, total_display_items, filter_active_on_col): (Vec<(usize, &BrowseItem)>, usize, bool) =
                if let Some(matched) = per_col_matches.as_ref() {
                    if matched.is_empty() {
                        (vec![], 0, true)
                    } else {
                        let items: Vec<_> = matched.iter()
                            .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
                            .collect();
                        let len = items.len();
                        (items, len, true)
                    }
                } else if let Some(results) = filter_results.filter(|_| is_filter_column) {
                    if results.matched_indices.is_empty() {
                        (vec![], 0, true)
                    } else {
                        let items: Vec<_> = results.matched_indices.iter()
                            .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
                            .collect();
                        let len = items.len();
                        (items, len, true)
                    }
                } else {
                    let items: Vec<_> = col.items.iter().enumerate().collect();
                    let len = items.len();
                    (items, len, false)
                };

            if items_to_show.is_empty() && filter_active_on_col {
                let empty = Paragraph::new("no matches")
                    .style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // Calculate scroll offset based on display items.
                // When per-column live filter is active, map the
                // real selected_index into the filtered display
                // index; otherwise fall back to the historic single-
                // column results / unfiltered position.
                let display_selected_idx = if let Some(matched) = per_col_matches.as_ref() {
                    matched.iter().position(|&idx| idx == selected_idx).unwrap_or(0)
                } else if let Some(results) = filter_results.filter(|_| filter_active_on_col) {
                    results.matched_indices.iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(display_selected_idx, visible_item_count, total_display_items),
                };

                let visible_items: Vec<ListItem> = items_to_show.into_iter()
                    .skip(scroll_offset)
                    .take(visible_item_count)
                    .map(|(orig_idx, item)| {
                        // When the cursor parks on the synthetic
                        // Play row, no real item should render as
                        // "selected" — the play row owns the
                        // highlight until ↓ pulls the cursor down
                        // into the items.
                        let is_selected = orig_idx == selected_idx && !col.on_play_row;
                        // Multi-select: row is in `col.selected_set`.
                        // Drives a visual "●" prefix and an accent
                        // tint, mirroring the queue track-list style.
                        let is_multi_selected = col.selected_set.contains(&orig_idx);

                        // Check if this is the currently playing track
                        let is_now_playing = matches!(item, BrowseItem::Track { key, .. } if current_track_key == Some(key.as_str()));

                        // Prefix based on item type
                        let is_pinned = matches!(item,
                            BrowseItem::AllArtists | BrowseItem::Compilations |
                            BrowseItem::AllTracks { .. } | BrowseItem::ArtistRadio { .. } |
                            BrowseItem::CompilationTracks { .. }
                        );
                        let prefix = match item {
                            BrowseItem::Track { .. } if is_now_playing && is_multi_selected => "♪●",
                            BrowseItem::Track { .. } if is_now_playing => "♪ ",
                            BrowseItem::Track { .. } if is_multi_selected => "● ",
                            BrowseItem::Track { .. } => "  ",
                            // Artist Radio plays on Enter / click —
                            // mark it with a play glyph rather than
                            // the usual blank, so the affordance is
                            // legible.
                            BrowseItem::ArtistRadio { .. } => "▶ ",
                            _ if is_multi_selected => "● ",
                            _ if is_pinned => "  ", // No arrow for pinned items
                            _ => "▸ ", // Drillable items get arrow
                        };

                        // Full text for line 1 (before truncation)
                        let full_text = match item {
                            BrowseItem::Album { title, year, .. } => {
                                if let Some(y) = year {
                                    format!("{} ({})", title, y)
                                } else {
                                    title.clone()
                                }
                            }
                            BrowseItem::Track { title, track_number, .. } => {
                                // Show track numbers only in album drill-downs (1-row mode)
                                if !is_two_row {
                                    if let Some(num) = track_number {
                                        format!("{:02}. {}", num, title)
                                    } else {
                                        title.clone()
                                    }
                                } else {
                                    title.clone()
                                }
                            }
                            _ => item.title().to_string(),
                        };

                        // Duration string for tracks (right-aligned)
                        let dur_str = match item {
                            BrowseItem::Track { duration_ms, .. } if *duration_ms > 0 => {
                                Some(crate::util::format_duration(*duration_ms))
                            }
                            _ => None,
                        };
                        // Reduce title width to make room for duration
                        let title_width = if let Some(ref d) = dur_str {
                            max_text_width.saturating_sub(d.len() + 1)
                        } else {
                            max_text_width
                        };

                        // Marquee for selected+focused item, or truncate normally
                        let display_text = if is_selected && is_focused {
                            let marquee_key = format!("miller:{}:{}", col_idx, orig_idx);
                            let mut marquee = state.marquee.borrow_mut();
                            if marquee.selection_key != marquee_key {
                                marquee.reset(marquee_key, full_text.clone(), title_width);
                            }
                            if marquee.phase == crate::app::state::MarqueePhase::Inactive {
                                truncate_middle(&full_text, title_width)
                            } else {
                                let text = marquee.display_text();
                                drop(marquee);
                                // Trim to title_width (display_text already pads)
                                text.chars().take(title_width).collect()
                            }
                        } else {
                            truncate_middle(&full_text, title_width)
                        };

                        // Build ListItem — 2-row for playlist tracks or All Artists albums, 1-row otherwise
                        if is_two_row {
                            // Determine subtitle content based on item type
                            let subtitle_content = match item {
                                BrowseItem::Track { artist_name, album_name, year, .. } => {
                                    match (artist_name.as_ref(), album_name.as_ref()) {
                                        (Some(a), Some(b)) => {
                                            if let Some(y) = year {
                                                format!("{} — {} ({})", a, b, y)
                                            } else {
                                                format!("{} — {}", a, b)
                                            }
                                        }
                                        (Some(a), None) => a.clone(),
                                        (None, Some(b)) => {
                                            if let Some(y) = year {
                                                format!("{} ({})", b, y)
                                            } else {
                                                b.clone()
                                            }
                                        }
                                        (None, None) => String::new(),
                                    }
                                }
                                BrowseItem::Album { artist, .. } => {
                                    // All Artists mode: show artist on second row
                                    artist.clone()
                                }
                                _ => String::new(),
                            };

                            if !subtitle_content.is_empty() || matches!(item, BrowseItem::Track { .. } | BrowseItem::Album { .. }) {
                                // Subtitle display width (5 indent + 2 padding = 7 overhead)
                                let subtitle_width = (inner.width as usize).saturating_sub(7);

                                // Marquee for subtitle row (independent of title)
                                let subtitle_display = if is_selected && is_focused && !subtitle_content.is_empty() {
                                    let sub_key = format!("miller:{}:{}:sub", col_idx, orig_idx);
                                    let mut sub_marquee = state.marquee_subtitle.borrow_mut();
                                    if sub_marquee.selection_key != sub_key {
                                        sub_marquee.reset(sub_key, subtitle_content.clone(), subtitle_width);
                                    }
                                    if sub_marquee.phase == crate::app::state::MarqueePhase::Inactive {
                                        truncate_middle(&subtitle_content, subtitle_width)
                                    } else {
                                        let text = sub_marquee.display_text();
                                        drop(sub_marquee);
                                        text
                                    }
                                } else {
                                    truncate_middle(&subtitle_content, subtitle_width)
                                };

                                let (line1_fg, line2_fg, item_bg) = if is_selected && is_focused {
                                    (
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().bg(t.colors.selection_bar_bg),
                                    )
                                } else if is_selected {
                                    // Selected row in a non-focused col:
                                    // dim "you were here" mark instead of
                                    // the strong cursor highlight, so the
                                    // user can tell at a glance which col
                                    // is actually accepting input.
                                    (
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().fg(t.colors.selection_text),
                                        Style::default().bg(t.colors.bg_highlight),
                                    )
                                } else if is_multi_selected {
                                    (
                                        Style::default().fg(t.colors.fg_accent),
                                        Style::default().fg(t.colors.fg_accent),
                                        Style::default().bg(t.colors.bg_secondary),
                                    )
                                } else if is_now_playing {
                                    (
                                        Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD),
                                        Style::default().fg(t.colors.fg_accent),
                                        Style::default(),
                                    )
                                } else {
                                    (
                                        Style::default().fg(t.colors.fg_primary),
                                        Style::default().fg(t.colors.fg_muted),
                                        Style::default(),
                                    )
                                };

                                // Build line 1 with optional right-aligned duration
                                let line1 = if let Some(ref dur) = dur_str {
                                    let title_chars = display_text.chars().count();
                                    let pad = title_width.saturating_sub(title_chars);
                                    Line::from(Span::styled(
                                        format!("{}{}{} {}", prefix, display_text, " ".repeat(pad), dur),
                                        line1_fg,
                                    ))
                                } else {
                                    Line::from(Span::styled(format!("{}{}", prefix, display_text), line1_fg))
                                };

                                let text = Text::from(vec![
                                    line1,
                                    Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg)),
                                ]);
                                ListItem::new(text).style(item_bg)
                            } else {
                                // Non-track/album item in a two-row column (handle gracefully)
                                let style = if is_selected && is_focused {
                                    Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                                } else if is_selected {
                                    Style::default().fg(t.colors.selection_text).bg(t.colors.bg_highlight)
                                } else if is_multi_selected {
                                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary)
                                } else if is_pinned {
                                    Style::default().fg(t.colors.fg_accent)
                                } else {
                                    Style::default().fg(t.colors.fg_primary)
                                };
                                ListItem::new(format!("{}{}", prefix, display_text)).style(style)
                            }
                        } else {
                            let style = if is_selected && is_focused {
                                // Cursor selection always wins so the
                                // user can see where the highlight is
                                // while expanding a multi-selection.
                                Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
                            } else if is_selected {
                                // Same row in an unfocused col: dim
                                // "you were here" mark.
                                Style::default().fg(t.colors.selection_text).bg(t.colors.bg_highlight)
                            } else if is_multi_selected {
                                Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary)
                            } else if is_now_playing {
                                Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_pinned {
                                Style::default().fg(t.colors.fg_accent)
                            } else {
                                Style::default().fg(t.colors.fg_primary)
                            };

                            // Build display with optional right-aligned duration
                            if let Some(ref dur) = dur_str {
                                let title_chars = display_text.chars().count();
                                let pad = title_width.saturating_sub(title_chars);
                                ListItem::new(format!("{}{}{} {}", prefix, display_text, " ".repeat(pad), dur)).style(style)
                            } else {
                                ListItem::new(format!("{}{}", prefix, display_text)).style(style)
                            }
                        }
                    })
                    .collect();

                let list = List::new(visible_items);
                frame.render_widget(list, inner);

                // Scrollbar + position indicator for long lists
                if total_display_items > visible_item_count {
                    // Render scrollbar on right edge of column
                    render_scrollbar(
                        frame,
                        col_area,
                        total_display_items,
                        visible_item_count,
                        scroll_offset,
                        Some(border_color),
                    );

                    let footer = format!("{}/{}", display_selected_idx + 1, total_display_items);
                    let footer_area = Rect::new(
                        col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
                        col_area.y + col_area.height - 1,
                        footer.len() as u16 + 1,
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
                        footer_area,
                    );
                }
            }
        }
    }

    // Equal-width layout: the outer `render_browse` already sized
    // `area` to fit exactly the meaningful columns at `col_width`,
    // so there are no spacer slots to fill. Loading placeholder
    // sits on the next-empty slot when nav is loading — animated
    // dot pattern in the background + "Loading..." text whose
    // trailing dots animate via `state.loading_tick`.
    let real_rendered = effective_columns.min(start_col + max_visible).saturating_sub(start_col);
    if nav.loading && real_rendered < max_visible {
        let vis_idx = real_rendered;
        let placeholder_area = Rect {
            x: area.x + (vis_idx as u16 * col_width),
            y: area.y,
            width: col_width.min(area.width.saturating_sub(vis_idx as u16 * col_width)),
            height: area.height,
        };
        render_loading_column(frame, placeholder_area, state.loading_tick);
    }

    // Scroll-position visual lives at the OUTER browse layer
    // (`render_browse`) so it spans the full window and reflects
    // the entire ribbon (sections + nav + pane), not just nav cols.
    let _ = scrolling;
}

/// Render an animated "Loading…" placeholder filling the given
/// area. Sparse dot pattern in the background; centred "Loading"
/// text with trailing dots driven by `loading_tick % 4`.
fn render_loading_column(frame: &mut Frame, area: Rect, loading_tick: u32) {
    let t = theme();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let muted_border = Style::default().fg(t.colors.fg_muted);
    let muted_bg = Style::default().bg(t.colors.bg_primary).fg(t.colors.fg_muted);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(muted_border)
        .style(muted_bg);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Sparse dot fill so the col reads as "intentionally empty
    // pending data" rather than "broken render". One dot every
    // two cells horizontally, every two rows vertically.
    let dot_row: String = (0..inner.width)
        .map(|x| if x % 2 == 0 { '\u{00b7}' } else { ' ' })
        .collect();
    for dy in 0..inner.height {
        if dy % 2 != 0 { continue; }
        let row_rect = Rect {
            x: inner.x,
            y: inner.y + dy,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(dot_row.clone())
                .style(Style::default().fg(t.colors.fg_muted)),
            row_rect,
        );
    }

    // Animated "Loading…" text — 4 frames (no dots, 1, 2, 3 dots).
    // Centred horizontally and vertically on the placeholder.
    let phase = (loading_tick as usize) % 4;
    let dots = match phase {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    };
    let label = format!("Loading{:.<3}", dots);
    let label_w = label.chars().count() as u16;
    if label_w <= inner.width {
        let lx = inner.x + inner.width.saturating_sub(label_w) / 2;
        let ly = inner.y + inner.height / 2;
        if ly < inner.y + inner.height {
            frame.render_widget(
                Paragraph::new(label)
                    .style(Style::default().fg(t.colors.fg_accent)),
                Rect { x: lx, y: ly, width: label_w, height: 1 },
            );
        }
    }
}

/// Render album art list for a column in cover art view mode.
/// Each row: artwork thumbnail on the left, title/artist text on the right.
fn render_album_art_grid(
    frame: &mut Frame,
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    is_focused: bool,
    inner: Rect,
    col_area: Rect,
    col_idx: usize,
    filter_results: Option<&crate::app::state::ListFilterResults>,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;
    let t = theme();

    // Build the list of items to display (filtered or full)
    let items_with_indices: Vec<(usize, &BrowseItem)> = if let Some(results) = filter_results {
        if results.matched_indices.is_empty() {
            let empty = Paragraph::new("no matches")
                .style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            return;
        }
        results.matched_indices.iter()
            .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item)))
            .collect()
    } else {
        col.items.iter().enumerate().collect()
    };

    let total_items = items_with_indices.len();
    if total_items == 0 {
        return;
    }

    // Classify items: "one-row" pinned items vs normal art-height items
    fn is_one_row(item: &BrowseItem) -> bool {
        matches!(item,
            BrowseItem::ArtistRadio { .. } |
            BrowseItem::AllTracks { .. } |
            BrowseItem::CompilationTracks { .. } |
            BrowseItem::Compilations
        )
    }

    // Count art items to size rows (one-row items don't affect art sizing)
    let art_item_count = items_with_indices.iter().filter(|(_, item)| !is_one_row(item)).count();

    // Each list row: artwork on left, text on right.
    //
    // Single source of truth for the art-grid row height — both the
    // renderer and the mouse hit-test go through `compute_art_grid_row`.
    let _ = art_item_count;
    let (art_row_height, art_width) = compute_art_grid_row(inner.width, inner.height);

    if art_row_height == 0 {
        return;
    }

    // Spacer between consecutive items of *different* row-height
    // class — drops a blank row at the boundary between art-height
    // album rows and one-row pinned action rows. Works in either
    // direction (one-row first or art first), so flipping the order
    // of pinned rows to the bottom of the column still gets the
    // visual separator.
    let has_spacer_after = |idx: usize| -> bool {
        idx + 1 < total_items
            && is_one_row(items_with_indices[idx].1)
                != is_one_row(items_with_indices[idx + 1].1)
    };

    // Compute how many items are visible from a given scroll offset
    let count_visible_from = |offset: usize| -> usize {
        let mut y = 0u16;
        let mut count = 0;
        for i in offset..total_items {
            let h = if is_one_row(items_with_indices[i].1) { 1 } else { art_row_height };
            // Account for spacer row after last one-row item
            let spacer = if has_spacer_after(i) { 1u16 } else { 0 };
            if y + h + spacer > inner.height { break; }
            y += h + spacer;
            count += 1;
        }
        count
    };

    let selected_idx = col.selected_index;

    // Convert selected_idx to display position within the (possibly filtered) list
    let display_selected = if filter_results.is_some() {
        items_with_indices.iter().position(|(idx, _)| *idx == selected_idx).unwrap_or(0)
    } else {
        selected_idx
    };

    // Scroll offset: respect pin, otherwise ensure selected item is visible
    let scroll_offset = match state.scroll.browse {
        Some((pin_col, pinned)) if pin_col == col_idx => pinned,
        _ => {
            // Simple approach: start from 0, advance until selected is visible
            let mut offset = 0;
            loop {
                let visible = count_visible_from(offset);
                if visible == 0 { break; }
                if display_selected >= offset && display_selected < offset + visible {
                    break;
                }
                if display_selected < offset {
                    offset = display_selected;
                    break;
                }
                offset += 1;
            }
            offset
        }
    };

    let visible_count = count_visible_from(scroll_offset);

    let mut row_y = inner.y;
    for vis_row in 0..visible_count {
        let display_idx = scroll_offset + vis_row;
        if display_idx >= total_items {
            break;
        }

        let (orig_idx, item) = items_with_indices[display_idx];
        let is_selected = orig_idx == selected_idx;
        let one_row = is_one_row(item);
        let row_height = if one_row { 1 } else { art_row_height };

        if row_y + row_height > inner.y + inner.height {
            break;
        }

        // Selection highlight background across the full row
        if is_selected {
            let row_area = Rect {
                x: inner.x,
                y: row_y,
                width: inner.width,
                height: row_height,
            };
            let bg_style = Style::default().bg(t.colors.selection_bar_bg);
            frame.render_widget(Block::default().style(bg_style), row_area);
        }

        if one_row {
            // One-row item: full-width text, no art area
            let max_text = inner.width.saturating_sub(2) as usize;
            let display_title = item.title();
            let title_text = truncate_middle(display_title, max_text);
            let title_style = if is_selected {
                Style::default().fg(t.colors.selection_text)
            } else {
                Style::default().fg(t.colors.fg_muted)
            };
            frame.render_widget(
                Paragraph::new(format!(" {}", title_text)).style(title_style),
                Rect { x: inner.x, y: row_y, width: inner.width, height: 1 },
            );
        } else {
            // Art-height item: artwork on left, text on right
            let image_area = Rect {
                x: inner.x,
                y: row_y,
                width: art_width,
                height: row_height,
            };
            let text_x = inner.x + art_width + 1;
            let text_width = inner.width.saturating_sub(art_width + 1);

            // Render album/artist art image or placeholder
            let mut rendered_image = false;
            let art_key = match item {
                BrowseItem::Album { key, .. } => Some(key.as_str()),
                BrowseItem::Artist { key, .. } => Some(key.as_str()),
                BrowseItem::ArtistRadio { artist_key, .. } => Some(artist_key.as_str()),
                BrowseItem::AllTracks { scope, .. } => scope.artist_key(),
                _ => None,
            };
            if let Some(key) = art_key {
                if let Some(data) = state.artwork.grid_cache.get(key) {
                    rendered_image = super::artwork::render_grid_image(frame, image_area, key, data);
                }
            }

            if !rendered_image {
                // Placeholder: centered initials in art area
                let initials: String = item.title()
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(3)
                    .collect();
                let placeholder_text = if state.artwork.grid_pending.contains(item.key()) {
                    "...".to_string()
                } else if initials.is_empty() {
                    "?".to_string()
                } else {
                    initials
                };

                let text_y_p = image_area.y + image_area.height / 2;
                let text_x_p = image_area.x + (image_area.width.saturating_sub(placeholder_text.len() as u16)) / 2;
                if text_y_p < image_area.y + image_area.height {
                    frame.render_widget(
                        Paragraph::new(placeholder_text).style(Style::default().fg(t.colors.fg_muted)),
                        Rect { x: text_x_p, y: text_y_p, width: image_area.width, height: 1 },
                    );
                }
            }

            // Text content to the right of artwork
            if text_width > 2 {
                let max_text = text_width.saturating_sub(1) as usize;

                // Title (line 1, vertically centered in row)
                let display_title = item.title();
                let title_text = truncate_middle(display_title, max_text);
                let title_y = row_y + (row_height / 2).saturating_sub(1);
                let title_style = if is_selected {
                    Style::default().fg(t.colors.selection_text)
                } else {
                    Style::default().fg(t.colors.fg_primary)
                };
                frame.render_widget(
                    Paragraph::new(title_text).style(title_style),
                    Rect { x: text_x, y: title_y, width: text_width, height: 1 },
                );

                // Artist and year (line 2)
                if let BrowseItem::Album { artist, year, .. } = item {
                    let subtitle = if let Some(y) = year {
                        truncate_middle(&format!("{} ({})", artist, y), max_text)
                    } else {
                        truncate_middle(artist, max_text)
                    };
                    let sub_style = Style::default().fg(t.colors.fg_muted);
                    frame.render_widget(
                        Paragraph::new(subtitle).style(sub_style),
                        Rect { x: text_x, y: title_y + 1, width: text_width, height: 1 },
                    );
                }
            }
        }

        row_y += row_height;

        // Add spacer row after last one-row item before art items
        if has_spacer_after(display_idx) {
            row_y += 1;
        }
    }

    // Scrollbar + position indicator
    if total_items > visible_count {
        let sb_border = if is_focused { Some(t.colors.title_focused) } else { None };
        render_scrollbar(frame, col_area, total_items, visible_count, scroll_offset, sb_border);

        let footer = format!("{}/{}", display_selected + 1, total_items);
        let footer_area = Rect::new(
            col_area.x + col_area.width.saturating_sub(footer.len() as u16 + 2),
            col_area.y + col_area.height - 1,
            footer.len() as u16 + 1,
            1,
        );
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(t.colors.fg_muted)),
            footer_area,
        );
    }
}
/// Render the transport bar (always visible, never hijacked by alt bar).
fn render_transport(frame: &mut Frame, state: &AppState, area: Rect) {
    widgets::transport::render(frame, state, area);
}

/// Render the command bar (3 rows: top info/tabs + spacer + contextual commands).
///
/// Top row layout: [library name] [^Q quit] ... [F-keys] [^L library] [^U queue] [^N now playing]
fn render_library_picker(frame: &mut Frame, state: &AppState) {
    let t = theme();
    let area = centered_rect(50, 30, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" switch library ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Build flat list of all libraries across servers
    let multi_server = state.has_multiple_servers();
    let all_libs = if multi_server {
        state.all_libraries_with_servers()
    } else {
        // Single server — use current libraries
        let server_id = state.active_server_id.as_deref().unwrap_or("");
        let server_name = state.active_server_name().unwrap_or("");
        state.libraries.iter()
            .map(|lib| (server_id, server_name, lib))
            .collect()
    };

    if all_libs.is_empty() {
        let msg = Paragraph::new("No libraries available")
            .style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
        return;
    }

    // Register hit regions for mouse handler
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.library_picker = Some(crate::ui::hit_regions::PopupListRegions {
            outer: area,
            items_area: inner,
            item_count: all_libs.len(),
        });
    }

    // Build library list items
    let items: Vec<ListItem> = all_libs.iter().enumerate().map(|(i, (server_id, server_name, lib))| {
        let is_selected = i == state.popups.library_picker_index;
        let is_active = state.active_library.as_deref() == Some(lib.key.as_str())
            && state.active_server_id.as_deref() == Some(*server_id);

        let prefix = if is_selected { "\u{266a} " } else { "  " };
        let suffix = if is_active { " *" } else { "" };
        let text = if multi_server {
            format!("{}{} ({}){}", prefix, lib.title, server_name, suffix)
        } else {
            format!("{}{}{}", prefix, lib.title, suffix)
        };

        let style = if is_selected {
            Style::default().fg(t.colors.selection_text).bg(t.colors.selection_bar_bg)
        } else {
            Style::default().fg(t.colors.fg_primary)
        };

        ListItem::new(text).style(style)
    }).collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render the artist bio popup (F4).
fn render_artist_bio_popup(frame: &mut Frame, state: &AppState) {
    let popup = match &state.popups.artist_bio {
        Some(p) => p,
        None => return,
    };

    let t = theme();
    let area = centered_rect(70, 60, frame.area());

    frame.render_widget(Clear, area);

    let title = format!(" {} ", popup.artist_name);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if popup.loading {
        let loading = Paragraph::new("Loading biography...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    let bio_area = inner;

    // Determine artwork size and whether to show it
    let has_artwork = popup.artwork_data.is_some() && popup.artwork_thumb.is_some();
    // Drive layout from height: fill ~60% of bio area vertically, then derive width
    // from height so a square image fills the rect exactly (terminal cells ≈ 2:1 aspect).
    let art_h = if has_artwork {
        let target = (bio_area.height * 3) / 5; // ~60% of bio area
        target.max(6).min(bio_area.height.saturating_sub(2))
    } else { 0 };
    let art_w = if has_artwork { (art_h * 2).min(bio_area.width / 2) } else { 0 };
    // 1 col gap between text and artwork
    let gap = if has_artwork && art_w > 0 { 1u16 } else { 0 };

    // Word-wrap bio text: narrow lines next to artwork, full-width lines below
    let full_width = bio_area.width as usize;
    let narrow_width = bio_area.width.saturating_sub(art_w + gap) as usize;
    let art_rows = art_h as usize;
    let wrapped = wrap_bio_text(&popup.bio, narrow_width, full_width, art_rows);
    let total_lines = wrapped.len() as u16;
    let visible = bio_area.height;
    let scroll = popup.scroll.min(total_lines.saturating_sub(visible));

    // Render artwork scrolling with text: crop top rows as user scrolls down.
    let art_visible_h = art_h.saturating_sub(scroll);
    if has_artwork && art_w > 0 && art_visible_h > 0 {
        let art_rect = Rect {
            x: bio_area.x + bio_area.width - art_w,
            y: bio_area.y,
            width: art_w,
            height: art_visible_h,
        };
        if let (Some(ref data), Some(ref thumb)) = (&popup.artwork_data, &popup.artwork_thumb) {
            BIO_ARTWORK_RENDERER.with(|renderer| {
                let mut renderer = renderer.borrow_mut();
                let crop_fraction = if scroll > 0 { scroll as f32 / art_h as f32 } else { 0.0 };
                if renderer.load_image_cropped(data, thumb, crop_fraction) {
                    renderer.render(frame, art_rect);
                }
            });
        }
    }

    // Render visible text lines
    let style = Style::default().fg(t.colors.fg_primary);
    for (screen_row, line_text) in wrapped.iter().skip(scroll as usize).take(visible as usize).enumerate() {
        let y = bio_area.y + screen_row as u16;
        // Narrow width only when artwork is visible on this screen row
        let in_art_zone = has_artwork && (screen_row as u16) < art_visible_h;
        let line_width = if in_art_zone { narrow_width as u16 } else { bio_area.width };
        let line_rect = Rect {
            x: bio_area.x,
            y,
            width: line_width,
            height: 1,
        };
        let p = Paragraph::new(line_text.as_str()).style(style);
        frame.render_widget(p, line_rect);
    }

    // Scrollbar
    if total_lines > visible {
        render_scrollbar(frame, area, total_lines as usize, visible as usize, scroll as usize, None);
    }
}

/// Word-wrap bio text with a narrow region (next to artwork) and full-width below.
/// The first `narrow_rows` output lines are wrapped at `narrow_width`;
/// subsequent lines are wrapped at `full_width`.
fn wrap_bio_text(text: &str, narrow_width: usize, full_width: usize, narrow_rows: usize) -> Vec<String> {
    if full_width == 0 {
        return vec![];
    }
    // If no artwork, everything is full width
    let narrow_width = if narrow_width == 0 || narrow_width >= full_width { full_width } else { narrow_width };

    let mut lines = Vec::new();

    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }

        let words: Vec<&str> = paragraph.split_whitespace().collect();
        let mut line = String::new();

        for word in &words {
            let max_w = if lines.len() < narrow_rows { narrow_width } else { full_width };
            if line.is_empty() {
                line.push_str(word);
            } else if line.len() + 1 + word.len() <= max_w {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }

    lines
}

fn render_error_popup(frame: &mut Frame, error: &str) {
    let t = theme();
    let area = centered_rect(60, 20, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Error ")
        .title_style(Style::default().fg(t.colors.error))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.error))
        .style(Style::default().bg(t.colors.bg_primary));

    let text = Paragraph::new(error)
        .style(Style::default().fg(t.colors.error))
        .wrap(Wrap { trim: true })
        .block(block);

    frame.render_widget(text, area);
}

fn render_input_dialog(frame: &mut Frame, dialog: &InputDialog) {
    let t = theme();
    // Use 50% width and 25% height to ensure the dialog is visible
    let area = centered_rect(50, 25, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner area for input and hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // input label
            Constraint::Length(1),  // input field
            Constraint::Length(1),  // hint
        ])
        .split(inner);

    // Input field with cursor
    let input_text = format!("{}▋", dialog.input);
    let input = Paragraph::new(input_text)
        .style(Style::default().fg(t.colors.fg_primary));
    frame.render_widget(input, chunks[1]);

    // Hint text
    let hint = Paragraph::new("Enter: Save  |  Esc: Cancel")
        .style(Style::default().fg(t.colors.fg_muted))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn render_confirm_dialog(frame: &mut Frame, state: &AppState, dialog: &ConfirmDialog) {
    let t = theme();
    let area = centered_rect(50, 25, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Message
    let msg = Paragraph::new(dialog.message.as_str())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: true });
    let msg_area = Rect { height: inner.height.saturating_sub(2), ..inner };
    frame.render_widget(msg, msg_area);

    // Button row at bottom of inner area
    let btn_y = inner.y + inner.height.saturating_sub(1);
    let yes_text = "  Yes  ";
    let no_text = "  No  ";
    let yes_x = inner.x + 1;
    let no_x = yes_x + yes_text.len() as u16 + 2;

    let yes_area = Rect { x: yes_x, y: btn_y, width: yes_text.len() as u16, height: 1 };
    let no_area = Rect { x: no_x, y: btn_y, width: no_text.len() as u16, height: 1 };

    // Register hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.confirm_dialog = Some(crate::ui::hit_regions::DialogRegions {
            outer: area,
            yes_button: yes_area,
            no_button: no_area,
        });
    }

    // Highlight the selected button with accent, dim the other
    let (yes_style, no_style) = if dialog.selected_yes {
        (
            Style::default().fg(t.colors.bg_primary).bg(t.colors.fg_accent),
            Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
        )
    } else {
        (
            Style::default().fg(t.colors.fg_muted).bg(t.colors.bg_secondary),
            Style::default().fg(t.colors.bg_primary).bg(t.colors.fg_accent),
        )
    };

    frame.render_widget(Paragraph::new(yes_text).style(yes_style), yes_area);
    frame.render_widget(Paragraph::new(no_text).style(no_style), no_area);

    // Hint text
    let hint_y = btn_y.saturating_sub(1);
    if hint_y > inner.y + 1 {
        let hint = Paragraph::new("Y/N or Enter to confirm")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        let hint_area = Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 };
        frame.render_widget(hint, hint_area);
    }
}

/// Check if a mouse click hit a confirm dialog button. Returns Some(true) for Yes, Some(false) for No, None for miss.
pub fn confirm_dialog_hit_test(dialog: &ConfirmDialog, frame_area: Rect, col: u16, row: u16) -> Option<bool> {
    let area = centered_rect(50, 25, frame_area);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);

    let btn_y = inner.y + inner.height.saturating_sub(1);
    if row != btn_y { return None; }

    let yes_text = "  Yes  ";
    let no_text = "  No  ";
    let yes_x = inner.x + 1;
    let no_x = yes_x + yes_text.len() as u16 + 2;

    let _ = dialog; // used for lifetime/future extensibility
    if col >= yes_x && col < yes_x + yes_text.len() as u16 {
        Some(true)
    } else if col >= no_x && col < no_x + no_text.len() as u16 {
        Some(false)
    } else {
        None
    }
}

fn render_toast(frame: &mut Frame, message: &str, area: Rect) {
    let t = theme();
    let padded_message = format!(" {} ", message);
    let width = (padded_message.len().min(50)) as u16;

    let toast_area = Rect {
        x: area.width.saturating_sub(width + 1),
        y: area.height.saturating_sub(4), // Above transport bar
        width,
        height: 1,
    };

    frame.render_widget(Clear, toast_area);
    let text = Paragraph::new(padded_message)
        .style(Style::default()
            .fg(t.colors.fg_primary)
            .bg(t.colors.fg_accent));
    frame.render_widget(text, toast_area);
}
