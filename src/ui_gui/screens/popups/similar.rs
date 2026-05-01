//! "Show Similar" modal popup.
//!
//! Invoked from a right-click context menu entry (Tracks / Albums /
//! Artists). Reads `state.similar` directly — the `DataAction::Load*`
//! handlers populate that struct asynchronously; we just render what's
//! there and show a loading hint until the results arrive.

use iced::widget::{button, column, container, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::BrowseAction;
use crate::app::state::SimilarMode;
use crate::app::{Action, AppState};
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let mode_label = match state.similar.mode {
        SimilarMode::Albums => "Similar Albums",
        SimilarMode::Tracks => "Similar Tracks",
        SimilarMode::Artists => "Similar Artists",
    };
    let title_line = text(format!(
        "{mode_label} \u{2014} {}",
        if state.similar.source_title.is_empty() { "(no source)" } else { state.similar.source_title.as_str() }
    ))
    .size(20);

    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseSimilarPopup)
        .style(popout_button_style);

    let toolbar = iced::widget::row![
        title_line,
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let list: Element<'_, GuiMessage> = if state.similar.loading {
        container(text("Loading similar\u{2026}").size(16))
            .padding(24)
            .center_x(Length::Fill)
            .into()
    } else {
        let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
        match state.similar.mode {
            SimilarMode::Albums => {
                if state.similar.albums.is_empty() {
                    rows.push(empty_row("No similar albums found."));
                } else {
                    for a in &state.similar.albums {
                        let label = format!(
                            "{}  \u{2014} {}",
                            a.title,
                            a.parent_title.as_deref().unwrap_or(""),
                        );
                        // Click navigates to the album in the Library
                        // (artist + album drill) — does NOT auto-play.
                        // Right-clicking a row in Library to play /
                        // queue is the explicit add-to-queue path.
                        let actions: Option<Vec<Action>> = a.parent_rating_key
                            .clone()
                            .map(|artist_key| vec![Action::Browse(BrowseAction::OpenInLibrary {
                                artist_key,
                                artist_name: a.parent_title.clone().unwrap_or_default(),
                                album_key: Some(a.rating_key.clone()),
                                album_title: Some(a.title.clone()),
                            })]);
                        rows.push(click_row(label, actions.unwrap_or_default()));
                    }
                }
            }
            SimilarMode::Tracks => {
                if state.similar.tracks.is_empty() {
                    rows.push(empty_row("No similar tracks found."));
                } else {
                    for t in &state.similar.tracks {
                        let label = format!("{}  \u{2014} {}", t.title, t.track_artist());
                        // Same "navigate, don't play" rule as Albums:
                        // click drills the user into the track's album
                        // page so they see context. To play it, they
                        // can right-click → Play track in the library
                        // pane (or the track-details pane's Play Track).
                        let actions: Option<Vec<Action>> = t.grandparent_rating_key
                            .clone()
                            .map(|artist_key| vec![Action::Browse(BrowseAction::OpenInLibrary {
                                artist_key,
                                artist_name: t.artist_name().to_string(),
                                album_key: t.parent_rating_key.clone(),
                                album_title: t.parent_title.clone(),
                            })]);
                        rows.push(click_row(label, actions.unwrap_or_default()));
                    }
                }
            }
            SimilarMode::Artists => {
                if state.similar.artists.is_empty() {
                    rows.push(empty_row("No similar artists found."));
                } else {
                    for ar in &state.similar.artists {
                        let artist_key = ar.rating_key.clone();
                        let label = ar.title.clone();
                        // Click → navigate to the artist in the library
                        // (matches user expectation: tapping an artist
                        // takes you to them, not to yet another similar
                        // list). Closes the popup.
                        rows.push(
                            button(text(label).size(15))
                                .width(Length::Fill)
                                .padding([6, 12])
                                .on_press(GuiMessage::NavigateToArtist { artist_key })
                                .style(row_style)
                                .into(),
                        );
                    }
                }
            }
        }
        scrollable(Column::with_children(rows).spacing(2)).height(Length::Fill).into()
    };

    container(
        column![
            toolbar,
            container(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(2)
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        border: Border { color: p.background.strong.color, width: 1.0, radius: 2.0.into() },
                        ..container::Style::default()
                    }
                }),
        ]
        .spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(720.0))
    .height(Length::Fixed(560.0))
    .style(frame_style)
    .into()
}

/// A clickable row. Click dispatches the bundle of actions and closes
/// the popup (the shared `TabClick` handler in `App::update` clears
/// the `similar_popup_open` flag before dispatching).
fn click_row(label: String, actions: Vec<Action>) -> Element<'static, GuiMessage> {
    button(text(label).size(15))
        .width(Length::Fill)
        .padding([6, 12])
        .on_press_with(move || GuiMessage::TabClick(actions.clone()))
        .style(row_style)
        .into()
}

fn empty_row(msg: &'static str) -> Element<'static, GuiMessage> {
    container(text(msg).size(15))
        .padding(24)
        .center_x(Length::Fill)
        .into()
}

fn row_style(theme: &Theme, status: button::Status) -> button::Style {
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
}

fn frame_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
