//! Exact rendered artist spans, shared by terminal metadata and result lists.
use super::RenderState;
use crate::app::presentation::ArtistLink;

use ratatui::{Frame, layout::Rect};
use unicode_width::UnicodeWidthStr;

fn strip_the(value: &str) -> &str {
    let value = value.trim();
    if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("the "))
    {
        &value[4..]
    } else {
        value
    }
}

pub fn register(state: &RenderState<'_>, area: Rect, name: &str, hint: Option<&str>) {
    if area.is_empty() || name.trim().is_empty() {
        return;
    }
    let normalized = strip_the(name);
    let artists = || {
        state
            .library
            .artists
            .iter()
            .chain(&state.library.track_artists)
    };
    let matches = |artist: &&crate::library::models::Artist| {
        strip_the(&artist.title).eq_ignore_ascii_case(normalized)
            || strip_the(&artist.title)
                .chars()
                .flat_map(char::to_lowercase)
                .eq(normalized.chars().flat_map(char::to_lowercase))
    };
    let known = hint
        .and_then(|key| artists().find(|artist| artist.rating_key == key && matches(artist)))
        .or_else(|| artists().find(matches));
    let Some(artist) = known else { return };
    state
        .hit_regions
        .borrow_mut()
        .artist_links
        .push(ArtistLink {
            area,
            generation: state.library_generation,
            key: artist.rating_key.clone(),
            name: artist.title.clone(),
        });
}
/// Find the artist in one actual formatted row, including padding and clipping.
pub fn text(state: &RenderState<'_>, area: Rect, text: &str, name: &str, hint: Option<&str>) {
    if name.contains(';') {
        for artist in name
            .split(';')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            self::text(state, area, text, artist, hint);
        }
        return;
    }
    let (offset, shown_width) = if let Some(offset) = text.rfind(name) {
        (offset, UnicodeWidthStr::width(name))
    } else {
        let prefix = text.trim().split('…').next().unwrap_or_default().trim_end();
        if prefix.chars().count() < 2 || !name.starts_with(prefix) {
            return;
        }
        (
            text.find(prefix).unwrap_or(0),
            UnicodeWidthStr::width(prefix),
        )
    };
    let x = UnicodeWidthStr::width(&text[..offset]) as u16;
    if x >= area.width {
        return;
    }
    register(
        state,
        Rect::new(
            area.x + x,
            area.y,
            (shown_width as u16).min(area.width - x),
            1,
        ),
        name,
        hint,
    );
}
/// Wrapped prose and metadata register only artist names actually on screen.
pub fn visible(
    frame: &mut Frame<'_>,
    state: &RenderState<'_>,
    area: Rect,
    name: &str,
    hint: Option<&str>,
) {
    let area = area.intersection(frame.area());
    for y in area.y..area.bottom() {
        let mut line = String::new();
        let mut x = area.x;
        while x < area.right() {
            let symbol = frame.buffer_mut()[(x, y)].symbol();
            line.push_str(symbol);
            x = x.saturating_add((UnicodeWidthStr::width(symbol) as u16).max(1));
        }
        text(
            state,
            Rect::new(area.x, y, area.width, 1),
            &line,
            name,
            hint,
        );
    }
}
