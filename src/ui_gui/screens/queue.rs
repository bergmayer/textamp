//! Queue view — artwork + sidebar buttons (left) + track list (right).
//!
//! The sidebar hosts the current artwork plus a stack of toggle
//! buttons that each open a modal popup or flip a panel: Radio,
//! Play Random Album, Visualizer, DJ Modes, Remix Tools. Buttons
//! that have an "active" state (radio playing, DJ mode running,
//! visualizer panel up, popup open) render in their pressed style
//! so the user always sees what's on at a glance.

use iced::widget::{button, column as iced_column, container, image, mouse_area, row as iced_row, scrollable, text, Column};
use iced::{Background, Border, Color, Element, Length, Padding, Shadow, Theme, Vector};

use crate::app::action::RadioAction;
use crate::app::state::PlaybackMode;
use crate::app::{Action, AppState};
use crate::ui_gui::images;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::{fmt_ms, popout_button_style, toggle_button_style};

const ARTWORK_SIDE: f32 = 240.0;
const LEFT_PANEL_WIDTH: f32 = 260.0;

pub fn view<'a>(
    state: &'a AppState,
    dragging: Option<usize>,
    show_visualizer: bool,
    stations_popup_open: bool,
    dj_modes_popup_open: bool,
    remix_tools_popup_open: bool,
) -> Element<'a, GuiMessage> {
    let artwork = artwork_view(state);
    let radio_btn = radio_button(state, stations_popup_open);
    let random_album_btn = random_album_button(state);
    let visualizer_btn = visualizer_button(show_visualizer);
    let dj_modes_btn = dj_modes_button(state, dj_modes_popup_open);
    let remix_tools_btn = remix_tools_button(remix_tools_popup_open);
    let queue = track_list(state, dragging);

    let left = iced_column![
        artwork,
        radio_btn,
        random_album_btn,
        visualizer_btn,
        dj_modes_btn,
        remix_tools_btn,
    ]
    .spacing(10)
    .width(Length::Fixed(LEFT_PANEL_WIDTH));

    // Right pane: queue (always) and the visualizer panel (only when
    // toggled on, occupying the bottom half of the available height).
    let right: Element<'a, GuiMessage> = if show_visualizer {
        let viz = super::now_playing::visualizer_panel(state);
        iced_column![
            container(queue).height(Length::FillPortion(1)).width(Length::Fill),
            container(viz).height(Length::FillPortion(1)).width(Length::Fill),
        ]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        queue
    };

    let body = iced_row![left, right]
        .spacing(12)
        .padding(12);

    container(body).width(Length::Fill).height(Length::Fill).into()
}

