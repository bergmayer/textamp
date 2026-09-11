/// Calculate thumb size and position.
pub fn calc_thumb(
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
    track_height: usize,
) -> (usize, usize) {
    let thumb_size = ((visible_items as f64 / total_items as f64) * track_height as f64)
        .ceil()
        .max(1.0) as usize;
    let max_scroll = total_items.saturating_sub(visible_items);
    let thumb_pos = if max_scroll > 0 {
        ((scroll_offset as f64 / max_scroll as f64) * (track_height - thumb_size) as f64).round()
            as usize
    } else {
        0
    };
    (thumb_size, thumb_pos)
}

/// Convert a mouse Y position into a scroll offset by reversing the thumb position math.
pub fn scroll_offset_from_y(
    mouse_y: u16,
    track_y_start: u16,
    track_height: u16,
    total_items: usize,
    visible_items: usize,
    grab_offset: u16,
) -> usize {
    if track_height == 0 || total_items == 0 || visible_items >= total_items {
        return 0;
    }

    let th = track_height as usize;
    let (thumb_size, _) = calc_thumb(total_items, visible_items, 0, th);
    let max_thumb_pos = th.saturating_sub(thumb_size);
    if max_thumb_pos == 0 {
        return 0;
    }

    // Where the top of the thumb should be based on mouse position
    let thumb_top = (mouse_y
        .saturating_sub(track_y_start)
        .saturating_sub(grab_offset)) as usize;
    let clamped = thumb_top.min(max_thumb_pos);

    let max_scroll = total_items.saturating_sub(visible_items);
    // Reverse: thumb_pos = (scroll / max_scroll) * max_thumb_pos
    //       => scroll = thumb_pos * max_scroll / max_thumb_pos
    let offset = (clamped as f64 / max_thumb_pos as f64 * max_scroll as f64).round() as usize;
    offset.min(max_scroll)
}
