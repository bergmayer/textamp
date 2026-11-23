//! Track and album destinations attached to their rendered terminal text.
use crate::app::{library_details::Destination, presentation::DetailLink};
use crate::library::models::Track;
use ratatui::{Frame, layout::Rect};
use unicode_width::UnicodeWidthStr;

/// Register only occupied cells of a field containing one metadata value.
pub fn field(
    frame: &mut Frame<'_>,
    state: &super::RenderState<'_>,
    area: Rect,
    track: &Track,
    destination: Destination,
) {
    let area = area.intersection(frame.area());
    for y in area.y..area.bottom() {
        let mut left = area.right();
        let mut right = area.x;
        for x in area.x..area.right() {
            let symbol = frame.buffer_mut()[(x, y)].symbol();
            if !symbol.trim().is_empty() {
                left = left.min(x);
                right = right.max(x.saturating_add(UnicodeWidthStr::width(symbol) as u16));
            }
        }
        if right > left {
            register(
                state,
                Rect::new(left, y, right.min(area.right()) - left, 1),
                track,
                destination,
            );
        }
    }
}
fn register(state: &super::RenderState<'_>, area: Rect, track: &Track, destination: Destination) {
    if area.is_empty() {
        return;
    }
    state
        .hit_regions
        .borrow_mut()
        .detail_links
        .push(DetailLink {
            area,
            generation: state.library_generation,
            track_key: track.rating_key.clone(),
            playback_id: state.playback.request_id,
            destination,
        });
}
