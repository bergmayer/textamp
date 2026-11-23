//! Terminal library, Now Playing, and independent right-side queue panels.
//! The transport remains within the main workspace; queue controls sit beneath its list.
//! Sidebar disclosures, settings and artist links dispatch shared app actions.

use std::cell::{Cell, RefCell};

use super::artwork::ArtworkRenderer;
use super::layout::{AppLayout, FullScreenLayout, centered_rect};
use super::screens;
use super::theme::theme;
use super::widgets;
use super::widgets::scrollbar::render_scrollbar;
use crate::app::state::{ConfirmDialog, InputDialog, View};
use crate::services::NavigationService;
use crate::ui::RenderState as AppState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

thread_local! {
    static BIO_ARTWORK_RENDERER: RefCell<ArtworkRenderer> = RefCell::new(ArtworkRenderer::new());
    static LAST_FULLSCREEN: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
    static LAST_RENDERED_VIEW: Cell<Option<View>> = const { Cell::new(None) };
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
    LAST_FULLSCREEN.with(|last| {
        let signature = (
            state.fullscreen.is_open(),
            state.fullscreen.is_artwork(),
            state.fullscreen.is_open() && state.fullscreen.is_artwork() && state.palette.open,
        );
        if last.replace(signature) != signature {
            super::artwork::reset_grid_placements();
            screens::now_playing::clear_artwork_cache();
        }
    });
    LAST_RENDERED_VIEW.with(|last| {
        if last.get() == Some(View::Similar) && state.view != View::Similar {
            // Recreate terminal placements obscured by a full-screen discovery surface.
            super::artwork::reset_grid_placements();
        }
        last.set(Some(state.view));
    });

    // Clear hit-test registry for this frame
    state.hit_regions.borrow_mut().clear();

    // Fill entire background with theme color
    let t = theme();
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        frame.area(),
    );

    // Discovery dialogs retain the workspace behind their overlay.
    let mut background_view =
        if matches!(state.view, View::Similar | View::Related | View::Settings) {
            state.previous_view.unwrap_or(View::Browse)
        } else {
            state.view
        };
    if background_view == View::Queue {
        background_view = state.tui_queue_workspace.view();
    }
    if matches!(
        background_view,
        View::Settings | View::Similar | View::Related
    ) {
        background_view = View::Browse;
    }
    let mut workspace = frame.area();
    if !state.fullscreen.is_open() {
        workspace.height = workspace.height.saturating_sub(2);
    }
    let now = std::time::Instant::now();
    let queue_progress = state.tui_motion.queue(state.tui_queue_visible, now);
    if queue_progress > 0.
        && matches!(background_view, View::Browse | View::NowPlaying)
        && !state.fullscreen.is_open()
    {
        let width = state
            .sidebar_width()
            .min(workspace.width.saturating_sub(24));
        let width = (f32::from(width) * queue_progress).round() as u16;
        let columns =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).split(workspace);
        workspace = columns[0];
        screens::now_playing::render_queue_mode(frame, state, columns[1]);
    }
    if state.fullscreen.is_open() {
        super::fullscreen::render(frame, state, frame.area());
    } else if state.tui_motion.player_active(now)
        && matches!(background_view, View::Browse | View::NowPlaying)
    {
        let queue_regions = state.hit_regions.borrow().clone();
        render_browse_in(frame, state, workspace);
        super::widgets::column_header::render(frame, state);
        let progress = state
            .tui_motion
            .player(background_view == View::NowPlaying, now);
        let height = (f32::from(workspace.height) * progress).round() as u16;
        let overlay = Rect::new(
            workspace.x,
            workspace.bottom().saturating_sub(height),
            workspace.width,
            height,
        );
        let library = frame.buffer_mut().clone();
        *state.hit_regions.borrow_mut() = queue_regions;
        frame.render_widget(Clear, workspace);
        render_queue_and_visualizer_in(frame, state, workspace);
        let player = std::mem::replace(frame.buffer_mut(), library);
        for y in 0..height {
            for x in workspace.x..workspace.right() {
                frame.buffer_mut()[(x, overlay.y + y)] = player[(x, workspace.y + y)].clone();
            }
        }
        let translate = |rect: Rect| {
            Rect::new(
                rect.x,
                rect.y.saturating_add(workspace.height - height),
                rect.width,
                rect.height,
            )
            .intersection(workspace)
        };
        let mut regions = state.hit_regions.borrow_mut();
        regions.player_toggle = regions.player_toggle.map(translate);
        if let Some(transport) = &mut regions.transport {
            transport.area = translate(transport.area);
            transport.play_pause = translate(transport.play_pause);
            transport.seekbar = translate(transport.seekbar);
            transport.prev_track = translate(transport.prev_track);
            transport.next_track = translate(transport.next_track);
            transport.track_info = transport.track_info.map(translate);
            transport.speaker_icon = transport.speaker_icon.map(translate);
            transport.volume_slider = transport.volume_slider.map(translate);
            for (rect, _) in &mut transport.controls {
                *rect = translate(*rect);
            }
            transport.controls.retain(|(rect, _)| !rect.is_empty());
        }
        if let Some(visualizer) = &mut regions.now_playing_content {
            visualizer.visualizer_tab_area = translate(visualizer.visualizer_tab_area);
            visualizer.visualizer_content_area = translate(visualizer.visualizer_content_area);
        }
        for link in &mut regions.artist_links {
            if link.area.x < workspace.right() {
                link.area = translate(link.area);
            }
        }
        regions.artist_links.retain(|link| !link.area.is_empty());
        for link in &mut regions.detail_links {
            if link.area.x < workspace.right() {
                link.area = translate(link.area);
            }
        }
        regions.detail_links.retain(|link| !link.area.is_empty());
        regions.player_overlay = Some(overlay);
    } else {
        match background_view {
            View::Browse => render_browse_in(frame, state, workspace),
            View::Queue => render_queue_and_visualizer_in(frame, state, workspace),
            View::NowPlaying => render_queue_and_visualizer_in(frame, state, workspace),
            View::Search => render_search(frame, state, workspace),
            View::Similar | View::Related => render_browse_in(frame, state, workspace),
            View::Help => render_help(frame, state, workspace),
            View::Settings => render_settings(frame, state, workspace),
        }
    }

    if background_view == View::Browse
        && !state.fullscreen.is_open()
        && !state.tui_motion.player_active(now)
    {
        super::widgets::column_header::render(frame, state);
    }

    if !state.fullscreen.is_open() {
        let area = frame.area();
        widgets::transport::render_footer(
            frame,
            state,
            Rect::new(
                area.x,
                area.bottom().saturating_sub(2),
                area.width,
                area.height.min(2),
            ),
        );
    }

    match state.view {
        // Similar owns its inline failure state and clickable F5 retry footer.
        View::Similar => screens::similar::render(frame, state, frame.area()),
        View::Related => screens::related::render(frame, state, frame.area()),
        _ => {}
    }

    if !state.overlays {
        return;
    }

    if state.view == View::Settings {
        state.hit_regions.borrow_mut().clear();
        render_settings(frame, state, frame.area());
    }

    // Render search popup if active (floating dialog)
    if state.popups.search_active() {
        screens::filter::render(frame, state, frame.area());
    }

    // Radio/queue edit controls use the shared capability-filtered command palette.
    // Shortcut labels and activation use the same presentation rows.
    // Render adventure launcher popup if active
    if state.popups.adventure_launcher().is_some()
        && !crate::app::radio_navigation::inline_input(state)
    {
        screens::adventure_launcher::render(frame, state, frame.area());
    }

    // Render artist radio picker popup if active
    if state.popups.artist_radio_picker().is_some()
        && !crate::app::radio_navigation::inline_input(state)
    {
        screens::artist_radio_picker::render(frame, state, frame.area());
    }

    // Render library picker popup if active
    if state.popups.library_picker_active() {
        render_library_picker(frame, state);
    }

    // Render sort popup if active
    if state.popups.sort().is_some() {
        screens::sort_popup::render(frame, state, frame.area());
    }

    // Render artist bio popup if active. When it's NOT active but the
    // renderer is still holding an image, drop the protocol so the
    // kitty/iterm2 delete-image command fires on the next terminal
    // write — otherwise the artwork (and the bio text the diff treats
    // as `skip` beneath it) lingers after every non-Esc dismissal
    // path (e.g. `popups.close_all()` when another popup opens).
    if state.popups.artist_bio().is_some() {
        render_artist_bio_popup(frame, state);
    } else if BIO_ARTWORK_RENDERER.with(|r| r.borrow().has_image()) {
        BIO_ARTWORK_RENDERER.with(|r| r.borrow_mut().clear());
    }

    if let Some(panel) = state.popups.lyrics() {
        let view = crate::app::lyrics::view(state).unwrap();
        let area = centered_rect(70, 80, frame.area());
        frame.render_widget(Clear, area);
        super::widgets::register_popup_close(state, area);
        let mut lines = vec![];
        if let Some(message) = &view.message {
            lines.push(Line::from(message.clone()));
        }
        for line in &view.lines {
            lines.push(Line::from(
                line.spans
                    .iter()
                    .map(|span| {
                        Span::styled(
                            span.text.clone(),
                            Style::default()
                                .fg(if line.active {
                                    t.colors.fg_accent
                                } else {
                                    t.colors.fg_primary
                                })
                                .add_modifier(if span.active {
                                    Modifier::BOLD | Modifier::UNDERLINED
                                } else if line.active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        )
                    })
                    .collect::<Vec<_>>(),
            ));
        }
        let width = area.width.saturating_sub(2);
        let total = Paragraph::new(lines.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        let active_row = view
            .active
            .map(|index| {
                Paragraph::new(lines[..index].to_vec())
                    .wrap(Wrap { trim: false })
                    .line_count(width)
            })
            .unwrap_or(0);
        let scroll = if view.follow {
            active_row.saturating_sub(usize::from(area.height) / 3)
        } else {
            usize::from(panel.scroll)
        };
        let scroll = scroll
            .min(total.saturating_sub(usize::from(area.height.saturating_sub(2))))
            .min(usize::from(u16::MAX)) as u16;
        let language = view
            .languages
            .get(view.language)
            .map(String::as_str)
            .unwrap_or("");
        frame.render_widget(
            Paragraph::new(lines)
                .style(
                    Style::default()
                        .fg(t.colors.fg_primary)
                        .bg(t.colors.bg_primary),
                )
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("{} · {}", view.title, language))
                        .title_bottom(format!(
                            " S follow: {} · L language · R reload · G web · ↑↓ scroll · Esc close ",
                            if view.follow { "on" } else { "off" }
                        )),
                ),
            area,
        );
    }

    if let Some(popup) = state.popups.text() {
        let area = centered_rect(70, 70, frame.area());
        frame.render_widget(Clear, area);
        super::widgets::register_popup_close(state, area);
        frame.render_widget(
            Paragraph::new(popup.text.as_str())
                .style(
                    Style::default()
                        .fg(t.colors.fg_primary)
                        .bg(t.colors.bg_primary),
                )
                .wrap(Wrap { trim: false })
                .scroll((popup.scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(popup.title.as_str())
                        .title_bottom(if popup.lyrics_query.is_some() {
                            " G search Google · ↑↓ scroll · Esc close "
                        } else {
                            " ↑↓ scroll · Esc close "
                        }),
                ),
            area,
        );
    }

    // Render error popup if present
    screens::library_dialog::render(frame, state);
    if let Some(ref error) = state.notifications.last_error {
        render_error_popup(frame, state, error);
    }

    // Render input dialog if present
    if let Some(dialog) = state.popups.input_dialog() {
        render_input_dialog(frame, state, dialog);
    }

    // Render confirm dialog if present
    if let Some(dialog) = state.popups.confirm_dialog() {
        render_confirm_dialog(frame, state, dialog);
    }

    if let Some(focus) = &state.tui_artist_focus
        && let Some(link) = state.hit_regions.borrow().artist_links.iter().find(|link| {
            link.key == focus.key
                && link.area == focus.area
                && link.generation == state.library_generation
        })
    {
        let area = link.area.intersection(frame.area());
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                frame.buffer_mut()[(x, y)]
                    .set_style(t.colors.focused_row().add_modifier(Modifier::UNDERLINED));
            }
        }
    }

    // Command-palette overlay rides ABOVE every other layer so the
    // user can summon it from any view (including over a popup).
    if state.palette.open {
        crate::ui::command_palette::render(frame, state, frame.area());
    }
    if let Some(rect) = state.hit_regions.borrow().popup_close {
        frame.render_widget(
            Paragraph::new("×").style(
                Style::default()
                    .fg(t.colors.fg_accent)
                    .bg(t.colors.bg_primary),
            ),
            rect,
        );
    }
}

