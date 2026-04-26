//! Global search popup (Ctrl+F).
//!
//! Single-screen UI: search input at the top, category tabs below
//! that, then the filtered result list. Clicking a result navigates
//! to it in the Library / Playlists / Genres view via the existing
//! `SelectSearchResult` handler so the destination's Miller columns
//! end up in the same state as if the user had drilled there
//! manually.

use iced::widget::{button, column, container, row, scrollable, text, text_input, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{Action, SearchAction};
use crate::app::state::SearchTab;
use crate::app::AppState;
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

const POPUP_WIDTH: f32 = 640.0;
const RESULTS_HEIGHT: f32 = 420.0;

pub fn view<'a>(state: &'a AppState) -> Element<'a, GuiMessage> {
    let selected_idx = state.list_state.search_item_index;
    let active_tab = state.search.tab;

    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStatePopup(StatePopupKind::Search))
        .style(popout_button_style);
    let header = row![
        text("Search").size(16),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // Live-edit text input. Updates `state.search.query` and re-runs
    // the search on each keystroke; Enter triggers the canonical
    // `ExecuteLocalSearch` so the GUI matches the TUI's prompt.
    let query_input = text_input("Search artists, albums, tracks\u{2026}", &state.search.query)
        .on_input(|s| GuiMessage::Action(Action::Search(SearchAction::SetSearchQuery(s))))
        .on_submit(GuiMessage::Action(Action::Search(SearchAction::ExecuteLocalSearch)))
        .size(13)
        .padding(6)
        .width(Length::Fill);

    // Category tabs. `Global` shows everything; the other tabs filter
    // to a single result category. Clicking a tab dispatches a small
    // GuiMessage that updates `state.search.tab` and resets the
    // selected index back to 0.
    let tabs = row![
        tab_btn("All", active_tab == SearchTab::Global, SearchTab::Global),
        tab_btn("Artists", active_tab == SearchTab::Artists, SearchTab::Artists),
        tab_btn("Albums", active_tab == SearchTab::Albums, SearchTab::Albums),
        tab_btn("Tracks", active_tab == SearchTab::Tracks, SearchTab::Tracks),
        tab_btn("Playlists", active_tab == SearchTab::Playlists, SearchTab::Playlists),
        tab_btn("Genres", active_tab == SearchTab::Genres, SearchTab::Genres),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let mut rows: Vec<Element<'a, GuiMessage>> = Vec::new();
    let mut flat_idx: usize = 0;

    if let Some(results) = &state.search.results {
        let show_artists = matches!(active_tab, SearchTab::Global | SearchTab::Artists);
        let show_albums = matches!(active_tab, SearchTab::Global | SearchTab::Albums);
        let show_tracks = matches!(active_tab, SearchTab::Global | SearchTab::Tracks);
        let show_playlists = matches!(active_tab, SearchTab::Global | SearchTab::Playlists);
        let show_genres = matches!(active_tab, SearchTab::Global | SearchTab::Genres);

        // Render order: artists → albums → playlists → genres → tracks.
        // Must match `resolve_global_index` in dispatch_search.rs so a
        // click on flat_idx N maps to the same item the resolver picks.
        if show_artists && !results.artists.is_empty() {
            if active_tab == SearchTab::Global { rows.push(section_header("Artists")); }
            for a in &results.artists {
                rows.push(result_row(format!("  {}", a.title), flat_idx, flat_idx == selected_idx));
                flat_idx += 1;
            }
        }
        if show_albums && !results.albums.is_empty() {
            if active_tab == SearchTab::Global { rows.push(section_header("Albums")); }
            for a in &results.albums {
                rows.push(result_row(
                    format!("  {}  \u{2014} {}", a.title, a.parent_title.as_deref().unwrap_or("")),
                    flat_idx,
                    flat_idx == selected_idx,
                ));
                flat_idx += 1;
            }
        }
        if show_playlists && !results.playlists.is_empty() {
            if active_tab == SearchTab::Global { rows.push(section_header("Playlists")); }
            for p in &results.playlists {
                rows.push(result_row(format!("  {}", p.title), flat_idx, flat_idx == selected_idx));
                flat_idx += 1;
            }
        }
        if show_genres && !results.genres.is_empty() {
            if active_tab == SearchTab::Global { rows.push(section_header("Genres")); }
            for g in &results.genres {
                rows.push(result_row(format!("  {}", g.title), flat_idx, flat_idx == selected_idx));
                flat_idx += 1;
            }
        }
        if show_tracks && !results.tracks.is_empty() {
            if active_tab == SearchTab::Global { rows.push(section_header("Tracks")); }
            for t in &results.tracks {
                rows.push(result_row(
                    format!("  {}  \u{2014} {}", t.title, t.track_artist()),
                    flat_idx,
                    flat_idx == selected_idx,
                ));
                flat_idx += 1;
            }
        }
        if rows.is_empty() {
            rows.push(text("No results in this category.").size(12).into());
        }
    } else if state.search.query.is_empty() {
        rows.push(text("Type to search\u{2026}").size(12).into());
    } else {
        rows.push(text("Searching\u{2026} (Enter to retry)").size(12).into());
    }

    let body = scrollable(Column::with_children(rows).spacing(2))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .height(Length::Fixed(RESULTS_HEIGHT));

    container(
        column![header, query_input, tabs, body].spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(POPUP_WIDTH))
    .style(|theme: &Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(p.background.base.color)),
            text_color: Some(p.background.base.text),
            border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
            ..container::Style::default()
        }
    })
    .into()
}

fn tab_btn<'a>(label: &'static str, active: bool, tab: SearchTab) -> Element<'a, GuiMessage> {
    button(text(label).size(12))
        .padding([4, 12])
        .on_press(GuiMessage::Action(Action::Search(SearchAction::SetSearchTab(tab))))
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = if active {
                (p.primary.strong.color, p.primary.strong.text)
            } else {
                match status {
                    button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                    _ => (p.background.weak.color, p.background.base.text),
                }
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border { color: p.background.strong.color, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

fn section_header(label: &'static str) -> Element<'static, GuiMessage> {
    container(text(format!("\u{2014} {} \u{2014}", label)).size(11))
        .padding([4, 6])
        .into()
}

fn result_row(label: String, idx: usize, is_selected: bool) -> Element<'static, GuiMessage> {
    button(text(label).size(12))
        .width(Length::Fill)
        .padding([4, 10])
        .on_press(GuiMessage::SearchPopupClick(idx))
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = if is_selected {
                (p.primary.strong.color, p.primary.strong.text)
            } else {
                match status {
                    button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
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
        .into()
}
