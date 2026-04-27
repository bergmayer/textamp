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

use crate::app::state::PlaybackMode;
use crate::app::{Action, AppState};
use crate::ui_gui::images;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::{fmt_ms, popout_button_style, toggle_button_style};

const LEFT_PANEL_WIDTH: f32 = 260.0;
/// Side length of the big "now-playing" artwork that lives in the
/// right-hand column of the Queue / Now Playing view. Sized so the
/// album cover dominates the right side without crowding out the
/// queue list in the middle.
const RIGHT_ART_SIDE: f32 = 480.0;
/// Right-column outer width = artwork + a few px breathing room.
const RIGHT_PANEL_WIDTH: f32 = 488.0;

pub fn view<'a>(
    state: &'a AppState,
    dragging: Option<usize>,
    stations_popup_open: bool,
    dj_modes_popup_open: bool,
    remix_tools_popup_open: bool,
) -> Element<'a, GuiMessage> {
    let artwork = big_artwork_view(state);
    let radio_btn = radio_button(state, stations_popup_open);
    let random_album_btn = random_album_button(state);
    let dj_modes_btn = dj_modes_button(state, dj_modes_popup_open);
    let remix_tools_btn = remix_tools_button(remix_tools_popup_open);
    let save_queue_btn = save_queue_button(state);
    let clear_queue_btn = clear_queue_button(state);
    let queue = track_list(state, dragging);

    // Left column: buttons only, stacked from the top. The artwork
    // moved to the right column and the visualizer toggle was removed
    // (visualizer is always-on now) so the column is shorter.
    let left = iced_column![
        radio_btn,
        random_album_btn,
        dj_modes_btn,
        remix_tools_btn,
        save_queue_btn,
        clear_queue_btn,
    ]
    .spacing(10)
    .width(Length::Fixed(LEFT_PANEL_WIDTH));

    // TOP HALF: buttons | queue list | big artwork.
    let top = iced_row![
        left,
        container(queue).width(Length::Fill).height(Length::Fill),
        iced_column![artwork]
            .width(Length::Fixed(RIGHT_PANEL_WIDTH))
            .height(Length::Fill),
    ]
    .spacing(12);

    // BOTTOM HALF: visualizer, full window width, always rendered.
    // Tabs (waveform / spectrum / spectrogram) inside the panel let
    // the user pick the mode without an extra "show / hide" toggle.
    let viz = super::now_playing::visualizer_panel(state);

    let body = iced_column![
        container(top).width(Length::Fill).height(Length::FillPortion(1)),
        container(viz).width(Length::Fill).height(Length::FillPortion(1)),
    ]
    .spacing(8)
    .padding(12);

    container(body).width(Length::Fill).height(Length::Fill).into()
}

/// Big right-side album artwork. Square at `RIGHT_ART_SIDE` per side;
/// shows the current track's album art via the same caches that drive
/// the in-row Miller-column thumbnails. Falls back to a "no cover"
/// placeholder when no track is loaded or no art is cached.
fn big_artwork_view(state: &AppState) -> Element<'_, GuiMessage> {
    let art_el: Element<'_, GuiMessage> = if let Some(bytes) = state.artwork.current_data.as_ref() {
        image(images::handle_from_bytes(bytes))
            .width(Length::Fixed(RIGHT_ART_SIDE))
            .height(Length::Fixed(RIGHT_ART_SIDE))
            .into()
    } else if let Some(track) = state.current_track() {
        if let Some(key) = track.parent_rating_key.as_ref() {
            if let Some(handle) = images::lookup_grid(&state.artwork.grid_cache, key) {
                image(handle)
                    .width(Length::Fixed(RIGHT_ART_SIDE))
                    .height(Length::Fixed(RIGHT_ART_SIDE))
                    .into()
            } else {
                big_placeholder_art()
            }
        } else {
            big_placeholder_art()
        }
    } else {
        big_placeholder_art()
    };

    container(art_el)
        .width(Length::Fixed(RIGHT_ART_SIDE))
        .height(Length::Fixed(RIGHT_ART_SIDE))
        .center_x(Length::Fixed(RIGHT_ART_SIDE))
        .center_y(Length::Fixed(RIGHT_ART_SIDE))
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