fn render_player_sync_column(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::player_sync;
    let colors = &theme().colors;
    let focused = !state.category_column_focused;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Audio Player ")
        .border_style(Style::default().fg(if focused {
            colors.border_focused
        } else {
            colors.border
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    state.hit_regions.borrow_mut().player_sync_column = Some(inner);
    let mut details = vec![player_sync::DESCRIPTION.to_owned()];
    if player_sync::webdav_filter(state).is_some() {
        details.push("/ Filter · Enter Open · Space Select · Left Back".into());
    }
    if let Some(size) = player_sync::playlist_size_label(state) {
        details.push(size);
    }
    let status = player_sync::status(state);
    if !status.is_empty() {
        details.push(status);
    }
    let details_height = (details.len() as u16).min(inner.height.saturating_sub(1));
    for (index, detail) in details
        .into_iter()
        .take(usize::from(details_height))
        .enumerate()
    {
        frame.render_widget(
            Paragraph::new(detail).style(Style::default().fg(colors.fg_secondary)),
            Rect::new(inner.x, inner.y + index as u16, inner.width, 1),
        );
    }
    let rows = player_sync::column_rows(state);
    let active = state
        .player_sync
        .column_cursor
        .min(rows.len().saturating_sub(1));
    let height = usize::from(inner.height.saturating_sub(details_height));
    let offset = (active + 1).saturating_sub(height);
    for (index, (label, command)) in rows.into_iter().enumerate().skip(offset).take(height) {
        let rect = Rect::new(
            inner.x,
            inner.y + details_height + (index - offset) as u16,
            inner.width,
            1,
        );
        let style = if focused && index == active {
            Style::default()
                .fg(colors.selection_text)
                .bg(colors.bg_selection)
        } else if command.is_some() {
            Style::default().fg(colors.fg_primary)
        } else {
            Style::default().fg(colors.fg_muted)
        };
        if let Some(command) = command {
            state
                .hit_regions
                .borrow_mut()
                .player_sync_rows
                .push((rect, index, command));
        }
        frame.render_widget(Paragraph::new(label).style(style), rect);
    }
}

fn render_browse_in(frame: &mut Frame, state: &AppState, area: Rect) {
    use crate::app::state::BrowseCategory;

    let layout = AppLayout::new(area);

    // Full area for all columns (combine left + right panels)
    let full_area = Rect {
        x: layout.left_panel.x,
        y: layout.left_panel.y,
        width: layout.left_panel.width + layout.right_panel.width,
        height: layout.left_panel.height,
    };

    // The sidebar has a stable width; Miller and detail columns share the remaining space.
    const MIN_COL_WIDTH: u16 = 26;

    let t = theme();
    let category_rows = state.category_rows();
    let sidebar = crate::app::sidebar::sidebar(state);
    let sidebar_indices =
        crate::app::sidebar::visible_indices(&sidebar, &state.desktop_collapsed_sections);

    let search_selected = state.category_column_focused
        && matches!(
            category_rows.get(state.category_column_index),
            Some(crate::app::state::CategoryRow::Search)
        );
    let n_meaningful_miller = if search_selected {
        1
    } else {
        count_meaningful_miller_cols(state)
    };

    // Track-details pane: counted as a "column slot" so it gets
    // the same width as everything else. Visible whenever the
    // focused Miller row is a Track AND the pane hasn't been
    // hidden via Ctrl+W for that specific track.
    let pane_track = if search_selected {
        None
    } else {
        state.pane_track()
    };
    let child = crate::app::sonic_details::visible(state);
    let n_pane = usize::from(pane_track.is_some()) + usize::from(child.is_some());

    let sidebar_width = (f32::from(state.sidebar_width().min(full_area.width / 3))
        * state
            .tui_motion
            .sidebar(state.tui_sidebar_visible, std::time::Instant::now()))
    .round() as u16;
    let category_base_offset = state.browse_column_offset();
    let show_alphabet_strip = !search_selected && state.alphabet_strip_visible();
    let strip_width = 0;
    let usable_width = full_area.width.saturating_sub(sidebar_width + strip_width);
    let ribbon_total = (n_meaningful_miller + n_pane).max(1);
    let ribbon_visible = ribbon_total.min((usable_width / MIN_COL_WIDTH).max(1) as usize);
    let col_width = usable_width / ribbon_visible as u16;
    let visible_count = usize::from(sidebar_width > 0) + ribbon_visible;
    let scrolling_browse = ribbon_total > ribbon_visible;
    let ribbon_focused = if state.track_pane_focused {
        ribbon_total.saturating_sub(if crate::app::sonic_details::focused(state) {
            1
        } else {
            n_pane
        })
    } else if state.category_column_focused || state.alphabet_strip_focused {
        0
    } else if let Some(nav) = state.browse_nav() {
        nav.focused_column.saturating_sub(category_base_offset)
    } else {
        state
            .folder_state
            .as_ref()
            .map(|folders| folders.focused_column)
            .unwrap_or(0)
    };
    let added_column = state
        .hit_regions
        .borrow()
        .miller_viewport
        .is_some_and(|(previous, _)| ribbon_total > previous);
    let ribbon_start = if added_column || state.miller_reveal_right {
        ribbon_total.saturating_sub(ribbon_visible)
    } else if state.miller_scroll_manual {
        state
            .miller_scroll_col
            .min(ribbon_total.saturating_sub(ribbon_visible))
    } else {
        (ribbon_focused + 1)
            .saturating_sub(ribbon_visible)
            .min(ribbon_total.saturating_sub(ribbon_visible))
    };
    state.hit_regions.borrow_mut().miller_viewport = Some((ribbon_total, ribbon_start));
    let ribbon_end = ribbon_start + ribbon_visible;
    let cat_col_renders = sidebar_width > 0;

    // Render the category column if visible
    if cat_col_renders {
        let col_area = Rect {
            x: full_area.x,
            y: full_area.y,
            width: sidebar_width,
            height: full_area.height,
        };

        let is_focused = state.view == View::Browse
            && state.category_column_focused
            && state.tui_transport_focus.is_none()
            && state.tui_footer_focus.is_none()
            && state.tui_artist_focus.is_none();
        let border_color = if is_focused {
            t.colors.border_focused
        } else {
            t.colors.fg_muted
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Thick)
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.colors.bg_secondary));
        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);

        // Register category column hit region
        let category_scroll = state
            .scroll
            .category
            .unwrap_or_else(|| {
                NavigationService::calc_scroll_offset(
                    state.category_column_index,
                    inner.height as usize,
                    category_rows.len(),
                )
            })
            .min(category_rows.len().saturating_sub(inner.height as usize));
        {
            let mut hr = state.hit_regions.borrow_mut();
            hr.category_column = Some(crate::app::presentation::CategoryColumnRegion {
                scroll_offset: category_scroll,
                area: col_area,
                inner,
                item_count: category_rows.len(),
            });
        }

        // Labelled headings scroll with their rows; clicks use the published offset.
        for (i, row) in category_rows.iter().enumerate().skip(category_scroll) {
            let y_offset = (i - category_scroll) as u16;
            if y_offset >= inner.height {
                break;
            }

            let (label, is_active) = match row {
                crate::app::state::CategoryRow::Search => ("Search…".to_owned(), false),
                crate::app::state::CategoryRow::Category(cat) => {
                    let label = cat.display_label();
                    let active =
                        state.browse_category == *cat && state.sources.nav_collection.is_none();
                    (label.to_string(), active)
                }
                crate::app::state::CategoryRow::Playlist(idx) => {
                    let p = match state.library.playlists.get(*idx) {
                        Some(p) => p,
                        None => continue,
                    };
                    let active = state.browse_category == BrowseCategory::Playlists
                        && state
                            .playlist_nav
                            .columns
                            .first()
                            .and_then(|c| c.items.get(c.selected_index))
                            .map(|it| it.key() == p.rating_key.as_str())
                            .unwrap_or(false);
                    // Force text-presentation for hearts so the
                    // playlist label paints in the column's foreground
                    // color instead of the colorful emoji glyph.
                    (crate::util::force_text_presentation(&p.title), active)
                }
                crate::app::state::CategoryRow::Station(i) => {
                    let Some(station) = state
                        .station_nav
                        .columns
                        .first()
                        .and_then(|col| col.stations.get(*i))
                    else {
                        continue;
                    };
                    (
                        format!("  {}", station.title),
                        state.desktop_library_root == crate::app::state::LibraryRoot::Radio
                            && state.station_detail.as_ref() == Some(&station.key),
                    )
                }
                crate::app::state::CategoryRow::NavidromeCollection(kind) => (
                    kind.label().to_owned(),
                    state.browse_category == BrowseCategory::Library
                        && state.sources.nav_collection == Some(*kind),
                ),
                crate::app::state::CategoryRow::Root(root) => (
                    (if *root == crate::app::state::LibraryRoot::Albums {
                        "  All"
                    } else {
                        root.label()
                    })
                    .to_owned(),
                    state.browse_category == BrowseCategory::Library
                        && state.desktop_library_root == *root
                        && state.sources.nav_collection.is_none(),
                ),
                crate::app::state::CategoryRow::Header(title) => {
                    let closed = state
                        .desktop_collapsed_sections
                        .iter()
                        .any(|item| item == title);
                    let arrow = if !crate::app::sidebar::allows_disclosure(&sidebar, title) {
                        " "
                    } else if closed {
                        "▸"
                    } else {
                        "▾"
                    };
                    (format!("{arrow} {title}"), false)
                }
            };

            let label = if let Some(projected) = sidebar_indices
                .get(i)
                .map(|index| &sidebar[*index])
                .filter(|row| !row.header)
            {
                format!(
                    "{}{}",
                    if projected.parent.is_some() {
                        "    "
                    } else {
                        "  "
                    },
                    projected.label
                )
            } else {
                label
            };
            let is_selected = i == state.category_column_index;
            let show_cursor = is_focused && is_selected;
            let show_active = !is_focused && is_active;
            let style = if show_cursor {
                t.colors.focused_row()
            } else if show_active {
                t.colors.inactive_row()
            } else {
                Style::default()
                    .fg(t.colors.fg_primary)
                    .bg(t.colors.bg_secondary)
            };

            let indicator = "";
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
            let line_area = Rect {
                x: inner.x,
                y: inner.y + y_offset,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(padded).style(style), line_area);
        }
        if category_rows.len() > inner.height as usize {
            render_scrollbar(
                frame,
                col_area,
                category_rows.len(),
                inner.height as usize,
                category_scroll,
                Some(border_color),
            );
        }
    }

    if crate::app::radio_navigation::visible(state) {
        screens::radio::render(
            frame,
            state,
            Rect::new(
                full_area.x + sidebar_width,
                full_area.y,
                full_area.width.saturating_sub(sidebar_width),
                full_area.height,
            ),
        );
        return;
    }
    let full_area = if crate::app::tui_navigation::describing(state) {
        let toolbar = Rect::new(
            full_area.x + sidebar_width,
            full_area.y,
            full_area.width.saturating_sub(sidebar_width),
            3.min(full_area.height),
        );
        let field = Rect::new(
            toolbar.x,
            toolbar.y,
            toolbar.width.saturating_sub(10),
            toolbar.height,
        );
        let submit = Rect::new(
            field.right(),
            toolbar.y + u16::from(toolbar.height > 1),
            toolbar.width.saturating_sub(field.width),
            u16::from(toolbar.height > 0),
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if state.tui_description_focused {
                t.colors.border_focused
            } else {
                t.colors.border
            }));
        let inner = block.inner(field);
        frame.render_widget(block, field);
        frame.render_widget(
            Paragraph::new(if state.tui_description_query.is_empty() {
                "Describe music…"
            } else {
                &state.tui_description_query
            })
            .style(Style::default().fg(t.colors.fg_primary)),
            inner,
        );
        widgets::text_input::render(
            frame,
            state,
            inner,
            crate::app::terminal_text::Target::Description,
            Style::default()
                .fg(t.colors.fg_primary)
                .bg(t.colors.bg_primary),
        );
        frame.render_widget(
            Paragraph::new("[ Search ]").style(if state.tui_description_submit_focused {
                t.colors.focused_row()
            } else {
                Style::default().fg(t.colors.fg_accent)
            }),
            submit,
        );
        state.hit_regions.borrow_mut().description_input = Some(field);
        state.hit_regions.borrow_mut().description_submit = Some(submit);
        Rect::new(
            full_area.x,
            full_area.y + toolbar.height,
            full_area.width,
            full_area.height.saturating_sub(toolbar.height),
        )
    } else {
        full_area
    };

    if let Some((title, selected)) = &state.tui_sidebar_group {
        let area = Rect::new(
            full_area.x + sidebar_width,
            full_area.y,
            full_area.width.saturating_sub(sidebar_width),
            full_area.height,
        );
        let block = Block::default()
            .title(if title == "Playlists" {
                state.playlist_group_filter.as_ref().map_or_else(
                    || " Playlists · / Filter ".to_owned(),
                    |query| format!(" Filter Playlists: {query} "),
                )
            } else {
                format!(" {title} ")
            })
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.colors.border_focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = crate::app::sidebar::filtered_group_rows(
            state,
            title,
            state
                .playlist_group_filter
                .as_deref()
                .filter(|_| title == "Playlists")
                .unwrap_or_default(),
        );
        let scroll =
            NavigationService::calc_scroll_offset(*selected, inner.height as usize, rows.len());
        for (index, (_, row)) in rows
            .iter()
            .enumerate()
            .skip(scroll)
            .take(inner.height as usize)
        {
            let rect = Rect::new(inner.x, inner.y + (index - scroll) as u16, inner.width, 1);
            let style = if index == *selected && !state.category_column_focused {
                t.colors.focused_row()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            frame.render_widget(
                Paragraph::new(crate::util::truncate_middle(
                    &row.label,
                    rect.width as usize,
                ))
                .style(style),
                rect,
            );
        }
        if area.width >= 4 {
            frame.render_widget(
                Paragraph::new("×").style(Style::default().fg(t.colors.fg_accent)),
                Rect::new(area.right() - 3, area.y, 1, 1),
            );
        }
        state.hit_regions.borrow_mut().sidebar_group = Some((area, scroll));
        render_transport(frame, state, layout.transport);
        return;
    }

    if crate::app::player_sync::visible(state) {
        let panel = Rect::new(
            full_area.x + sidebar_width,
            full_area.y,
            full_area.width.saturating_sub(sidebar_width),
            full_area.height,
        );
        render_player_sync_column(frame, state, panel);
        render_transport(frame, state, layout.transport);
        return;
    }

    // Search is a transient landing panel, not a new navigation stack:
    // [ Search / Browse sidebar ][ themed ANSI logo + Enter prompt ]
    // Retain the previous stack, but do not render/register its hit targets.
    if search_selected {
        let panel = Rect::new(
            full_area.x + sidebar_width,
            full_area.y,
            full_area.width.saturating_sub(sidebar_width),
            full_area.height,
        );
        state.hit_regions.borrow_mut().search_landing = Some(panel);
        widgets::logo::render_search(frame, panel, &t.colors);
        render_transport(frame, state, layout.transport);
        return;
    }

    let cat_w = if cat_col_renders { sidebar_width } else { 0 };

    let pane_renders = n_pane > 0
        && (!scrolling_browse || {
            let idx = ribbon_total - n_pane;
            idx >= ribbon_start && idx < ribbon_end
        });
    let child_renders = child.is_some()
        && (!scrolling_browse || {
            let idx = ribbon_total - 1;
            idx >= ribbon_start && idx < ribbon_end
        });
    let pane_w = if pane_renders { col_width } else { 0 };
    let n_visible_miller = visible_count
        .saturating_sub(usize::from(cat_col_renders))
        .saturating_sub(usize::from(pane_renders) + usize::from(child_renders));
    let miller_w = (n_visible_miller as u16) * col_width;

    let content_area_x_offset = strip_width;
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

    let column_offset =
        state.browse_column_offset() + if scrolling_browse { ribbon_start } else { 0 };
    match state.browse_category {
        BrowseCategory::Folders => render_folder_view(frame, state, content_area, Some(col_width)),
        category => {
            if let Some(nav) = state.browse_nav() {
                let title = if category == BrowseCategory::Library {
                    state.sources.nav_collection.map_or("artists", |kind| {
                        if kind
                            == crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(
                                crate::audiomuse::Feature::Describe,
                            )
                        {
                            "Description history"
                        } else {
                            kind.label()
                        }
                    })
                } else {
                    category.name()
                };
                render_browse_miller_columns(
                    frame,
                    state,
                    nav,
                    title,
                    content_area,
                    Some(col_width),
                    column_offset,
                );
            }
        }
    }

    // Track details pane (when focused row is a Track).
    if let (Some(area), Some(track)) = (pane_area, pane_track) {
        render_track_details_pane(frame, state, area, track, None);
    }

    if child_renders && let Some(child) = child {
        let area = Rect {
            x: content_area.x + miller_w + pane_w,
            y: full_area.y,
            width: col_width,
            height: full_area.height,
        };
        let parent_regions = state.hit_regions.borrow_mut().track_pane.take();
        render_track_details_pane(
            frame,
            state,
            area,
            &child.tracks[child.index],
            Some(&child.details),
        );
        let mut regions = state.hit_regions.borrow_mut();
        regions.sonic_details = regions.track_pane.take();
        regions.track_pane = parent_regions;
    }

    if show_alphabet_strip {
        let artist_area = state
            .hit_regions
            .borrow()
            .miller_columns
            .as_ref()
            .and_then(|regions| regions.columns.iter().find(|column| column.col_idx == 0))
            .map(|column| column.area);
        if let Some(area) = artist_area.filter(|area| area.width >= 8) {
            render_alphabet_strip(
                frame,
                state,
                Rect::new(
                    area.right() - 3,
                    area.y + 1,
                    1,
                    area.height.saturating_sub(2),
                ),
            );
        }
    }

    // Horizontal scrollbar across the bottom row of the browse band.
    // Half-height: rail uses the "lower one eighth block" glyph, thumb
    // uses the "lower half block" glyph. Both pin to the bottom of
    // the row so they visually merge into a single thin horizontal
    // stripe rather than a chunky full-row bar. Only painted when
    // the ribbon contains more slots than fit on screen.
    if scrolling_browse && ribbon_total > ribbon_visible && full_area.height >= 1 {
        let bar_y = full_area.y + full_area.height - 1;
        let total_cells = usable_width as usize;
        let filled_w = ((ribbon_visible * total_cells + ribbon_total / 2) / ribbon_total).max(1);
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
        let rail_area = Rect {
            x: full_area.x + sidebar_width + strip_width,
            y: bar_y,
            width: usable_width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(rail_text).style(rail_style), rail_area);

        // U+2584 LOWER HALF BLOCK: thumb stands proud above the rail
        // but still in the lower half of the row.
        let thumb_text = "\u{2584}".repeat(filled_w);
        let thumb_area = Rect {
            x: full_area.x + sidebar_width + strip_width + filled_x as u16,
            y: bar_y,
            width: filled_w as u16,
            height: 1,
        };
        frame.render_widget(Paragraph::new(thumb_text).style(thumb_style), thumb_area);

        // Register the rail for click + drag hit-testing. The mouse
        // handler maps clicks anywhere on the rail to a ribbon-slot
        // scroll position; dragging the thumb pans continuously.
        state.hit_regions.borrow_mut().miller_h_scrollbar =
            Some(crate::app::presentation::MillerHScrollbar {
                rail: rail_area,
                thumb_x: full_area.x + sidebar_width + strip_width + filled_x as u16,
                thumb_w: filled_w as u16,
                total: ribbon_total,
                visible: ribbon_visible,
            });
    }

    // Chrome: tab bar, transport, command bar
    render_transport(frame, state, layout.transport);
}