/// Visualizer toggle — same size/style as the Radio and Play Random
/// Album buttons, but uses `toggle_button_style` so it renders inset
/// (depressed) when the panel is currently visible. Clicking it
/// flips `App::show_queue_visualizer`.
fn visualizer_button(active: bool) -> Element<'static, GuiMessage> {
    use crate::ui_gui::widgets::transport_bar::toggle_button_style;
    button(
        container(text("Visualizer").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press(GuiMessage::ToggleQueueVisualizer)
    .style(toggle_button_style(active))
    .into()
}

/// "Radio" launcher — opens the stations popup. The button renders
/// in the depressed / pressed state when EITHER a Plex radio station
/// is currently playing (`PlaybackMode::Radio`) OR the stations
/// popup itself is open. Matches the way the Visualizer toggle
/// stays "pressed" while its panel is showing — clicking the
/// button toggles a related panel, so the depressed look is the
/// "panel is up" indicator.
fn radio_button(state: &AppState, stations_popup_open: bool) -> Element<'_, GuiMessage> {
    use crate::ui_gui::widgets::transport_bar::toggle_button_style;
    let radio_active = state.playback_mode == PlaybackMode::Radio || stations_popup_open;
    button(
        container(text("Radio").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press(GuiMessage::OpenStationsPopup)
    .style(toggle_button_style(radio_active))
    .into()
}

/// "Play Random Album" — kicks off the Plex `randomAlbum` station for
/// the active library. Mirrors the Alt+R keyboard shortcut. Disabled
/// when no library is connected.
fn random_album_button(state: &AppState) -> Element<'_, GuiMessage> {
    let action = state.active_library.as_ref().map(|lib_key| {
        let key = format!("/library/sections/{}/stations/randomAlbum", lib_key);
        GuiMessage::Action(Action::Radio(RadioAction::PlayStation(key)))
    });
    button(
        container(text("Play Random Album").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press_maybe(action)
    .style(popout_button_style)
    .into()
}

/// "DJ Modes" launcher — opens the DJ Modes picker popup. Renders in
/// the pressed style when EITHER a DJ mode is currently active OR the
/// popup itself is open. Same convention as `radio_button` /
/// `visualizer_button` — a depressed look means "the related panel /
/// state is on".
fn dj_modes_button(state: &AppState, popup_open: bool) -> Element<'_, GuiMessage> {
    let active = state.dj.active_mode.is_some() || popup_open;
    button(
        container(text("DJ Modes").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press(GuiMessage::OpenDjModesPopup)
    .style(toggle_button_style(active))
    .into()
}

/// "Remix Tools" launcher — opens the Remix Tools popup. Pressed when
/// the popup is open. Remix actions are one-shot (no persistent
/// "active" state) so the button doesn't stay pressed after a remix
/// runs.
fn remix_tools_button(popup_open: bool) -> Element<'static, GuiMessage> {
    button(
        container(text("Remix Tools").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press(GuiMessage::OpenRemixToolsPopup)
    .style(toggle_button_style(popup_open))
    .into()
}

fn artwork_view(state: &AppState) -> Element<'_, GuiMessage> {
    let art_el: Element<'_, GuiMessage> = if let Some(bytes) = state.artwork.current_data.as_ref() {
        image(images::handle_from_bytes(bytes))
            .width(Length::Fixed(ARTWORK_SIDE))
            .height(Length::Fixed(ARTWORK_SIDE))
            .into()
    } else if let Some(track) = state.current_track() {
        if let Some(key) = track.parent_rating_key.as_ref() {
            if let Some(handle) = images::lookup_grid(&state.artwork.grid_cache, key) {
                image(handle)
                    .width(Length::Fixed(ARTWORK_SIDE))
                    .height(Length::Fixed(ARTWORK_SIDE))
                    .into()
            } else {
                placeholder_art()
            }
        } else {
            placeholder_art()
        }
    } else {
        placeholder_art()
    };

    container(art_el)
        .width(Length::Fixed(LEFT_PANEL_WIDTH))
        .height(Length::Fixed(ARTWORK_SIDE))
        .center_x(Length::Fill)
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.weak.color)),
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

fn placeholder_art<'a>() -> Element<'a, GuiMessage> {
    container(text("no cover").size(12))
        .width(Length::Fixed(ARTWORK_SIDE))
        .height(Length::Fixed(ARTWORK_SIDE))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn track_list<'a>(state: &'a AppState, dragging: Option<usize>) -> Element<'a, GuiMessage> {
    let (tracks, current_idx, mode_label, allow_reorder) = match state.playback_mode {
        PlaybackMode::Radio => (&state.radio.tracks, state.radio.track_index, "radio", false),
        PlaybackMode::Queue | PlaybackMode::None => (&state.queue.tracks, state.queue.index, "queue", true),
    };

    let header_text = format!(" {mode_label} \u{2014} {} tracks ", tracks.len());
    let header = text(header_text).size(12);

    let focused_idx = state.list_state.queue_index;

    let rows = if tracks.is_empty() {
        vec![
            container(text("No tracks queued").size(13))
                .padding(12)
                .width(Length::Fill)
                .into(),
        ]
    } else {
        tracks.iter().enumerate().map(|(idx, t)| {
            let is_current = Some(idx) == current_idx;
            let is_focused = idx == focused_idx;
            let is_dragging = dragging == Some(idx);
            let prefix = if is_current { "> " } else { "  " };
            let duration = fmt_ms(t.duration.unwrap_or(0));
            let label = format!(
                "{prefix}{}  - {}  - {}",
                t.title,
                t.track_artist(),
                duration,
            );

            // Each row is a styled container; the click + drag wiring
            // lives on a `mouse_area` wrapper. on_press starts the
            // gesture (which is decided as click-vs-drag on release);
            // on_enter updates the drop target while dragging;
            // on_right_press opens the context menu.
            //
            // While the row is the active drag source we add a thicker
            // border + drop shadow so it visibly "pops" out of the list
            // — standard reorder feedback. Padding stays the same so
            // the rest of the list doesn't reflow under the dragged
            // row.
            let body = container(text(label).size(13))
                .width(Length::Fill)
                .padding(Padding::from([2, 8]))
                .style(move |theme: &Theme| {
                    let p = theme.extended_palette();
                    let (bg, fg) = if is_dragging {
                        (p.primary.weak.color, p.primary.weak.text)
                    } else if is_current {
                        (p.primary.strong.color, p.primary.strong.text)
                    } else if is_focused {
                        (p.background.weak.color, p.background.weak.text)
                    } else {
                        (Color::TRANSPARENT, p.background.base.text)
                    };
                    let border = if is_dragging {
                        Border {
                            color: p.primary.strong.color,
                            width: 1.5,
                            radius: 4.0.into(),
                        }
                    } else {
                        Border::default()
                    };
                    let shadow = if is_dragging {
                        Shadow {
                            color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 },
                            offset: Vector::new(0.0, 3.0),
                            blur_radius: 6.0,
                        }
                    } else {
                        Shadow::default()
                    };
                    container::Style {
                        background: Some(Background::Color(bg)),
                        text_color: Some(fg),
                        border,
                        shadow,
                        ..container::Style::default()
                    }
                });

            let area = mouse_area(body)
                .on_press(GuiMessage::QueueDragStart(idx))
                .on_enter(GuiMessage::QueueDragOver(idx))
                .on_right_press(GuiMessage::OpenQueueContextMenu { row_index: idx });

            // Radio rows can't be reordered, but we still want click =
            // play (= JumpToRadioTrack via QueueDragEnd). The drag-over
            // wiring is harmless when reordering is disabled because
            // `App::update` no-ops the move when `from == to`.
            let _ = allow_reorder;
            area.into()
        }).collect::<Vec<Element<'a, GuiMessage>>>()
    };

    let list = scrollable(Column::with_children(rows))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .height(Length::Fill);

    container(iced_column![header, list].spacing(6).padding(4))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
