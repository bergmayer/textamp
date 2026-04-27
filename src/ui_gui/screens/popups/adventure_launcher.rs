//! Sonic Adventure launcher popup.
//!
//! Single-screen form: shows the start track, track count, and end
//! track all at once, with a search panel for picking either track
//! and a Reverse button for swapping start ↔ end.
//!
//! Each row is independently clickable so the user can drop into
//! "pick start" / "pick end" mode at will. The popup pre-fills the
//! start slot when launched from a track context (right-click on a
//! track → Sonic Adventure…).

use iced::widget::{button, column, container, row, scrollable, text, text_input, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{Action, SearchAction};
use crate::app::state::{AdventureDrillLevel, AdventureLauncherState, AdventureStep};
use crate::plex::models::Track;
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

const POPUP_WIDTH: f32 = 600.0;

pub fn view<'a>(p: &'a AdventureLauncherState) -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStatePopup(StatePopupKind::AdventureLauncher))
        .style(popout_button_style);
    let header = row![
        text("Sonic Adventure").size(16),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // The three always-visible fields. The active field (the one the
    // search panel will fill in) is highlighted via field_card's
    // `active` flag so the user can see at a glance where the next
    // selected track will land.
    let start_card = field_card(
        "Start track",
        p.start_track.as_ref(),
        p.step == AdventureStep::FindStartTrack,
        AdventureStep::FindStartTrack,
        SearchAction::AdventureLauncherClearStart,
    );
    let end_card = field_card(
        "End track",
        p.end_track.as_ref(),
        p.step == AdventureStep::FindEndTrack,
        AdventureStep::FindEndTrack,
        SearchAction::AdventureLauncherClearEnd,
    );

    // Track count input — always editable. The dispatcher filters out
    // non-digit characters, so we can wire `on_input` directly to it
    // without re-validating here.
    let count_input = text_input("20", &p.track_count_input)
        .on_input(|s| GuiMessage::Action(Action::Search(SearchAction::AdventureLauncherSetTrackCount(s))))
        .size(13)
        .padding(4)
        .width(Length::Fixed(80.0));
    let count_row = row![
        container(text("Track count").size(13))
            .width(Length::Fixed(96.0))
            .align_y(Alignment::Center)
            .height(Length::Fixed(28.0)),
        count_input,
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    // Reverse button: swap start ↔ end. Greyed out when both slots
    // are empty (nothing to swap). One slot empty is still a valid
    // swap — the other slot ends up empty afterwards.
    let reverse_enabled = p.start_track.is_some() || p.end_track.is_some();
    let reverse_btn = action_button("\u{21C4} Reverse", reverse_enabled,
        Action::Search(SearchAction::AdventureLauncherReverse));

    // Generate button: needs both tracks AND a non-zero count.
    let count_valid = p.track_count_input.parse::<usize>().map_or(false, |n| n >= 5);
    let generate_enabled = p.start_track.is_some() && p.end_track.is_some() && count_valid;
    let generate_btn = action_button("Generate", generate_enabled,
        Action::Search(SearchAction::AdventureLauncherGenerate));

    let action_row = row![reverse_btn, Space::with_width(Length::Fill), generate_btn]
        .align_y(Alignment::Center);

    // Search panel: visible whenever the user is currently editing
    // start or end (the EnterTrackCount step has no search relevance,
    // but we never put the launcher into that step in the new flow —
    // it only appears as an inert default if both tracks are set).
    let search_panel = if matches!(p.step, AdventureStep::FindStartTrack | AdventureStep::FindEndTrack) {
        Some(search_panel(p))
    } else {
        None
    };

    let mut body = column![
        start_card,
        count_row,
        end_card,
        Space::with_height(Length::Fixed(4.0)),
        action_row,
    ]
    .spacing(10);
    if let Some(panel) = search_panel {
        body = body.push(Space::with_height(Length::Fixed(4.0)));
        body = body.push(panel);
    }

    container(column![header, body].spacing(12))
        .padding(18)
        .width(Length::Fixed(POPUP_WIDTH))
        .style(frame_style)
        .into()
}

/// One of the three field rows (Start track / End track). Shows the
/// currently-selected track (or a placeholder), with a "Pick" button
/// that activates this field for the search panel and an X-button
/// that clears the slot.
fn field_card<'a>(
    label: &'a str,
    track: Option<&'a Track>,
    active: bool,
    activate_step: AdventureStep,
    clear_action: SearchAction,
) -> Element<'a, GuiMessage> {
    let (track_line, has_track) = match track {
        Some(t) => (
            format!("{}  \u{2014} {}", t.title, t.track_artist()),
            true,
        ),
        None => ("(not selected)".to_string(), false),
    };

    let pick_label = if has_track { "Change\u{2026}" } else { "Pick\u{2026}" };
    let pick_btn = button(text(pick_label).size(12))
        .padding([4, 12])
        .on_press(GuiMessage::Action(Action::Search(SearchAction::AdventureLauncherSetStep(activate_step))))
        .style(popout_button_style);

    let mut right_buttons = row![pick_btn].spacing(6);
    if has_track {
        let clear_btn = button(text("x").size(12))
            .padding([4, 10])
            .on_press(GuiMessage::Action(Action::Search(clear_action)))
            .style(popout_button_style);
        right_buttons = right_buttons.push(clear_btn);
    }

    let label_col = container(text(label).size(13))
        .width(Length::Fixed(96.0))
        .align_y(Alignment::Center)
        .height(Length::Fixed(28.0));

    let track_text = container(text(track_line).size(13))
        .width(Length::Fill)
        .padding([4, 8])
        .style(move |theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(if active {
                    p.primary.weak.color
                } else {
                    p.background.weak.color
                })),
                text_color: Some(if has_track {
                    p.background.base.text
                } else {
                    p.background.weak.text
                }),
                border: Border {
                    color: if active { p.primary.strong.color } else { p.background.strong.color },
                    width: if active { 1.5 } else { 1.0 },
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }
        });

    row![label_col, track_text, right_buttons]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