/// Artwork and metadata above the transport, visualizer below it.
/// The independent queue and application footer are rendered by the workspace owner.
fn render_queue_and_visualizer_in(frame: &mut Frame, state: &AppState, area_param: Rect) {
    use ratatui::layout::{Constraint, Layout};

    let header = crate::ui::layout::player_header(
        f32::from(area_param.height) * 20.,
        f32::from(area_param.width) * state.cell_width(),
    );
    let retained = state.window_geometry().player_header;
    let header = if retained > 0 {
        f32::from(retained).min(f32::from(area_param.height.saturating_sub(5)) * 20.)
    } else {
        header
    };
    let hero = ((header - crate::ui::layout::TRANSPORT_HEIGHT).max(0.) / 20.).round() as u16;
    let parts = Layout::vertical([
        Constraint::Length(hero),
        Constraint::Length(5),
        Constraint::Min(5),
    ])
    .split(area_param);
    screens::now_playing::render_current_track(frame, state, parts[0]);
    render_transport(frame, state, parts[1]);
    screens::now_playing::render_visualizer_panel(frame, state, parts[2]);
    if parts[0].width >= 4 && parts[0].height >= 2 {
        let control = crate::app::transport_controls::TransportControl::Player;
        let rect = Rect::new(parts[0].right() - 3, parts[0].y + 1, 2, 1);
        let colors = theme().colors;
        frame.render_widget(
            Paragraph::new("⌄ ").style(if state.tui_transport_focus == Some(control) {
                colors.focused_row()
            } else {
                Style::new().fg(colors.fg_secondary)
            }),
            rect,
        );
        let mut regions = state.hit_regions.borrow_mut();
        regions.player_toggle = Some(rect);
        if let Some(transport) = &mut regions.transport {
            transport.controls.push((rect, control));
        }
    }
}

fn render_search(frame: &mut Frame, state: &AppState, area: Rect) {
    let layout = FullScreenLayout::new(area);

    // Unified search/filter screen handles all tabs including Global (with 3-column layout)
    screens::filter::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
}

fn render_help(frame: &mut Frame, state: &AppState, area: Rect) {
    let layout = FullScreenLayout::new(area);

    screens::help::render(frame, state, layout.content);
    render_transport(frame, state, layout.transport);
}

fn render_settings(frame: &mut Frame, state: &AppState, area: Rect) {
    let width = area.width.saturating_sub(4).clamp(1, 108).min(area.width);
    let height = area.height.saturating_sub(4).clamp(1, 34).min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    screens::settings::render(frame, state, popup);
    super::widgets::register_popup_close(state, popup);
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
    area: Rect,
    fixed_col_width: Option<u16>,
) {
    let focus_override = state.category_column_focused.then_some(0);
    let filter_results = state.list_filter.results.as_ref().filter(|_| {
        state.list_filter.active && state.list_filter.category == state.browse_category
    });
    let filter_column = filter_results.map(|_| state.list_filter.column);
    use crate::services::FolderItemType;

    let t = theme();

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
            let loading =
                Paragraph::new("Loading...").style(Style::default().fg(t.colors.fg_muted));
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
            .find(|&i| {
                !folder_state.columns[i].items.is_empty() || i <= folder_state.focused_column
            })
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
        let start_col = if state.miller_scroll_manual {
            state
                .miller_scroll_col
                .min(effective_columns.saturating_sub(max_visible))
        } else {
            (viewport_focus + 1).saturating_sub(max_visible)
        };

        // Get currently playing track key once for all columns
        let current_track_key = state.current_track().map(|t| t.rating_key.as_str());

        // Register folder Miller column regions for hit-testing
        {
            let mut column_regions = Vec::new();
            for (vis_idx, col_idx) in
                (start_col..effective_columns.min(start_col + max_visible)).enumerate()
            {
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
                column_regions.push(crate::app::presentation::MillerColumnRegion {
                    col_idx,
                    area: col_area,
                    inner: inner_tmp,
                    details_max_scroll: None,
                    rows_per_item: 1, // Folder items are always 1-row
                    is_art_mode: false,
                    view_options: None,
                    controls: Vec::new(),
                    close_x,
                });
            }
            let mut hr = state.hit_regions.borrow_mut();
            hr.miller_columns = Some(crate::app::presentation::MillerRegions {
                area,
                columns: column_regions,
            });
        }

        for (vis_idx, col_idx) in
            (start_col..effective_columns.min(start_col + max_visible)).enumerate()
        {
            let col = &folder_state.columns[col_idx];
            let is_focused = state.view == View::Browse
                && focus_override.is_none()
                && col_idx == folder_state.focused_column;

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

            let border_color = if is_focused {
                t.colors.border_focused
            } else {
                t.colors.fg_muted
            };
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
                let title_color = if is_focused {
                    t.colors.border_focused
                } else {
                    t.colors.fg_accent
                };
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
                    Style::default()
                        .fg(t.colors.fg_accent)
                        .bg(t.colors.bg_primary)
                } else {
                    Style::default()
                        .fg(t.colors.fg_muted)
                        .bg(t.colors.bg_primary)
                };
                frame.render_widget(Paragraph::new("\u{2715}").style(style), close_x);
            }

            if col.items.is_empty() {
                let empty = Paragraph::new("(empty)").style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // LAZY LOADING: Only render visible items
                let visible_height = inner.height as usize;
                let selected_idx = col.selected_index;

                // Calculate max width for text (minus prefix and padding)
                let max_text_width = inner.width.saturating_sub(4) as usize;

                // Check if filter is active on this column
                let is_filter_column = filter_column == Some(col_idx);
                let column_filter = filter_results.filter(|_| is_filter_column);
                let display_indices =
                    column_filter.map(|results| results.matched_indices.as_slice());
                let filter_active_on_col = column_filter.is_some();
                let total_items = display_indices.map_or(col.items.len(), <[usize]>::len);

                // Calculate scroll offset (needed for both rendering and scrollbar)
                let display_selected_idx =
                    if let Some(results) = column_filter.filter(|_| filter_active_on_col) {
                        results
                            .matched_indices
                            .iter()
                            .position(|&idx| idx == selected_idx)
                            .unwrap_or(0)
                    } else {
                        selected_idx
                    };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(
                        display_selected_idx,
                        visible_height,
                        total_items,
                    ),
                };

                let scroll_offset = state
                    .presentation_anchors
                    .as_ref()
                    .and_then(|anchors| {
                        anchors.display_offset(
                            state,
                            col_idx,
                            col.items.iter().map(|item| item.key.as_str()),
                            display_indices,
                        )
                    })
                    .unwrap_or(scroll_offset);
                state
                    .hit_regions
                    .borrow_mut()
                    .miller_offsets
                    .push((col_idx, scroll_offset));
                if total_items == 0 && filter_active_on_col {
                    let empty =
                        Paragraph::new("no matches").style(Style::default().fg(t.colors.fg_muted));
                    frame.render_widget(empty, inner);
                } else {
                    // Only create ListItems for visible range
                    let display_items: Box<
                        dyn Iterator<Item = (usize, &crate::services::FolderItem)> + '_,
                    > = if let Some(indices) = display_indices {
                        Box::new(
                            indices
                                .iter()
                                .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item))),
                        )
                    } else {
                        Box::new(col.items.iter().enumerate())
                    };
                    let visible_items: Vec<ListItem> = display_items
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
                                Style::default()
                                    .fg(t.colors.fg_accent)
                                    .add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_selected && is_focused {
                                t.colors.focused_row()
                            } else if is_selected {
                                // Selected row in a non-focused folder
                                // col: dim highlight, not the cursor.
                                t.colors.inactive_row()
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
                    render_scrollbar(
                        frame,
                        col_area,
                        total_items,
                        visible_height,
                        scroll_offset,
                        Some(border_color),
                    );

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
        let msg =
            Paragraph::new("Loading folders...").style(Style::default().fg(t.colors.fg_muted));
        frame.render_widget(msg, inner);
    }
}

