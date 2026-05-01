//! Bottom-of-window transport bar: track info, progress, play/pause, volume.
//!
//! Reads everything it needs from `AppState` — no widget-owned state. All
//! controls emit `Action`s via `GuiMessage::Action(...)`.

use iced::widget::{button, column, container, row, slider, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Shadow, Theme, Vector};

use crate::ui_gui::widgets::text;
use crate::app::action::PlaybackAction;
use crate::app::{Action, AppState};
use crate::app::state::PlayStatus;
use crate::ui_gui::message::GuiMessage;

/// Shared "button-y" style — bevelled, lozenge-shaped, pops out from
/// the surrounding chrome via a strong drop shadow. Idle = raised,
/// Hovered = brightened, Pressed = recessed into the surface. The
/// pressed state is deliberately exaggerated:
///   - background drops to the darker `background.strong` swatch
///     (instead of the primary fill colour) so the button looks
///     pushed *into* its parent rather than just shaded;
///   - border switches to a thick, dark inset stroke;
///   - drop shadow flips to a tall negative-Y inner shadow so the
///     button reads as below the surrounding panel.
/// This guarantees the pressed look is unmistakable in every theme,
/// not just the dark default.
pub fn popout_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    if matches!(status, button::Status::Pressed) {
        return pressed_style(palette);
    }
    // Each state pulls (bg, fg) from the SAME pair so iced's
    // `readable()` guarantee holds in every theme — including
    // strict-monochrome (Black and White), where mixing pairs
    // (e.g. primary.weak.color + primary.strong.text) collapses to
    // white-on-white or black-on-black.
    let (bg, fg, shadow_offset, shadow_alpha, blur, border_w) = match status {
        button::Status::Hovered => (
            palette.primary.base.color,
            palette.primary.base.text,
            Vector::new(0.0, 3.0),
            0.55,
            6.0,
            1.5,
        ),
        button::Status::Disabled => (
            palette.background.weak.color,
            palette.background.weak.text,
            Vector::new(0.0, 0.0),
            0.0,
            0.0,
            1.0,
        ),
        _ => (
            palette.primary.weak.color,
            palette.primary.weak.text,
            Vector::new(0.0, 3.0),
            0.50,
            6.0,
            1.5,
        ),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: palette.background.strong.color,
            width: border_w,
            // Half of the smallest button height — gives a true
            // lozenge / pill outline. Iced clamps oversize radii
            // internally, so larger buttons still look pill-shaped
            // but never visually break.
            radius: 999.0.into(),
        },
        shadow: Shadow {
            color: Color { r: 0.0, g: 0.0, b: 0.0, a: shadow_alpha },
            offset: shadow_offset,
            blur_radius: blur,
        },
    }
}

/// Visually-recessed style shared by transient `Pressed` events on
/// `popout_button_style` and sticky `active=true` toggle buttons.
/// Every theme sees the same bg darken / border inset / shadow flip
/// so the pressed look is unmistakable. Text uses
/// `background.strong.text` rather than `primary.strong.color` —
/// iced's `Pair::text` is guaranteed readable on its `color`, so this
/// avoids the black-on-black collapse the strict Black and White
/// theme would otherwise produce.
fn pressed_style(palette: &iced::theme::palette::Extended) -> button::Style {
    let bg = palette.background.strong.color;
    let fg = palette.background.strong.text;
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: palette.background.strong.color,
            width: 2.5,
            radius: 999.0.into(),
        },
        shadow: Shadow {
            color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.65 },
            offset: Vector::new(0.0, -3.0),
            blur_radius: 5.0,
        },
    }
}

/// Shared primary-action button — used for "Play Track" (track details
/// pane), "Play Album" (album-tracks column header), and "Artist Radio"
/// (action item in artist column). Shrinks to its label so it reads as
/// a button instead of a list row, with a small fixed height that pairs
/// with the `999.0` lozenge radius from `popout_button_style`. Callers
/// are responsible for centring the button inside its row.
pub fn primary_action_button<'a>(
    label: impl Into<String>,
    on_press: crate::ui_gui::message::GuiMessage,
) -> iced::widget::Button<'a, crate::ui_gui::message::GuiMessage> {
    use iced::widget::{button};
    use iced::Length;
    // Wrapping::None keeps action labels (e.g. "Artist Radio - <long
    // artist name>") on a single line; the surrounding column header
    // clips overflow rather than letting the button wrap into a
    // multi-line lozenge.
    button(text(label.into()).size(15).wrapping(iced::widget::text::Wrapping::None))
        .padding([4, 18])
        .width(Length::Shrink)
        .height(Length::Fixed(28.0))
        .on_press(on_press)
        .style(popout_button_style)
}