fn big_placeholder_art<'a>() -> Element<'a, GuiMessage> {
    container(text("no cover").size(13))
        .width(Length::Fixed(RIGHT_ART_SIDE))
        .height(Length::Fixed(RIGHT_ART_SIDE))
        .center_x(Length::Fixed(RIGHT_ART_SIDE))
        .center_y(Length::Fixed(RIGHT_ART_SIDE))
        .into()
}

/// "Radio" launcher — opens the stations popup. Renders depressed when
/// any radio station is playing (PlaybackMode::Radio, regardless of
/// kind) or while the stations popup is open. The sidebar's "Play
/// Random Album" button now uses one-shot `PlayAlbumNow` and stays in
/// PlaybackMode::Queue, so it never trips this; the only thing that
/// activates Radio mode is an actual radio station from the popup.
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

/// "Play Random Album" — picks one random album from the active
/// library, clears the queue, and starts playing it. ONE-SHOT: when
/// the album finishes, playback ends. The continuous "Random Album
/// Radio" station (which keeps queuing fresh random albums forever)
/// is a separate thing reachable via the Radio button → stations
/// popup; conflating them would mislabel which control is driving
/// playback. Disabled when no album library is loaded.
fn random_album_button(state: &AppState) -> Element<'_, GuiMessage> {
    let enabled = !state.library.albums.is_empty();
    button(
        container(text("Play Random Album").size(13))
            .center_y(Length::Fixed(36.0))
            .center_x(Length::Fill)
            .padding([0, 12]),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press_maybe(if enabled { Some(GuiMessage::PlayOneRandomAlbum) } else { None })
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

/// "Save Queue as Playlist…" — promoted out of Remix Tools because
/// saving the queue isn't a remix operation. Disabled when nothing is
/// queued. Triggers the same `PromptSavePlaylist` flow as the queue
/// context menu and Cmd+S shortcut.
fn save_queue_button(state: &AppState) -> Element<'_, GuiMessage> {
    use crate::app::action::QueueAction;
    let any_queued = !state.queue.tracks.is_empty() || !state.radio.tracks.is_empty();
    let action = if any_queued {
        Some(GuiMessage::Action(Action::Queue(QueueAction::PromptSavePlaylist)))
    } else {
        None
    };
    button(
        container(text("Save Queue as Playlist\u{2026}").size(13))
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

/// "Clear Queue" — promoted out of Remix Tools for the same reason.
/// Disabled when the queue is already empty.
fn clear_queue_button(state: &AppState) -> Element<'_, GuiMessage> {
    use crate::app::action::QueueAction;
    let any_queued = !state.queue.tracks.is_empty() || !state.radio.tracks.is_empty();
    let action = if any_queued {
        Some(GuiMessage::Action(Action::Queue(QueueAction::ClearQueue)))
    } else {
        None
    };
    button(
        container(text("Clear Queue").size(13))
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
            // Multi-selection: rows in `state.queue.selected` get the
            // selection swatch even when they aren't the keyboard
            // cursor / playing track. The set is populated by
            // shift+click (range) or cmd-click (toggle) in
            // `QueueDragStart`. A bullet prefix marks them so the
            // selection is legible in monochrome themes that collapse
            // primary/background.
            let is_multi = state.queue.selected.contains(&idx);
            let prefix = if is_current && is_multi {
                "♪●"
            } else if is_current {
                "> "
            } else if is_multi {
                "● "
            } else {
                "  "
            };
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
                    } else if is_multi {
                        (p.primary.weak.color, p.primary.weak.text)
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
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
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