/// Single source of truth for the art-grid row height (and matching
/// art width). Used by `render_album_art_grid` for layout *and* by
/// the Miller-column hit-region registration so `mouse_input` can
/// read the same value that was rendered. Returns `(row_height,
/// artwork width), both in terminal cells. Large covers place captions below; small covers use a side caption.
pub(crate) fn compute_art_grid_row(
    inner_width: u16,
    inner_height: u16,
    cell_width: f32,
    large: bool,
) -> (u16, u16) {
    let cover_height = ((if large {
        super::layout::LARGE_ARTWORK
    } else {
        super::layout::SMALL_ARTWORK
    } / 20.)
        .round() as u16)
        .max(1)
        .min(inner_height.saturating_sub(2))
        .min((f32::from(inner_width.saturating_sub(2)) * cell_width / 20.) as u16);

    (
        (cover_height + if large { 2 } else { 1 })
            .min(inner_height)
            .max(1),
        ((f32::from(cover_height) * 20. / cell_width).round() as u16).min(inner_width),
    )
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

    let column_offset = state.browse_column_offset();
    let Some(nav) = state.browse_nav() else {
        return 0;
    };
    let num_columns = nav.columns.len();
    if num_columns <= column_offset {
        return if nav.loading { 1 } else { 0 };
    }
    let last_meaningful = (column_offset..num_columns)
        .rev()
        .find(|&i| {
            !nav.columns[i].items.is_empty()
                || nav.columns[i].details.is_some()
                || i <= nav.focused_column
        })
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

    let inner = area;
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
    );

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
        .is_some_and(|c| !c.sort_ascending);

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
        let is_selected = state.view == View::Browse
            && state.alphabet_strip_focused
            && state.alphabet_strip_index == letter_idx;
        if is_selected {
            t.colors.focused_row()
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
        for (visual_idx, &row) in rows.iter().enumerate().take(n) {
            let letter_idx = if descending {
                n - 1 - visual_idx
            } else {
                visual_idx
            };
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
                let cur_dist = row.abs_diff(cur);
                let nxt_dist = row.abs_diff(nxt);
                if nxt_dist < cur_dist {
                    next_idx += 1;
                } else {
                    break;
                }
            }
            let visual_idx = next_idx;
            let letter_idx = if descending {
                n - 1 - visual_idx
            } else {
                visual_idx
            };
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
            let letter_idx = if descending {
                n - 1 - visual_idx
            } else {
                visual_idx
            };
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
    hr.alphabet_strip = Some(crate::app::presentation::AlphabetStripRegions {
        area,
        letters: letters_hit,
    });
}