/// Sticky toggle-button style — when `pressed` is true the button
/// renders inset (depressed look) using the same exaggerated
/// recessed treatment as a transient press. When false it renders
/// raised like a regular `popout` button. Used for the Visualizer
/// toggle so the user can tell at a glance whether the panel is
/// open.
pub fn toggle_button_style(pressed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, _status: button::Status| {
        let palette = theme.extended_palette();
        if pressed {
            pressed_style(palette)
        } else {
            popout_button_style(theme, button::Status::Active)
        }
    }
}

/// Format milliseconds as `m:ss` (or `h:mm:ss` past an hour).
pub fn fmt_ms(ms: u64) -> String {
    let total_s = ms / 1000;
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let track = state.current_track();

    // Title / artist / album lines.
    let title_text = track.map(|t| t.title.clone()).unwrap_or_else(|| "No track".to_string());
    let artist_text = track.map(|t| t.track_artist().to_string()).unwrap_or_default();
    let album_text = track.and_then(|t| t.parent_title.clone()).unwrap_or_default();

    let track_info = column![
        text(title_text).size(16),
        text(format!("{artist_text}  \u{2014}  {album_text}"))
            .size(13),
    ]
    .width(Length::FillPortion(3));

    // Play/pause + prev/next buttons. Labels use plain ASCII so they
    // render identically on every platform's default font (no emoji fallback
    // to the "?" placeholder glyph).
    let play_label = match state.playback.status {
        PlayStatus::Playing   => "||",    // pause
        PlayStatus::Paused    => ">",     // play
        PlayStatus::Stopped   => ">",
        PlayStatus::Buffering => "...",
    };
    let play_btn = button(text(play_label).size(16))
        .on_press(GuiMessage::Action(Action::Playback(PlaybackAction::TogglePlayPause)))
        .padding([6, 14])
        .style(popout_button_style);
    let prev_btn = button(text("<<").size(14))
        .on_press(GuiMessage::Action(Action::Playback(PlaybackAction::Previous)))
        .padding([6, 12])
        .style(popout_button_style);
    let next_btn = button(text(">>").size(14))
        .on_press(GuiMessage::Action(Action::Playback(PlaybackAction::Next)))
        .padding([6, 12])
        .style(popout_button_style);

    // Seek slider (position / duration).
    let dur = state.playback.duration_ms.max(1) as f32;
    let pos = (state.playback.position_ms as f32).min(dur);
    let seek_slider = slider(0.0..=dur, pos, |ms| {
        GuiMessage::Action(Action::Playback(PlaybackAction::Seek(ms as u64)))
    })
    .width(Length::Fill);
    let pos_label = text(fmt_ms(state.playback.position_ms)).size(13);
    let dur_label = text(fmt_ms(state.playback.duration_ms)).size(13);

    let controls = row![prev_btn, play_btn, next_btn].spacing(4).align_y(Alignment::Center);
    let seek_row = row![pos_label, seek_slider, dur_label]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::FillPortion(4));

    // Tab strip — Library / Now Playing — embedded directly in the
    // transport. Keeps the persistent view-switcher on screen even
    // when the user's mouse is near the controls. Volume lives in
    // Settings now (keyboard shortcuts + Playback menu still work).
    let tabs = crate::ui_gui::widgets::tab_strip::inline_view(state);

    let bar = row![
        controls,
        track_info,
        seek_row,
        Space::new(Length::Fixed(12.0), Length::Shrink),
        tabs,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding([6, 12]);

    // Error banner shown above the transport when something has gone
    // wrong (e.g. "Audio unavailable…"). Keeps the user informed without
    // blocking interaction.
    let maybe_banner: Option<Element<'_, GuiMessage>> = state
        .notifications
        .last_error
        .as_ref()
        .map(|msg| {
            container(text(msg.clone()).size(13))
                .padding([4, 12])
                .width(Length::Fill)
                .style(|theme: &iced::Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(Background::Color(p.danger.base.color)),
                        text_color: Some(p.danger.base.text),
                        ..container::Style::default()
                    }
                })
                .into()
        });

    let stacked: Element<'_, GuiMessage> = match maybe_banner {
        Some(banner) => column![banner, bar].spacing(0).into(),
        None => bar.into(),
    };

    container(stacked)
        .width(Length::Fill)
        .style(|theme: &iced::Theme| {
            let p = theme.extended_palette();
            container::Style {
                // Match the menu bar: weak background swatch with a
                // 1 px strong-coloured outline. In themes that mix
                // weak away from base this still reads as a subtle
                // separator chrome; in the strict Black and White
                // theme weak == base == pure white, so the only
                // separator is the thin black border.
                background: Some(iced::Background::Color(p.background.weak.color)),
                text_color: Some(p.background.weak.text),
                border: iced::Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