/// Common styling for the Reverse / Generate action buttons. When
/// `enabled` is false the button has no `on_press` so iced renders
/// it in its disabled style and ignores clicks.
fn action_button<'a>(label: &'a str, enabled: bool, action: Action) -> Element<'a, GuiMessage> {
    let btn = button(text(label).size(13)).padding([6, 16]).style(popout_button_style);
    if enabled {
        btn.on_press(GuiMessage::Action(action)).into()
    } else {
        btn.into()
    }
}

/// Search input + result list. The TextInput dispatches
/// `AdventureLauncherSetQuery` on every keystroke, which both updates
/// `state.popups.adventure_launcher.query` and re-runs the search.
fn search_panel<'a>(p: &'a AdventureLauncherState) -> Element<'a, GuiMessage> {
    let query_input = text_input("Search artists / albums / tracks\u{2026}", &p.query)
        .on_input(|s| GuiMessage::Action(Action::Search(SearchAction::AdventureLauncherSetQuery(s))))
        .on_submit(GuiMessage::Action(Action::Search(SearchAction::AdventureLauncherSearch)))
        .size(12)
        .padding(4)
        .width(Length::Fill);

    let mut rows: Vec<Element<'a, GuiMessage>> = Vec::new();
    let mut flat_idx: usize = 0;

    match &p.drill {
        AdventureDrillLevel::Search => {
            if let Some(r) = &p.results {
                // The launcher only ever assigns the picked result
                // into `start_track` or `end_track`, so artist and
                // album rows have no effect — they'd just dispatch
                // a click that mismatches the current step. Show
                // tracks only to keep the search panel focused.
                if !r.tracks.is_empty() {
                    for t in &r.tracks {
                        rows.push(result_row(format!("{}  \u{2014} {}", t.title, t.track_artist()), flat_idx, flat_idx == p.item_index));
                        flat_idx += 1;
                    }
                }
                if rows.is_empty() {
                    rows.push(text("No matching tracks.").size(12).into());
                }
            } else if p.loading {
                rows.push(text("Searching\u{2026}").size(12).into());
            } else if p.query.is_empty() {
                rows.push(text("Type to search for a track\u{2026}").size(12).into());
            } else {
                rows.push(text("Press Enter to search.").size(12).into());
            }
        }
        AdventureDrillLevel::ArtistAlbums { artist_name, albums, .. } => {
            rows.push(section_header(&format!("Albums \u{2014} {}", artist_name)));
            for a in albums {
                rows.push(result_row(a.title.clone(), flat_idx, flat_idx == p.item_index));
                flat_idx += 1;
            }
        }
        AdventureDrillLevel::AlbumTracks { album_title, artist_name, tracks, .. } => {
            rows.push(section_header(&format!("Tracks \u{2014} {} / {}", artist_name, album_title)));
            for t in tracks {
                rows.push(result_row(format!("{}  \u{2014} {}", t.title, t.track_artist()), flat_idx, flat_idx == p.item_index));
                flat_idx += 1;
            }
        }
    }

    let list = scrollable(Column::with_children(rows).spacing(2))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fixed(280.0));

    column![query_input, list].spacing(6).into()
}

fn section_header<'a>(label: &str) -> Element<'a, GuiMessage> {
    container(text(format!("\u{2014} {} \u{2014}", label)).size(11))
        .padding([4, 6])
        .into()
}

fn result_row(label: String, idx: usize, is_selected: bool) -> Element<'static, GuiMessage> {
    button(text(label).size(12))
        .width(Length::Fill)
        .padding([4, 10])
        .on_press(GuiMessage::AdventureLauncherClick(idx))
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

fn frame_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