/// Track details with pinned playback controls, artwork, metadata and similar tracks.
/// Keyboard focus highlights the selected control or result row.
pub(crate) fn render_track_details_pane(
    frame: &mut Frame,
    state: &AppState,
    area: Rect,
    track: &crate::library::models::Track,
    details: Option<&crate::app::result_details::State>,
) {
    use crate::util::truncate_middle;
    let t = theme();

    let pane_focused = details.map_or(
        state.view == View::Browse
            && state.track_pane_focused
            && !crate::app::sonic_details::focused(state),
        |d| d.focused,
    );
    let pane_idx = details.map_or(state.track_pane_index, |d| d.action_index);
    let child_details =
        details.is_some() && state.view == View::Browse && !state.popups.search_active();
    let show_similar =
        (details.is_none() || child_details) && crate::app::sources::sonic::enabled(state);

    let block = Block::default()
        .title(format!(
            " {} ",
            super::widgets::column_header::compact_title(
                "Track Details",
                area.width.saturating_sub(6) as usize
            )
        ))
        .title_style(Style::default().fg(if pane_focused {
            t.colors.border_focused
        } else {
            t.colors.fg_accent
        }))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if pane_focused {
            t.colors.border_focused
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
            Style::default()
                .fg(t.colors.fg_accent)
                .bg(t.colors.bg_primary)
        } else {
            Style::default()
                .fg(t.colors.fg_muted)
                .bg(t.colors.bg_primary)
        };
        frame.render_widget(Paragraph::new("\u{2715}").style(style), r);
        Some(r)
    } else {
        None
    };

    let action_width = 5.min(inner.width / 3).max(1);
    let action_x = inner.x + inner.width.saturating_sub(action_width * 3) / 2;
    let play_area = Rect::new(action_x, inner.y, action_width, 1).intersection(inner);
    let next_area =
        Rect::new(action_x + action_width, inner.y, action_width, 1).intersection(inner);
    let add_area =
        Rect::new(action_x + action_width * 2, inner.y, action_width, 1).intersection(inner);
    for (rect, label, index) in [
        (play_area, "▷ ", 0),
        (next_area, "▹ ", 1),
        (add_area, "+ ", 2),
    ] {
        frame.render_widget(
            Paragraph::new(label.trim())
                .alignment(Alignment::Center)
                .style(if pane_focused && pane_idx == index {
                    t.colors.focused_row()
                } else {
                    Style::default()
                        .fg(t.colors.fg_accent)
                        .bg(t.colors.bg_primary)
                }),
            rect,
        );
    }
    let body_y = inner.y + 2.min(inner.height);
    let body_h = inner.height.saturating_sub(2);
    if body_h == 0 {
        // No room for a body; register hit regions and return.
        let mut hr = state.hit_regions.borrow_mut();
        hr.track_pane = Some(crate::app::presentation::TrackPaneRegions {
            scroll: 0,
            max_scroll: 0,
            outer: area,
            play_button: play_area,
            add_button: add_area,
            next_button: next_area,
            similar_rows: Vec::new(),
            close_x: close_x_rect,
        });
        return;
    }
    let scrollbar_w: u16 = if body_h >= 4 { 1 } else { 0 };
    let body_w = inner.width.saturating_sub(scrollbar_w);

    // Leave metadata and related tracks visible beneath the cover.
    let art_max_h_by_body = (body_h / 2).max(2);
    let art_max_h_cap: u16 = 12;
    let art_max_h = art_max_h_by_body.min(art_max_h_cap);
    let art_max_w = body_w;
    let art_w = art_max_w.min(art_max_h.saturating_mul(2)).max(4);
    let art_h = if track
        .parent_rating_key
        .as_ref()
        .is_none_or(|key| !state.artwork.grid_cache.contains_key(key))
    {
        1 // Keep metadata near the actions when there is no cached cover to show.
    } else {
        (art_w / 2).max(2).min(art_max_h)
    };

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
    //   ...         : optional "Sonically Similar" header (library setting)
    //   ...         : optional similar rows

    let max_w = body_w as usize;
    let mut lines: Vec<(Style, String)> = Vec::new();

    // Title (bold/accent).
    lines.push((
        Style::default()
            .fg(t.colors.fg_primary)
            .add_modifier(ratatui::style::Modifier::BOLD),
        truncate_middle(&track.title, max_w),
    ));

    for (label, value) in crate::app::details::track_rows(track) {
        for line in wrap_text(&format!("{label}: {value}"), max_w) {
            lines.push((Style::default().fg(t.colors.fg_primary), line));
        }
    }
    // ── Compute virtual content layout ──────────────────────────────
    // Content rows are addressed in body-relative coordinates (0 = the
    // first row of the body region, just below the pinned Play row).
    // Layout: blank, artwork, blank, metadata lines, blank, "Sonically
    // Similar" header, similar rows (or placeholder).
    let art_top: u16 = 1; // 1-row gap after Play
    let art_bot: u16 = art_top + art_h; // exclusive
    let info_top: u16 = art_bot + 1; // 1-row gap after art
    let info_bot: u16 = info_top + lines.len() as u16;
    let header_y: u16 = info_bot + 1; // 1-row gap after info
    let list_y: u16 = header_y + 1;
    let similar_result = state.track_pane_similar.get(&track.rating_key);
    let similar_data = similar_result.and_then(|result| result.as_ref().ok());
    let similar_count: u16 = similar_data
        .map(|v| if v.is_empty() { 1 } else { v.len() as u16 })
        .unwrap_or(1); // "Loading…" placeholder occupies 1 row
    let content_h: u16 = if show_similar {
        list_y + similar_count
    } else {
        info_bot
    };

    // Auto-scroll: when the pane has focus and the user has highlighted
    // a similar row that lives outside the visible window, slide the
    // window so the selected row remains visible. Playback controls stay pinned.
    let selected_content_y: Option<u16> = if pane_focused && pane_idx > 2 {
        Some(list_y + (pane_idx as u16 - 3))
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

    let scroll = if child_details {
        crate::app::sonic_details::visible(state)
            .and_then(|c| c.scroll_pin)
            .unwrap_or_else(|| scroll.max(details.map_or(0, |d| d.scroll)))
            .min(max_scroll)
    } else {
        details.map_or(scroll, |d| d.scroll.min(max_scroll))
    };

    // Helper: convert content_y → screen_y, returning None if the row
    // is clipped (above or below the body window).
    let to_screen = |cy: u16| -> Option<u16> {
        if cy < scroll {
            return None;
        }
        let off = cy - scroll;
        if off >= body_h {
            return None;
        }
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
            if let Some(album_key) = track.parent_rating_key.as_deref()
                && let Some(data) = state.artwork.grid_cache.get(album_key)
            {
                rendered_art = super::artwork::render_grid_image(frame, art_area, album_key, data);
            }
            if !rendered_art {
                super::artwork::render_placeholder(
                    frame,
                    art_area,
                    super::artwork::Placeholder::Album,
                );
            }
        } else {
            // Partially clipped — paint a blank-ish slice so any
            // previous frame's glyphs don't bleed through.
            for r in visible_art_top..visible_art_bot {
                let sy = body_y + (r - scroll);
                frame.render_widget(
                    Paragraph::new("").style(Style::default().bg(t.colors.bg_primary)),
                    Rect {
                        x: inner.x,
                        y: sy,
                        width: body_w,
                        height: 1,
                    },
                );
            }
        }
    }

    // Metadata lines.
    for (i, (style, text)) in lines.iter().enumerate() {
        let cy = info_top + i as u16;
        if let Some(sy) = to_screen(cy) {
            if text.starts_with("Artist: ") {
                crate::ui::artist_links::text(
                    state,
                    Rect::new(inner.x, sy, body_w, 1),
                    text,
                    track.track_artist(),
                    track.grandparent_rating_key.as_deref(),
                );
            }
            frame.render_widget(
                Paragraph::new(text.as_str()).style(*style),
                Rect {
                    x: inner.x,
                    y: sy,
                    width: body_w,
                    height: 1,
                },
            );
        }
    }

    // Sonic-only section disappears when disabled; metadata/play controls remain.
    let mut similar_rows: Vec<(Rect, usize)> = Vec::new();
    if show_similar {
        // "Sonically Similar" header.
        if let Some(sy) = to_screen(header_y) {
            frame.render_widget(
                Paragraph::new(crate::app::details::Kind::Track.related_label()).style(
                    Style::default()
                        .fg(t.colors.fg_accent)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Rect {
                    x: inner.x,
                    y: sy,
                    width: body_w,
                    height: 1,
                },
            );
        }

        // Similar rows (or placeholder).
        match similar_data {
            Some(list) if list.is_empty() => {
                if let Some(sy) = to_screen(list_y) {
                    frame.render_widget(
                        Paragraph::new("(no similar tracks found)")
                            .style(Style::default().fg(t.colors.fg_muted)),
                        Rect {
                            x: inner.x,
                            y: sy,
                            width: body_w,
                            height: 1,
                        },
                    );
                }
            }
            Some(list) => {
                for (i, sim) in list.iter().enumerate() {
                    let cy = list_y + i as u16;
                    let sy = match to_screen(cy) {
                        Some(v) => v,
                        None => continue,
                    };
                    let label = format!("\u{2022} {} \u{2014} {}", sim.title, sim.track_artist(),);
                    let truncated = truncate_middle(&label, max_w);
                    let row_focused = pane_focused && pane_idx == i + 3;
                    let style = if row_focused {
                        t.colors.focused_row()
                    } else {
                        Style::default().fg(t.colors.fg_primary)
                    };
                    let row_area = Rect {
                        x: inner.x,
                        y: sy,
                        width: body_w,
                        height: 1,
                    };
                    crate::ui::artist_links::text(
                        state,
                        row_area,
                        &truncated,
                        sim.track_artist(),
                        sim.grandparent_rating_key.as_deref(),
                    );
                    frame.render_widget(Paragraph::new(truncated).style(style), row_area);
                    similar_rows.push((row_area, i));
                }
            }
            None => {
                if let Some(sy) = to_screen(list_y) {
                    frame.render_widget(
                        Paragraph::new(
                            crate::app::details::related_message(
                                similar_result,
                                state.track_pane_similar_loading.contains(&track.rating_key),
                            )
                            .unwrap_or_default(),
                        )
                        .style(Style::default().fg(t.colors.fg_muted)),
                        Rect {
                            x: inner.x,
                            y: sy,
                            width: body_w,
                            height: 1,
                        },
                    );
                }
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
            let in_thumb = (r as usize) >= thumb_pos && (r as usize) < thumb_pos + thumb_size;
            let glyph = if in_thumb { "\u{2588}" } else { "\u{2502}" };
            let style = if in_thumb {
                Style::default().fg(t.colors.fg_accent)
            } else {
                Style::default().fg(t.colors.border)
            };
            frame.render_widget(
                Paragraph::new(glyph).style(style),
                Rect {
                    x: bar_x,
                    y: sy,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    // Register click regions so the mouse handler can dispatch
    // Play / drill-into-similar / close-pane.
    let mut hr = state.hit_regions.borrow_mut();
    hr.track_pane = Some(crate::app::presentation::TrackPaneRegions {
        scroll,
        max_scroll,
        outer: area,
        play_button: play_area,
        add_button: add_area,
        next_button: next_area,
        similar_rows,
        close_x: close_x_rect,
    });
}

fn render_browse_miller_columns(
    frame: &mut Frame,
    state: &AppState,
    nav: &crate::app::state::BrowseNavigationState,
    root_title: &str,
    area: Rect,
    fixed_col_width: Option<u16>,
    // Number of leading columns to hide. Used by the Playlists
    // category to suppress the redundant root "playlists" column —
    // the playlists are already enumerated in the leftmost browse
    // column, so showing them again next door is duplicate chrome.
    column_offset: usize,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;
    let current_track_key = state.current_track().map(|track| track.rating_key.as_str());
    let focus_override = state.category_column_focused.then_some(0);
    let filter_results = state.list_filter.results.as_ref().filter(|_| {
        state.list_filter.active && state.list_filter.category == state.browse_category
    });
    let filter_column = filter_results.map(|_| state.list_filter.column);
    let t = theme();

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
        let loading = Paragraph::new("Loading...").style(Style::default().fg(t.colors.fg_muted));
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
        .find(|&i| {
            !nav.columns[i].items.is_empty()
                || nav.columns[i].details.is_some()
                || i <= nav.focused_column
        })
        .unwrap_or(column_offset);
    let effective_columns = last_meaningful + 1;
    // When loading with existing columns, reserve space for a loading indicator column
    let layout_columns = if nav.loading {
        effective_columns + 1
    } else {
        effective_columns
    };

    // Pick the column width.
    //
    let (max_visible, col_width) = if let Some(fixed_w) = fixed_col_width {
        let max_vis = (area.width / fixed_w).max(1) as usize;
        (max_vis, fixed_w)
    } else {
        let max_vis = 3.min(layout_columns).max(2);
        (max_vis, area.width / max_vis as u16)
    };

    let start_col = column_offset;

    // Register Miller column regions for hit-testing
    {
        let mut column_regions = Vec::new();
        for (vis_idx, col_idx) in
            (start_col..effective_columns.min(start_col + max_visible)).enumerate()
        {
            let col = &nav.columns[col_idx];
            let col_area = Rect {
                x: area.x + (vis_idx as u16 * col_width),
                y: area.y,
                width: col_width.min(area.width.saturating_sub(vis_idx as u16 * col_width)),
                height: area.height,
            };
            let block_tmp = Block::default().borders(Borders::ALL);
            let mut inner_tmp = block_tmp.inner(col_area);
            if col_idx == 0 && state.alphabet_strip_visible() {
                inner_tmp.width = inner_tmp.width.saturating_sub(2);
            }
            if widgets::column_header::has_content_row(col) {
                inner_tmp.y += u16::from(inner_tmp.height > 0);
                inner_tmp.height = inner_tmp.height.saturating_sub(1);
            }
            let is_two_row =
                crate::app::presentation::is_two_row_browse_column(state, col, col_idx, nav);
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
            column_regions.push(crate::app::presentation::MillerColumnRegion {
                col_idx,
                area: col_area,
                inner: inner_tmp,
                details_max_scroll: album_detail_lines(state, col).map(|lines| {
                    let (_, text) = detail_layout(col, inner_tmp);
                    Paragraph::new(lines.join("\n"))
                        .wrap(Wrap { trim: false })
                        .line_count(text.width)
                        .saturating_sub(text.height as usize)
                }),
                rows_per_item: if is_two_row { 2 } else { 1 },
                is_art_mode: col.artwork_visible,
                view_options: (state.column_options_type(col_idx).is_some()
                    && col_area.width >= 10)
                    .then(|| {
                        Rect::new(
                            col_area.right() - if close_x.is_some() { 5 } else { 3 },
                            col_area.y,
                            1,
                            1,
                        )
                    }),
                close_x,
                controls: Vec::new(),
            });
        }
        let mut hr = state.hit_regions.borrow_mut();
        hr.miller_columns = Some(crate::app::presentation::MillerRegions {
            area,
            columns: column_regions,
        });
    }

    for (vis_idx, col_idx) in
        (start_col..effective_columns.min(start_col + max_visible)).enumerate()
    {
        let col = &nav.columns[col_idx];
        // Track-details pane is treated as the rightmost column for
        // focus purposes: when it's focused, no miller column should
        // also paint as focused, otherwise the user sees two
        // simultaneously highlighted "selected" rows (the pane on the
        // right and the track row in the tracks column behind it).
        let is_focused = state.view == View::Browse
            && state.tui_transport_focus.is_none()
            && state.tui_footer_focus.is_none()
            && state.tui_artist_focus.is_none()
            && !state.tui_description_focused
            && !state.tui_description_submit_focused
            && focus_override.is_none()
            && col_idx == nav.focused_column
            && !state.track_pane_focused;
        let is_root = col_idx == 0;

        let col_area = Rect {
            x: area.x + (vis_idx as u16 * col_width),
            y: area.y,
            width: col_width.min(area.width.saturating_sub(vis_idx as u16 * col_width)),
            height: area.height,
        };

        let border_color = if is_focused {
            t.colors.border_focused
        } else {
            t.colors.fg_muted
        };

        // Show title for all columns with sort suffix
        let sort_suffix = {
            let suffix = col.sort_mode.header_suffix(!col.sort_ascending);
            if suffix.is_empty() {
                String::new()
            } else {
                format!(" ({})", suffix)
            }
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
        // Description results expose the same save command as the palette at
        // the top of the column, without consuming a track row or changing hit tests.
        let save_description =
            col_idx == 1 && crate::app::sources::audiomuse::description_results(state).is_some();
        let title = if save_description {
            " [Save as playlist] ".to_owned()
        } else if col
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::Description { .. }))
        {
            format!("{} · Del: Remove ", title.trim_end())
        } else {
            title
        };
        let view_options = state
            .hit_regions
            .borrow()
            .miller_columns
            .as_ref()
            .and_then(|m| m.columns.iter().find(|r| r.col_idx == col_idx))
            .and_then(|r| r.view_options);
        // Block titles overflow past the column edge when they're
        // longer than the column is wide — Mac-style middle-truncate
        // so both ends stay readable. Reserve 4 cells for the corner
        // border characters and a leading/trailing space.
        let title = if !title.is_empty() {
            let max_title_w = view_options
                .map(|r| r.x.saturating_sub(col_area.x + 1) as usize)
                .unwrap_or_else(|| (col_area.width as usize).saturating_sub(4));
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
            let title_color = if is_focused {
                t.colors.fg_accent
            } else {
                t.colors.fg_secondary
            };
            block = block
                .title(title)
                .title_style(Style::default().fg(title_color));
        }

        let mut full_inner = block.inner(col_area);
        if col_idx == 0 && state.alphabet_strip_visible() {
            full_inner.width = full_inner.width.saturating_sub(2);
        }
        frame.render_widget(block, col_area);
        if let Some(parent) = col_idx
            .checked_sub(1)
            .and_then(|index| nav.columns.get(index))
            .and_then(|column| column.selected_item())
        {
            let artist = match parent {
                BrowseItem::Artist { key, title, .. } => Some((key.as_str(), title.as_str())),
                BrowseItem::ArtistInfo {
                    artist_key,
                    artist_name,
                    ..
                }
                | BrowseItem::CompilationTracks {
                    artist_key,
                    artist_name,
                    ..
                } => Some((artist_key.as_str(), artist_name.as_str())),
                _ => None,
            };
            if let Some((key, name)) = artist {
                crate::ui::artist_links::visible(
                    frame,
                    state,
                    Rect::new(col_area.x, col_area.y, col_area.width, 1),
                    name,
                    Some(key),
                );
            }
        }
        if save_description && col_area.width >= 6 {
            state.hit_regions.borrow_mut().description_save = Some(Rect::new(
                col_area.x + 1,
                col_area.y,
                20.min(col_area.width.saturating_sub(5)),
                1,
            ));
        }

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
                Style::default()
                    .fg(t.colors.fg_accent)
                    .bg(t.colors.bg_primary)
            } else {
                Style::default()
                    .fg(t.colors.fg_muted)
                    .bg(t.colors.bg_primary)
            };
            frame.render_widget(Paragraph::new("\u{2715}").style(style), close_x);
        }

        if let Some(rect) = view_options {
            frame.render_widget(
                Paragraph::new("≡").style(
                    Style::default()
                        .fg(if is_focused {
                            t.colors.fg_accent
                        } else {
                            t.colors.fg_muted
                        })
                        .bg(t.colors.bg_primary),
                ),
                rect,
            );
        }

        let mut inner = full_inner;
        if widgets::column_header::has_content_row(col) {
            inner.y += u16::from(inner.height > 0);
            inner.height = inner.height.saturating_sub(1);
        }

        if let Some(lines) = album_detail_lines(state, col) {
            let (art, inner) = detail_layout(col, inner);
            if art.width > 0 && art.height > 0 {
                let rendered = col.detail_art_key().is_some_and(|key| {
                    state.artwork.grid_cache.get(key).is_some_and(|data| {
                        super::artwork::render_grid_image(frame, art, key, data)
                    })
                });
                if !rendered {
                    super::artwork::render_placeholder(
                        frame,
                        art,
                        if matches!(
                            col.details,
                            Some(crate::app::state::BrowseDetails::Artist(_))
                        ) {
                            super::artwork::Placeholder::Artist
                        } else {
                            super::artwork::Placeholder::Album
                        },
                    );
                }
            }
            let paragraph = Paragraph::new(lines.join("\n"))
                .style(Style::default().fg(t.colors.fg_primary))
                .wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(inner.width)
                .saturating_sub(inner.height as usize);
            frame.render_widget(
                paragraph.scroll((
                    col.selected_index.min(max_scroll).min(u16::MAX as usize) as u16,
                    0,
                )),
                inner,
            );
            match &col.details {
                Some(crate::app::state::BrowseDetails::Artist(info)) => {
                    crate::ui::artist_links::visible(
                        frame,
                        state,
                        Rect::new(col_area.x, col_area.y, col_area.width, 1),
                        &info.artist_name,
                        Some(&info.artist_key),
                    );
                    for artist in &info.document.related_artists {
                        crate::ui::artist_links::visible(
                            frame,
                            state,
                            inner,
                            &artist.title,
                            Some(&artist.rating_key),
                        );
                    }
                }
                Some(crate::app::state::BrowseDetails::Album { lines, .. }) => {
                    for line in lines
                        .iter()
                        .filter_map(|line| line.strip_prefix("Artist: "))
                    {
                        let count = state.hit_regions.borrow().artist_links.len();
                        crate::ui::artist_links::visible(frame, state, inner, line, None);
                        if state.hit_regions.borrow().artist_links.len() == count {
                            for name in line.split(", ") {
                                crate::ui::artist_links::visible(frame, state, inner, name, None);
                            }
                        }
                    }
                }
                None => {}
            }
            continue;
        }

        if col.items.is_empty() {
            let text = if nav.loading { "Loading…" } else { "(empty)" };
            let empty = Paragraph::new(text).style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            continue;
        }

        let is_filter_column = filter_column == Some(col_idx);
        let per_col_result = filter_results.filter(|_| is_filter_column);

        if col.artwork_visible {
            let col_filter = per_col_result;
            render_album_art_grid(
                frame,
                state,
                col,
                is_focused
                    && col.header_selection == crate::app::state::ColumnHeaderSelection::Items,
                (col_area, inner),
                col_idx,
                col_filter,
            );
            continue;
        }

        {
            // Calculate visible range for lazy loading
            let visible_height = inner.height as usize;
            let selected_idx = col.selected_index;

            // Calculate max width for text (minus prefix and padding)
            let max_text_width = inner.width.saturating_sub(4) as usize;

            let is_two_row =
                crate::app::presentation::is_two_row_browse_column(state, col, col_idx, nav);
            let rows_per_item = if is_two_row { 2 } else { 1 };
            let visible_item_count = visible_height / rows_per_item;

            // Keep the unfiltered path allocation-free: retaining a slice of
            // matched indices avoids constructing a 70k-element Vec<&Item>
            // merely to render the twenty rows visible in the terminal.
            let active_filter = per_col_result;
            let display_indices = active_filter.map(|results| results.matched_indices.as_slice());
            let filter_active_on_col = active_filter.is_some();
            let total_display_items = display_indices.map_or(col.items.len(), <[usize]>::len);

            if total_display_items == 0 && filter_active_on_col {
                let empty =
                    Paragraph::new("no matches").style(Style::default().fg(t.colors.fg_muted));
                frame.render_widget(empty, inner);
            } else {
                // Calculate scroll offset based on display items.
                // When per-column live filter is active, map the
                // real selected_index into the filtered display
                // index; otherwise fall back to the historic single-
                // column results / unfiltered position.
                let display_selected_idx = if let Some(results) = active_filter {
                    results
                        .matched_indices
                        .iter()
                        .position(|&idx| idx == selected_idx)
                        .unwrap_or(0)
                } else {
                    selected_idx
                };
                let scroll_offset = match state.scroll.browse {
                    Some((pin_col, pinned)) if pin_col == col_idx => pinned,
                    _ => NavigationService::calc_scroll_offset(
                        display_selected_idx,
                        visible_item_count,
                        total_display_items,
                    ),
                };

                let scroll_offset = state
                    .presentation_anchors
                    .as_ref()
                    .and_then(|anchors| {
                        anchors.display_offset(
                            state,
                            col_idx,
                            col.items.iter().map(BrowseItem::key),
                            display_indices,
                        )
                    })
                    .unwrap_or(scroll_offset);
                state
                    .hit_regions
                    .borrow_mut()
                    .miller_offsets
                    .push((col_idx, scroll_offset));
                let display_items: Box<dyn Iterator<Item = (usize, &BrowseItem)> + '_> =
                    if let Some(indices) = display_indices {
                        Box::new(
                            indices
                                .iter()
                                .filter_map(|&idx| col.items.get(idx).map(|item| (idx, item))),
                        )
                    } else {
                        Box::new(col.items.iter().enumerate())
                    };
                let visible_items: Vec<ListItem> = display_items
                    .skip(scroll_offset)
                    .take(visible_item_count)
                    .enumerate()
                    .map(|(visible_pos, (orig_idx, item))| {
                        // When the cursor parks on the synthetic
                        // Play row, no real item should render as
                        // "selected" — the play row owns the
                        // highlight until ↓ pulls the cursor down
                        // into the items.
                        let is_selected = orig_idx == selected_idx && col.header_selection == crate::app::state::ColumnHeaderSelection::Items;
                        if let Some(id) = item.pending_download() {
                            let status = crate::app::downloads::pending_status(state, id).unwrap_or_default();
                            let width = inner.width.saturating_sub(4) as usize;
                            let title = crate::util::truncate_middle(item.title(), width);
                            let text = if is_two_row { vec![Line::from(title), Line::from(status)] }
                                else { vec![Line::from(crate::util::truncate_middle(&format!("{title} · {status}"), width))] };
                            return ListItem::new(text).style(Style::default().fg(t.colors.fg_muted)
                                .add_modifier(if is_selected { Modifier::UNDERLINED } else { Modifier::empty() }));
                        }

                        // Multi-select: row is in `col.selected_set`.
                        // Drives a visual "●" prefix and an accent
                        // tint, mirroring the queue track-list style.
                        let is_multi_selected = col.selected_set.contains(&orig_idx);

                        // Check if this is the currently playing track
                        let is_now_playing = matches!(item, BrowseItem::Track { key, .. } if current_track_key == Some(key.as_str()));

                        // Prefix based on item type
                        let is_pinned = matches!(item,
                            BrowseItem::AllArtists | BrowseItem::Compilations |
                            BrowseItem::AllTracks { .. } | BrowseItem::ArtistInfo { .. } |
                            BrowseItem::CompilationTracks { .. }
                        );
                        let prefix = match item {
                            BrowseItem::Track { .. } | BrowseItem::Album { .. } if col.playlist_editing => if is_multi_selected { "☑ " } else { "☐ " },
                            BrowseItem::Track { .. } if is_now_playing && is_multi_selected => "♪●",
                            BrowseItem::Track { .. } if is_now_playing => "♪ ",
                            BrowseItem::Track { .. } if is_multi_selected => "● ",
                            BrowseItem::Track { .. } => "  ",
                            BrowseItem::ArtistInfo { .. } => "ⓘ ",
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
                                        t.colors.focused_row(),
                                        t.colors.focused_row(),
                                        t.colors.focused_row(),
                                    )
                                } else if is_selected {
                                    // Selected row in a non-focused col:
                                    // dim "you were here" mark instead of
                                    // the strong cursor highlight, so the
                                    // user can tell at a glance which col
                                    // is actually accepting input.
                                    (
                                        t.colors.inactive_row(),
                                        t.colors.inactive_row(),
                                        t.colors.inactive_row(),
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

                                let artist=match item {BrowseItem::Track{artist_name,..}=>artist_name.as_deref(),BrowseItem::Album{artist,..}=>Some(artist.as_str()),_=>None};
                                if let Some(name)=artist {crate::ui::artist_links::text(state,Rect::new(inner.x+5,inner.y+(visible_pos*2+1) as u16,inner.width.saturating_sub(5),1),&subtitle_display,name,None);}
                                let text = Text::from(vec![
                                    line1,
                                    Line::from(Span::styled(format!("     {}", subtitle_display), line2_fg)),
                                ]);
                                ListItem::new(text).style(item_bg)
                            } else {
                                // Non-track/album item in a two-row column (handle gracefully)
                                let style = if is_selected && is_focused {
                                    t.colors.focused_row()
                                } else if is_selected {
                                    t.colors.inactive_row()
                                } else if is_multi_selected {
                                    Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary)
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
                                t.colors.focused_row()
                            } else if is_selected {
                                // Same row in an unfocused col: dim
                                // "you were here" mark.
                                t.colors.inactive_row()
                            } else if is_multi_selected {
                                Style::default().fg(t.colors.fg_accent).bg(t.colors.bg_secondary)
                            } else if is_now_playing {
                                Style::default().fg(t.colors.fg_accent).add_modifier(ratatui::style::Modifier::BOLD)
                            } else if is_pinned {
                                Style::default().fg(t.colors.fg_accent)
                            } else {
                                Style::default().fg(t.colors.fg_primary)
                            };

                            let style = if state.is_ghost_artist(item.key()) { style.add_modifier(ratatui::style::Modifier::ITALIC).patch(if is_selected { Style::default() } else { Style::default().fg(t.colors.fg_muted) }) } else { style };
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
                for pos in
                    0..visible_item_count.min(total_display_items.saturating_sub(scroll_offset))
                {
                    let index = display_indices
                        .map_or(scroll_offset + pos, |indices| indices[scroll_offset + pos]);
                    if let Some(id) = col.items[index].pending_download() {
                        let rect = Rect::new(
                            inner.right().saturating_sub(3),
                            inner.y + (pos * rows_per_item) as u16,
                            3,
                            1,
                        )
                        .intersection(inner);
                        frame.render_widget(
                            Paragraph::new(" × ").style(Style::default().fg(t.colors.fg_muted)),
                            rect,
                        );
                        state
                            .hit_regions
                            .borrow_mut()
                            .download_cancel
                            .push((rect, id.to_owned()));
                    }
                }

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
    let real_rendered = effective_columns
        .min(start_col + max_visible)
        .saturating_sub(start_col);
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
    let muted_bg = Style::default()
        .bg(t.colors.bg_primary)
        .fg(t.colors.fg_muted);
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
        if dy % 2 != 0 {
            continue;
        }
        let row_rect = Rect {
            x: inner.x,
            y: inner.y + dy,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(dot_row.clone()).style(Style::default().fg(t.colors.fg_muted)),
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
                Paragraph::new(label).style(Style::default().fg(t.colors.fg_accent)),
                Rect {
                    x: lx,
                    y: ly,
                    width: label_w,
                    height: 1,
                },
            );
        }
    }
}

/// Keep detail artwork above metadata, leaving text space on short terminals.
fn detail_layout(column: &crate::app::state::BrowseColumn, inner: Rect) -> (Rect, Rect) {
    if matches!(
        column.details,
        Some(crate::app::state::BrowseDetails::Artist(_))
    ) && !column.artwork_visible
    {
        return (Rect::default(), inner);
    }
    let art_height = (inner.width / 2)
        .min(inner.height.saturating_sub(5) / 2)
        .min(12);
    let art_width = (art_height * 2).min(inner.width);
    let art = Rect::new(
        inner.x + inner.width.saturating_sub(art_width) / 2,
        inner.y,
        art_width,
        art_height,
    );
    let reserved = if art_height > 0 { art_height + 1 } else { 0 };
    let text = Rect::new(
        inner.x,
        inner.y + reserved,
        inner.width,
        inner.height.saturating_sub(reserved),
    );
    (art, text)
}

fn render_album_art_grid(
    frame: &mut Frame,
    state: &AppState,
    col: &crate::app::state::BrowseColumn,
    is_focused: bool,
    (col_area, inner): (Rect, Rect),
    col_idx: usize,
    filter_results: Option<&crate::app::state::ListFilterResults>,
) {
    use crate::app::state::BrowseItem;
    use crate::util::truncate_middle;
    let t = theme();

    // Retain only the optional index slice. The unfiltered art path must not
    // allocate a Vec entry for every album on every terminal frame.
    let display_indices = if let Some(results) = filter_results {
        if results.matched_indices.is_empty() {
            let empty = Paragraph::new("no matches").style(Style::default().fg(t.colors.fg_muted));
            frame.render_widget(empty, inner);
            return;
        }
        Some(results.matched_indices.as_slice())
    } else {
        None
    };

    let total_items = display_indices.map_or(col.items.len(), <[usize]>::len);
    if total_items == 0 {
        return;
    }
    let item_at = |display_index: usize| -> Option<(usize, &BrowseItem)> {
        let original_index = display_indices
            .and_then(|indices| indices.get(display_index).copied())
            .unwrap_or(display_index);
        col.items
            .get(original_index)
            .map(|item| (original_index, item))
    };

    // Classify items: "one-row" pinned items vs normal art-height items
    fn is_one_row(item: &BrowseItem) -> bool {
        matches!(
            item,
            BrowseItem::ArtistInfo { .. }
                | BrowseItem::AllTracks { .. }
                | BrowseItem::CompilationTracks { .. }
                | BrowseItem::Compilations
        )
    }

    // Artwork cards share their height with pointer hit-testing.
    //
    // Single source of truth for the art-grid row height — both the
    // renderer and the mouse hit-test go through `compute_art_grid_row`.
    let (art_row_height, art_width) = compute_art_grid_row(
        inner.width,
        inner.height,
        state.cell_width(),
        state.large_artwork,
    );

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
        if idx + 1 >= total_items {
            return false;
        }
        match (item_at(idx), item_at(idx + 1)) {
            (Some((_, current)), Some((_, next))) => is_one_row(current) != is_one_row(next),
            _ => false,
        }
    };

    // Compute how many items are visible from a given scroll offset
    let count_visible_from = |offset: usize| -> usize {
        let mut y = 0u16;
        let mut count = 0;
        for i in offset..total_items {
            let Some((_, item)) = item_at(i) else {
                continue;
            };
            let h = if is_one_row(item) { 1 } else { art_row_height };
            // Account for spacer row after last one-row item
            let spacer = if has_spacer_after(i) { 1u16 } else { 0 };
            if y + h + spacer > inner.height {
                break;
            }
            y += h + spacer;
            count += 1;
        }
        count
    };

    let selected_idx = col.selected_index;

    // Convert selected_idx to display position within the (possibly filtered) list
    let display_selected = if let Some(indices) = display_indices {
        indices
            .iter()
            .position(|&idx| idx == selected_idx)
            .unwrap_or(0)
    } else {
        selected_idx
    };

    // Scroll offset: respect pin, otherwise ensure selected item is visible
    let scroll_offset = match state.scroll.browse {
        Some((pin_col, pinned)) if pin_col == col_idx => pinned,
        _ => {
            let item_height = |index| {
                item_at(index).map_or(0, |(_, item)| {
                    (if is_one_row(item) { 1 } else { art_row_height })
                        + u16::from(has_spacer_after(index))
                })
            };
            let mut offset = display_selected.min(total_items - 1);
            let mut used = item_height(offset);
            while offset > 0 {
                let previous = item_height(offset - 1);
                if used.saturating_add(previous) > inner.height {
                    break;
                }
                used += previous;
                offset -= 1;
            }
            offset
        }
    };

    let scroll_offset = state
        .presentation_anchors
        .as_ref()
        .and_then(|anchors| {
            anchors.display_offset(
                state,
                col_idx,
                col.items.iter().map(BrowseItem::key),
                display_indices,
            )
        })
        .unwrap_or(scroll_offset);
    state
        .hit_regions
        .borrow_mut()
        .miller_offsets
        .push((col_idx, scroll_offset));
    let visible_count = count_visible_from(scroll_offset);

    let mut row_y = inner.y;
    for vis_row in 0..visible_count {
        let display_idx = scroll_offset + vis_row;
        if display_idx >= total_items {
            break;
        }

        let Some((orig_idx, item)) = item_at(display_idx) else {
            continue;
        };
        let is_selected = orig_idx == selected_idx;
        let selection_style = if is_focused {
            t.colors.focused_row()
        } else {
            t.colors.inactive_row()
        };
        let one_row = is_one_row(item);
        let row_height = if one_row { 1 } else { art_row_height };

        if row_y + row_height > inner.y + inner.height {
            break;
        }

        state.hit_regions.borrow_mut().miller_item_rows.push((
            col_idx,
            Rect::new(inner.x, row_y, inner.width, row_height),
            orig_idx,
        ));
        let below = crate::app::view_options::ArtworkLayout::from(state.large_artwork)
            == crate::app::view_options::ArtworkLayout::Below;
        let caption_area = if one_row {
            Rect::new(inner.x, row_y, inner.width, 1)
        } else if below {
            Rect::new(
                inner.x,
                row_y + row_height.saturating_sub(2),
                inner.width,
                1,
            )
        } else {
            let x = inner.x + art_width + 1;
            Rect::new(
                x,
                row_y + row_height.saturating_sub(1) / 2,
                inner.right().saturating_sub(x),
                1,
            )
        };
        if is_selected {
            frame.render_widget(
                Block::default().style(selection_style),
                Rect::new(inner.x, row_y, inner.width, row_height),
            );
        }

        if one_row {
            // One-row item: full-width text, no art area
            let max_text = inner.width.saturating_sub(2) as usize;
            let display_title = item.title();
            let title_text = truncate_middle(display_title, max_text);
            let title_style = if is_selected {
                selection_style
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            frame.render_widget(
                Paragraph::new(format!(" {}", title_text)).style(title_style),
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
            );
        } else {
            let image_area = Rect::new(
                inner.x,
                row_y,
                art_width,
                row_height.saturating_sub(if below { 2 } else { 1 }),
            );
            // Render album/artist art image or placeholder
            let mut rendered_image = false;
            let mosaic_key = crate::app::artwork_mosaic::key(item);
            let art_key = mosaic_key.as_deref().or(match item {
                BrowseItem::Album { key, .. } => Some(key.as_str()),
                BrowseItem::Artist { key, .. } => Some(key.as_str()),
                BrowseItem::ArtistInfo { artist_key, .. } => Some(artist_key.as_str()),
                _ => None,
            });
            if let Some(key) = art_key
                && let Some(data) = state.artwork.grid_cache.get(key)
            {
                rendered_image = super::artwork::render_grid_image(frame, image_area, key, data);
            }

            if !rendered_image {
                super::artwork::render_placeholder(
                    frame,
                    image_area,
                    if matches!(
                        item,
                        BrowseItem::Artist { .. } | BrowseItem::ArtistInfo { .. }
                    ) {
                        super::artwork::Placeholder::Artist
                    } else {
                        super::artwork::Placeholder::Album
                    },
                );
            }

            let caption = match item {
                BrowseItem::Album { title, year, .. } => {
                    crate::app::view_options::album_caption(title, *year)
                }
                _ => item.title().into(),
            };
            let style = if is_selected {
                selection_style
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            frame.render_widget(
                Paragraph::new(truncate_middle(&caption, caption_area.width as usize))
                    .alignment(Alignment::Left)
                    .style(style),
                caption_area,
            );
        }
        row_y += row_height;

        // Add spacer row after last one-row item before art items
        if has_spacer_after(display_idx) {
            row_y += 1;
        }
    }

    // Scrollbar + position indicator
    if total_items > visible_count {
        let sb_border = if is_focused {
            Some(t.colors.border_focused)
        } else {
            None
        };
        render_scrollbar(
            frame,
            col_area,
            total_items,
            visible_count,
            scroll_offset,
            sb_border,
        );

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

/// A compact switch-only popup; management lives in Settings → Libraries.
fn render_library_picker(frame: &mut Frame, state: &AppState) {
    // Underlying Settings may have registered tabs; none belong to this modal.
    state.hit_regions.borrow_mut().library_manager_rows = None;
    state
        .hit_regions
        .borrow_mut()
        .library_manager_actions
        .clear();
    // A quick switcher overlays, but never replaces, the normal app.
    // ┌ Switch library ─────────────────────────────┐
    // │ > Music · server       Beatles · WebDAV       │
    // │ Enter switch · F2 manage · Esc cancel       │
    // └────────────────────────────────────────────┘
    let t = theme();
    let count = crate::app::sources::choices(state).len();
    let screen = frame.area();
    let width = 84.min(screen.width.saturating_sub(4));
    let height = ((count.max(1) + 4).min(20) as u16).min(screen.height.saturating_sub(2));
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    super::widgets::register_popup_close(state, area);
    let block = Block::default()
        .title(" Switch library ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    render_library_choices(frame, state, area, rows[0]);
    frame.render_widget(
        Paragraph::new("Enter switch · F2 manage · Esc cancel")
            .style(Style::default().fg(t.colors.fg_muted)),
        rows[1],
    );
}

/// Settings content, not a startup screen or full-window modal.
pub(super) fn render_library_manager(frame: &mut Frame, state: &AppState, inner: Rect) {
    // One library list, with the selected source's details below it. The category
    // pane stays visible. Add, manage and re-scan are directly accessible here.
    use screens::settings::{panel, paper};
    let focused = state.settings_state.focus == crate::app::state::SettingsFocus::Content;
    let block = panel(" Libraries ")
        .borders(Borders::TOP)
        .border_style(paper().fg(if focused {
            theme().colors.border_focused
        } else {
            theme().colors.border
        }));
    let content = block.inner(inner);
    frame.render_widget(block, inner);
    let active = crate::app::sources::library_choices(state)
        .into_iter()
        .find(|entry| entry.active(state))
        .map_or_else(|| "None".into(), |entry| entry.label());
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(if content.height >= 12 { 4 } else { 0 }),
        Constraint::Length(if content.height < 6 {
            0
        } else if content.width < 29 {
            3
        } else if content.width < 48 {
            2
        } else {
            1
        }),
    ])
    .split(content);
    frame.render_widget(
        Paragraph::new(format!("Active: {active}")).style(paper().fg(theme().colors.fg_muted)),
        rows[0],
    );
    render_managed_libraries(frame, state, rows[1]);
    let choice = crate::app::sources::choices(state)
        .get(state.popups.library_picker_index)
        .cloned();
    let detail = match &choice {
        Some(crate::app::sources::LibraryChoice::Add) | None => {
            "Add a local folder, WebDAV share, or Subsonic / Navidrome server.".to_string()
        }
        Some(choice) => format!(
            "{} · {}\n{}\n{}",
            choice.label(),
            choice.provider(),
            choice.location(),
            crate::app::sources::cache::status(choice, state)
        ),
    };
    frame.render_widget(
        Paragraph::new(detail)
            .style(paper().fg(theme().colors.fg_muted))
            .wrap(Wrap { trim: true }),
        rows[2],
    );
    // The same actions are available by keyboard and as labelled buttons.
    let mut buttons = vec![("A Add", crate::app::sources::LibraryChoice::Add.action())];
    if let Some(choice) = choice.filter(|choice| choice.cache_store().is_ok()) {
        buttons.push((
            "Enter Manage",
            crate::app::action::SettingsAction::ManageLibrary(choice.clone()).into(),
        ));
        buttons.push((
            if crate::app::sources::cache::running(&choice, state) {
                "F5 Cancel scan"
            } else {
                "F5 Re-scan"
            },
            crate::app::action::SettingsAction::RescanSourceCache(choice).into(),
        ));
    }
    let mut x = rows[3].x;
    let mut y = rows[3].y;
    for (label, action) in buttons {
        let wanted = label.len() as u16 + 4;
        if x != rows[3].x && x + wanted > rows[3].right() {
            x = rows[3].x;
            y += 1;
        }
        let width = wanted.min(rows[3].right().saturating_sub(x));
        let rect = Rect::new(x, y, width, u16::from(y < rows[3].bottom()));
        if !rect.is_empty() {
            frame.render_widget(
                Paragraph::new(format!("[ {label} ]")).style(paper().fg(theme().colors.fg_accent)),
                rect,
            );
            state
                .hit_regions
                .borrow_mut()
                .library_manager_actions
                .push((rect, action));
        }
        x = x.saturating_add(width + 1);
    }
}

fn render_managed_libraries(frame: &mut Frame, state: &AppState, area: Rect) {
    use screens::settings::{paper, selected};
    let entries = crate::app::sources::choices(state);
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    let mut previous = String::new();
    let mut selected_line = 0;
    for (index, entry) in entries.iter().enumerate() {
        let group = entry.group();
        if group != previous {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            if !group.is_empty() {
                lines.push(Line::styled(
                    group.clone(),
                    paper().fg(theme().colors.fg_accent),
                ));
            }
            previous = group;
        }
        if index == state.popups.library_picker_index {
            selected_line = lines.len();
        }
        let is_selected = index == state.popups.library_picker_index
            && state.settings_state.focus == crate::app::state::SettingsFocus::Content;
        let style = if is_selected { selected() } else { paper() };
        let name = match entry {
            crate::app::sources::LibraryChoice::Navidrome { name, .. } => name.clone(),
            _ => entry.label(),
        };
        rows.push((lines.len(), index));
        lines.push(Line::styled(
            format!(
                "{}{}{}",
                if is_selected { "> " } else { "  " },
                if entry.active(state) { "[Active] " } else { "" },
                name
            ),
            style,
        ));
    }
    let total = lines.len();
    let offset = state
        .sources
        .picker_scroll_pin
        .unwrap_or_else(|| selected_line.saturating_sub(area.height.saturating_sub(1) as usize))
        .min(total.saturating_sub(area.height as usize));
    let rows = rows
        .into_iter()
        .filter_map(|(line, index)| {
            let y = line.checked_sub(offset)?;
            (y < area.height as usize)
                .then_some((Rect::new(area.x, area.y + y as u16, area.width, 1), index))
        })
        .collect();
    state.hit_regions.borrow_mut().library_manager_rows =
        Some(crate::app::presentation::SettingsContentRegion {
            inner: area,
            rows,
            scroll_offset: offset,
            total_lines: total,
        });
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), area);
    screens::settings::scroll_marker(
        frame,
        Rect::new(area.right(), area.y, 1, area.height),
        total,
        offset,
    );
}

fn render_library_choices(frame: &mut Frame, state: &AppState, outer: Rect, list_area: Rect) {
    let t = theme();
    let entries = crate::app::sources::choices(state);
    let offset = crate::app::sources::picker_offset(state, list_area.height as usize);
    state.hit_regions.borrow_mut().library_picker =
        Some(crate::app::presentation::PopupListRegions {
            outer,
            items_area: list_area,
            item_count: entries.len(),
        });
    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_area.height as usize)
        .map(|(i, entry)| {
            let selected = i == state.popups.library_picker_index
                && (state.popups.library_picker_active()
                    || state.settings_state.focus == crate::app::state::SettingsFocus::Content);
            let style = if selected {
                t.colors.focused_row()
            } else {
                Style::default().fg(t.colors.fg_primary)
            };
            ListItem::new(format!(
                "{}{}{} · {}",
                if selected { "> " } else { "  " },
                if entry.active(state) { "[Active] " } else { "" },
                entry.label(),
                entry.provider(),
            ))
            .style(style)
        })
        .collect();
    frame.render_widget(List::new(items), list_area);
    if entries.is_empty() {
        let message = if state.popups.library_picker_active() {
            "No saved libraries. F2 to add or reconnect."
        } else {
            "No libraries found.\nUse Add library to connect a source."
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(t.colors.fg_muted)),
            list_area,
        );
    }
}

/// Render the artist bio popup (F4): prose beside the selected photo,
/// full-width prose below it; left/right selects another article photo.
/// Down past the prose focuses the Google button; Up returns, Tab switches directly.
fn render_artist_bio_popup(frame: &mut Frame, state: &AppState) {
    state.hit_regions.borrow_mut().artist_links.clear();
    state.hit_regions.borrow_mut().detail_links.clear();
    let popup = match state.popups.artist_bio() {
        Some(p) => p,
        None => return,
    };

    let t = theme();
    let area = centered_rect(70, 60, frame.area());

    frame.render_widget(Clear, area);
    super::widgets::register_popup_close(state, area);

    let title = format!(" {} ", popup.artist_name);
    let hint = if popup.google_focused {
        " ↑ back · Enter search · G Google · Esc close "
    } else if popup.document.source_url.is_some() {
        " ↑↓ scroll · click artist: library · Tab button · ←→ photos · B source · G Google · Esc close "
    } else {
        " ↑↓ scroll · click artist: library · Tab button · G Google · Esc close "
    };
    let block = Block::default()
        .title(title)
        .title_bottom(hint)
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state
        .sources
        .active
        .capabilities()
        .supports(crate::library::capabilities::Feature::ArtistRadio)
        && inner.height > 1
    {
        let radio = Rect::new(inner.x, inner.y, inner.width.min(20), 1);
        frame.render_widget(
            Paragraph::new("[ Artist Mix · R ]").style(Style::default().fg(t.colors.fg_accent)),
            radio,
        );
        state.hit_regions.borrow_mut().biography_radio = Some(radio);
    }
    // Biography / photos above a fixed, keyboard- and mouse-accessible browser button.
    let mut bio_area = inner;
    if state.hit_regions.borrow().biography_radio.is_some() {
        bio_area.y += 1;
        bio_area.height = bio_area.height.saturating_sub(1);
    }
    if !inner.is_empty() {
        bio_area.height = bio_area.height.saturating_sub(1);
        let label = format!("[ {} ]", state.web_search_provider.mark());
        let width = (label.len() as u16).min(inner.width);
        let button = Rect::new(
            inner.x + (inner.width - width) / 2,
            inner.bottom() - 1,
            width,
            1,
        );
        frame.render_widget(
            Paragraph::new(label).style(if popup.google_focused {
                t.colors.focused_row()
            } else {
                Style::default().fg(t.colors.fg_primary)
            }),
            button,
        );
        state.hit_regions.borrow_mut().biography_google = Some(button);
    }
    state.hit_regions.borrow_mut().biography_text =
        Some(crate::app::presentation::ScrollableTextRegion {
            area: bio_area,
            lines: 0,
            visible: bio_area.height as usize,
        });

    if popup.task.is_some() {
        let loading = Paragraph::new("Loading biography...")
            .style(Style::default().fg(t.colors.fg_muted))
            .alignment(Alignment::Center);
        frame.render_widget(loading, bio_area);
        return;
    }

    // Determine artwork size and whether to show it
    let photo = popup.document.images.get(popup.image_index);
    let has_artwork = photo.is_some();
    // Drive layout from height: fill ~60% of bio area vertically, then derive width
    // from height so a square image fills the rect exactly (terminal cells ≈ 2:1 aspect).
    let art_h = if has_artwork {
        let target = (bio_area.height * 3) / 5; // ~60% of bio area
        target.max(6).min(bio_area.height.saturating_sub(2))
    } else {
        0
    };
    let art_w = if has_artwork {
        (art_h * 2).min(bio_area.width / 2)
    } else {
        0
    };
    // 1 col gap between text and artwork
    let gap = if has_artwork && art_w > 0 { 1u16 } else { 0 };

    // Word-wrap bio text: narrow lines next to artwork, full-width lines below
    let full_width = bio_area.width as usize;
    let narrow_width = bio_area.width.saturating_sub(art_w + gap) as usize;
    let art_rows = art_h as usize;
    let mut text = match photo.filter(|p| !p.caption.is_empty()) {
        Some(photo) => format!(
            "Photo {}/{}: {}\n\n{}",
            popup.image_index + 1,
            popup.document.images.len(),
            photo.caption,
            popup.document.text
        ),
        None => popup.document.text.clone(),
    };
    if !popup.document.related_artists.is_empty() {
        text.push_str("\n\nRelated Artists\n");
        for artist in &popup.document.related_artists {
            text.push_str(&artist.title);
            text.push('\n');
        }
    }
    let wrapped = wrap_bio_text(&text, narrow_width, full_width, art_rows);
    let total_lines = wrapped.len() as u16;
    let visible = bio_area.height;
    let scroll = popup.scroll.min(total_lines.saturating_sub(visible));
    state.hit_regions.borrow_mut().biography_text =
        Some(crate::app::presentation::ScrollableTextRegion {
            area: bio_area,
            lines: wrapped.len(),
            visible: visible as usize,
        });

    // Render artwork scrolling with text: crop top rows as user scrolls down.
    let art_visible_h = art_h.saturating_sub(scroll);
    if has_artwork && art_w > 0 && art_visible_h > 0 {
        let art_rect = Rect {
            x: bio_area.x + bio_area.width - art_w,
            y: bio_area.y,
            width: art_w,
            height: art_visible_h,
        };
        if let Some(photo) = photo {
            BIO_ARTWORK_RENDERER.with(|renderer| {
                let mut renderer = renderer.borrow_mut();
                let crop_fraction = if scroll > 0 {
                    scroll as f32 / art_h as f32
                } else {
                    0.0
                };
                if renderer.load_image_cropped(&photo.data, &photo.key, crop_fraction) {
                    renderer.render(frame, art_rect);
                }
            });
        }
    }

    // Render visible text lines
    let style = Style::default().fg(t.colors.fg_primary);
    for (screen_row, line_text) in wrapped
        .iter()
        .skip(scroll as usize)
        .take(visible as usize)
        .enumerate()
    {
        let y = bio_area.y + screen_row as u16;
        // Narrow width only when artwork is visible on this screen row
        let in_art_zone = has_artwork && (screen_row as u16) < art_visible_h;
        let line_width = if in_art_zone {
            narrow_width as u16
        } else {
            bio_area.width
        };
        let line_rect = Rect {
            x: bio_area.x,
            y,
            width: line_width,
            height: 1,
        };
        let p = Paragraph::new(line_text.as_str()).style(style);
        frame.render_widget(p, line_rect);
        for artist in &popup.document.related_artists {
            crate::ui::artist_links::text(
                state,
                line_rect,
                line_text,
                &artist.title,
                Some(&artist.rating_key),
            );
        }
    }
    crate::ui::artist_links::visible(
        frame,
        state,
        Rect::new(area.x, area.y, area.width, 1),
        &popup.artist_name,
        Some(&popup.artist_key),
    );

    // Scrollbar
    if total_lines > visible {
        render_scrollbar(
            frame,
            area,
            total_lines as usize,
            visible as usize,
            scroll as usize,
            None,
        );
    }
}

/// Word-wrap bio text with a narrow region (next to artwork) and full-width below.
/// The first `narrow_rows` output lines are wrapped at `narrow_width`;
/// subsequent lines are wrapped at `full_width`.
fn wrap_bio_text(
    text: &str,
    narrow_width: usize,
    full_width: usize,
    narrow_rows: usize,
) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;

    if full_width == 0 {
        return vec![];
    }
    // If no artwork, everything is full width
    let narrow_width = if narrow_width == 0 || narrow_width >= full_width {
        full_width
    } else {
        narrow_width
    };

    let mut lines = Vec::new();

    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }

        let words: Vec<&str> = paragraph.split_whitespace().collect();
        let mut line = String::new();
        let mut line_width = 0usize;

        for word in &words {
            // Widths are terminal cells, not bytes — bios with accented or
            // CJK text would otherwise wrap at the wrong column.
            let word_width = UnicodeWidthStr::width(*word);
            let max_w = if lines.len() < narrow_rows {
                narrow_width
            } else {
                full_width
            };
            if line.is_empty() {
                line.push_str(word);
                line_width = word_width;
            } else if line_width + 1 + word_width <= max_w {
                line.push(' ');
                line.push_str(word);
                line_width += 1 + word_width;
            } else {
                lines.push(line);
                line = word.to_string();
                line_width = word_width;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }

    lines
}

fn render_error_popup(frame: &mut Frame, state: &AppState, error: &str) {
    let t = theme();
    let area = centered_rect(70, 35, frame.area());

    frame.render_widget(Clear, area);
    super::widgets::register_popup_close(state, area);

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

/// Wrap plain input/metadata without discarding characters in a narrow pane.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        return Vec::new();
    }
    let mut rows = vec![String::new()];
    let mut row_width = 0;
    for word in text.split_whitespace() {
        let word_width = unicode_width::UnicodeWidthStr::width(word);
        if row_width > 0 && row_width + 1 + word_width <= width {
            rows.last_mut().unwrap().push(' ');
            row_width += 1;
        } else if row_width > 0 {
            rows.push(String::new());
            row_width = 0;
        }
        for ch in word.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if row_width > 0 && row_width + ch_width > width {
                rows.push(String::new());
                row_width = 0;
            }
            rows.last_mut().unwrap().push(ch);
            row_width += ch_width;
        }
    }
    rows
}

