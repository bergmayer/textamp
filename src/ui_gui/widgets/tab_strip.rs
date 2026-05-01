//! Primary-view tab strip: Library / Now Playing.
//!
//! Lives in the transport bar (bottom of the window). The active tab
//! is drawn with the strong primary-colour swatch; inactive tabs show
//! a subtle hover state.
//!
//! - Library     → Browse view + Library category
//! - Now Playing → Queue view (combined queue list + optional
//!                 visualizer panel)

use iced::widget::{button, container, row, text_input};
use iced::{Alignment, Background, Border, Element, Length, Shadow, Theme};

use crate::app::action::{NavigationAction, SearchAction};
use crate::app::state::{BrowseCategory, View};
use crate::app::{Action, AppState};
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
/// Build the message that the filter text_input fires on Enter.
///
/// If the filter has a query: leave fast-filter mode, open the global
/// Search popup, and seed it with the same query so the user can
/// continue typing without re-entering the term. If empty: no-op.
fn submit_filter_msg(query: &str) -> GuiMessage {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        GuiMessage::Noop
    } else {
        GuiMessage::TabClick(vec![
            Action::Search(SearchAction::OpenSearchPopup),
            Action::Search(SearchAction::SetSearchQuery(trimmed.to_string())),
        ])
    }
}

const STRIP_HEIGHT: u16 = 28;
const TAB_WIDTH: f32 = 130.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Library,
    NowPlaying,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::NowPlaying => "Now Playing",
        }
    }

    fn action(self) -> Vec<Action> {
        match self {
            Tab::Library => vec![
                Action::Navigation(NavigationAction::set_category(BrowseCategory::Library)),
                Action::Navigation(NavigationAction::SetView(View::Browse)),
            ],
            // The unified Now Playing screen uses View::Queue under
            // the hood (the queue::view renderer is now the home
            // for both queue and visualizer).
            Tab::NowPlaying => vec![Action::Navigation(NavigationAction::SetView(View::Queue))],
        }
    }

    fn is_active(self, state: &AppState) -> bool {
        match self {
            // Any Browse-style view counts as "Library" — the user
            // sees Library / Genres / Folders / Playlists as facets
            // of the same library tab, so drilling into a playlist
            // shouldn't visually disown the Library tab.
            Tab::Library => state.view == View::Browse,
            Tab::NowPlaying => matches!(state.view, View::Queue | View::NowPlaying),
        }
    }
}

/// Inline tab strip embedded inside the transport bar. Order is
/// Library, Now Playing, then the quick-filter input.
///
/// The filter input is always visible (even on Now Playing) so the
/// user can issue a search from anywhere. Typing filters the active
/// browse list when the Library view is up; pressing Enter while a
/// query is non-empty leaves fast-filter mode and opens the global
/// search popup seeded with the same term.
pub fn inline_view(state: &AppState) -> Element<'_, GuiMessage> {
    let mut bar = row![].spacing(0).align_y(Alignment::Center);
    bar = bar.push(tab_button(Tab::Library, Tab::Library.is_active(state)));
    bar = bar.push(tab_button(Tab::NowPlaying, Tab::NowPlaying.is_active(state)));

    let filter_value = state.list_filter.query.clone();
    let placeholder = "filter / search\u{2026}";
    let input = text_input(placeholder, &filter_value)
        .on_input(GuiMessage::FilterChanged)
        .on_submit(submit_filter_msg(&filter_value))
        .size(14)
        .padding(4)
        .width(Length::Fixed(180.0));
    let mut filter_row = row![input].spacing(4).align_y(Alignment::Center);
    // Standard X-button to clear the filter when it's non-empty.
    if !filter_value.is_empty() {
        // Standard "clear search" affordance — a round badge with
        // an X glyph, drawn against the same neutral palette as
        // the input chrome so it reads as part of the input.
        let clear_btn = button(text("x").size(13))
            .padding([0, 6])
            .height(Length::Fixed(20.0))
            .on_press(GuiMessage::Action(Action::Search(SearchAction::DeactivateListFilter)))
            .style(|theme: &Theme, status: button::Status| {
                let p = theme.extended_palette();
                let (bg, fg) = match status {
                    button::Status::Hovered => (p.danger.base.color, p.danger.base.text),
                    button::Status::Pressed => (p.danger.strong.color, p.danger.strong.text),
                    _ => (p.background.strong.color, p.background.strong.text),
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border {
                        color: p.background.strong.color,
                        width: 1.0,
                        radius: 999.0.into(),
                    },
                    ..button::Style::default()
                }
            });
        filter_row = filter_row.push(clear_btn);
    }
    bar = bar.push(
        container(filter_row)
            .padding([0, 8])
            .align_y(Alignment::Center)
            .height(Length::Fixed(STRIP_HEIGHT as f32)),
    );

    container(bar)
        .height(Length::Fixed(STRIP_HEIGHT as f32))
        .into()
}

fn tab_button(tab: Tab, active: bool) -> Element<'static, GuiMessage> {
    let label = text(tab.label()).size(16);
    let inner = container(label)
        .center_y(Length::Fixed(STRIP_HEIGHT as f32 - 2.0))
        .center_x(Length::Fixed(TAB_WIDTH))
        .padding([0, 12]);

    let action_list = tab.action();

    // Tab visual chrome — square (no border-radius, no shadow), so
    // the row reads as a strip of tabs rather than two pill buttons.
    // Colours match the sidebar toggle buttons: an active tab uses
    // the recessed/pressed swatch (background.strong fill +
    // primary.strong text) and an inactive tab uses the raised
    // popout swatch (primary.weak fill + primary.strong text).
    button(inner)
        .width(Length::Fixed(TAB_WIDTH))
        .padding(0)
        .on_press_with(move || GuiMessage::TabClick(action_list.clone()))
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            // Pair-consistent (bg, fg) — `Pair::text` is guaranteed
            // readable on `Pair::color` so each theme renders both an
            // active and an inactive tab without ever collapsing to
            // same-colour text on bg.
            //
            // The Black & White theme's `primary.weak` is white bg /
            // black text, which makes the active tab vanish into the
            // body (also white). Flip the active/inactive swatches in
            // that one theme so the active tab is the black-on-white
            // chip and inactive tabs read as raised white plates.
            let is_bw = crate::ui_gui::theme::is_monochrome(theme);
            let (bg, fg) = if active {
                if is_bw {
                    (p.background.strong.color, p.background.strong.text)
                } else {
                    (p.primary.weak.color, p.primary.weak.text)
                }
            } else {
                let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
                if hover {
                    (p.primary.base.color, p.primary.base.text)
                } else if is_bw {
                    (p.primary.weak.color, p.primary.weak.text)
                } else {
                    (p.background.strong.color, p.background.strong.text)
                }
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                shadow: Shadow::default(),
            }
        })
        .into()
}