fn render_input_dialog(frame: &mut Frame, state: &AppState, dialog: &InputDialog) {
    let t = theme();
    let describe = matches!(
        dialog.action_type,
        crate::app::state::InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe)
    );
    let area = if describe {
        // Leave room for a sentence or two; longer input scrolls as it wraps.
        let bounds = frame.area();
        let width = bounds.width.saturating_sub(4).min(68);
        let height = bounds.height.saturating_sub(2).min(8);
        Rect::new(
            bounds.x + (bounds.width - width) / 2,
            bounds.y + (bounds.height - height) / 2,
            width,
            height,
        )
    } else {
        centered_rect(50, 25, frame.area())
    };

    frame.render_widget(Clear, area);
    super::widgets::register_popup_close(state, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border))
        .style(Style::default().bg(t.colors.bg_primary));

    let block = if state.view == View::Settings {
        screens::settings::panel(format!(" {} ", dialog.title))
    } else {
        block
    };
    let text_style = if state.view == View::Settings {
        screens::settings::paper()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let input_area = Rect {
        y: inner.y,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let hint_area = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    let input_text = dialog.input.as_str();
    if describe {
        let rows = wrap_text(input_text, input_area.width as usize);
        let start = rows.len().saturating_sub(input_area.height as usize);
        frame.render_widget(
            Paragraph::new(rows[start..].join("\n")).style(text_style),
            input_area,
        );
    } else {
        frame.render_widget(Paragraph::new(input_text).style(text_style), input_area);
    }

    if describe {
        crate::ui::widgets::text_cursor(frame, input_area, input_text);
    } else {
        widgets::text_input::render(
            frame,
            state,
            input_area,
            crate::app::terminal_text::Target::Input(dialog.title.clone()),
            text_style,
        );
    }

    // Hint text
    let hint = Paragraph::new(format!(
        "Enter: {}  |  Esc: Cancel",
        dialog.action_type.submit_label()
    ))
    .style(text_style)
    .alignment(Alignment::Center);
    frame.render_widget(hint, hint_area);
}

fn render_confirm_dialog(frame: &mut Frame, state: &AppState, dialog: &ConfirmDialog) {
    let t = theme();
    let area = if state.view == View::Settings {
        let width = frame.area().width.saturating_sub(4).min(72);
        let height = frame.area().height.saturating_sub(2).min(11);
        Rect::new(
            (frame.area().width - width) / 2,
            (frame.area().height - height) / 2,
            width,
            height,
        )
    } else {
        centered_rect(50, 25, frame.area())
    };

    frame.render_widget(Clear, area);
    super::widgets::register_popup_close(state, area);

    let block = Block::default()
        .title(format!(" {} ", dialog.title))
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));

    let block = if state.view == View::Settings {
        screens::settings::panel(format!(" {} ", dialog.title))
    } else {
        block
    };
    let text_style = if state.view == View::Settings {
        screens::settings::paper()
    } else {
        Style::default().fg(t.colors.fg_primary)
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Message
    let msg = Paragraph::new(dialog.message.as_str())
        .style(text_style)
        .wrap(Wrap { trim: true });
    let msg_area = Rect {
        height: inner.height.saturating_sub(2),
        ..inner
    };
    frame.render_widget(msg, msg_area);

    // Button row at bottom of inner area
    let btn_y = inner.y + inner.height.saturating_sub(1);
    let yes_text = "  Yes  ";
    let no_text = "  No  ";
    let yes_x = inner.x + 1;
    let no_x = yes_x + yes_text.len() as u16 + 2;

    let yes_area = Rect {
        x: yes_x,
        y: btn_y,
        width: yes_text.len() as u16,
        height: 1,
    };
    let no_area = Rect {
        x: no_x,
        y: btn_y,
        width: no_text.len() as u16,
        height: 1,
    };

    // Register hit regions
    {
        let mut hr = state.hit_regions.borrow_mut();
        hr.confirm_dialog = Some(crate::app::presentation::DialogRegions {
            outer: area,
            yes_button: yes_area,
            no_button: no_area,
        });
    }

    // Highlight the selected button with accent, dim the other
    let (yes_style, no_style) = if dialog.selected_yes {
        (
            Style::default()
                .fg(t.colors.bg_primary)
                .bg(t.colors.fg_accent),
            Style::default()
                .fg(t.colors.fg_muted)
                .bg(t.colors.bg_secondary),
        )
    } else {
        (
            Style::default()
                .fg(t.colors.fg_muted)
                .bg(t.colors.bg_secondary),
            Style::default()
                .fg(t.colors.bg_primary)
                .bg(t.colors.fg_accent),
        )
    };

    let (yes_style, no_style) = if state.view == View::Settings {
        if dialog.selected_yes {
            (screens::settings::selected(), screens::settings::paper())
        } else {
            (screens::settings::paper(), screens::settings::selected())
        }
    } else {
        (yes_style, no_style)
    };
    frame.render_widget(Paragraph::new(yes_text).style(yes_style), yes_area);
    frame.render_widget(Paragraph::new(no_text).style(no_style), no_area);

    // Hint text
    let hint_y = btn_y.saturating_sub(1);
    if hint_y > inner.y + 1 {
        let hint = Paragraph::new("Y/N or Enter to confirm")
            .style(text_style)
            .alignment(Alignment::Center);
        let hint_area = Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(hint, hint_area);
    }
}

/// Check if a mouse click hit a confirm dialog button. Returns Some(true) for Yes, Some(false) for No, None for miss.
pub fn confirm_dialog_hit_test(
    dialog: &ConfirmDialog,
    frame_area: Rect,
    col: u16,
    row: u16,
) -> Option<bool> {
    let area = centered_rect(50, 25, frame_area);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);

    let btn_y = inner.y + inner.height.saturating_sub(1);
    if row != btn_y {
        return None;
    }

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

fn album_detail_lines<'a>(
    state: &AppState,
    column: &'a crate::app::state::BrowseColumn,
) -> Option<std::borrow::Cow<'a, [String]>> {
    let mut lines = column.detail_lines()?;
    if let Some(crate::app::state::BrowseDetails::Album { artwork_key, .. }) = &column.details
        && let Some(notes) = &state.sources.album_notes
        && notes.matches(state.library_generation, artwork_key)
    {
        if !notes.info.review.is_empty() {
            let text: String = notes
                .info
                .review
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            lines.to_mut().push(String::new());
            lines.to_mut().extend(text.lines().map(str::to_owned));
        }
        if let Some(link) = notes.info.display_link() {
            lines.to_mut().push(link.to_string());
        }
    }
    Some(lines)
}
