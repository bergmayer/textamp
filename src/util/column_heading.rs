//! Display names and timing for truncated library column headers.
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const DELAY: Duration = Duration::from_secs(10);
const END_HOLD: Duration = Duration::from_secs(2);

pub fn destination(title: &str) -> &str {
    [" — ", " – ", " - ", ": "]
        .into_iter()
        .find_map(|separator| {
            let (kind, name) = title.split_once(separator)?;
            [
                "album",
                "albums",
                "artist",
                "artists",
                "track",
                "tracks",
                "folder",
                "folders",
                "playlist",
                "playlists",
                "artist details",
                "album details",
                "track details",
                "details",
            ]
            .iter()
            .any(|label| kind.eq_ignore_ascii_case(label))
            .then_some(name)
        })
        .unwrap_or(title)
}

/// The initial center ellipsis returns after the trailing edge has been readable for two seconds.
pub fn offset(elapsed: Duration, overflow: f32, speed: f32) -> Option<f32> {
    if overflow <= 0. {
        return None;
    }
    let travel = overflow / speed;
    let phase = elapsed.as_secs_f32() % (DELAY.as_secs_f32() + travel + END_HOLD.as_secs_f32());
    (phase >= DELAY.as_secs_f32()).then(|| ((phase - DELAY.as_secs_f32()) * speed).min(overflow))
}

pub fn cells(title: &str, width: usize, elapsed: Duration) -> String {
    let total = title.width();
    if total <= width {
        return title.into();
    }
    let graphemes = title.graphemes(true);
    if let Some(offset) = offset(elapsed, (total - width) as f32, 5.) {
        let mut skipped = 0;
        let mut used = 0;
        return graphemes
            .skip_while(|g| {
                if skipped >= offset as usize {
                    return false;
                }
                skipped += g.width();
                true
            })
            .take_while(|g| {
                used += g.width();
                used <= width
            })
            .collect();
    }
    if width == 0 {
        return String::new();
    }
    let room = width - 1;
    let mut used = 0;
    let left: String = graphemes
        .clone()
        .take_while(|g| {
            used += g.width();
            used <= room.div_ceil(2)
        })
        .collect();
    let mut right = title.len();
    used = 0;
    for g in graphemes.rev() {
        used += g.width();
        if used > room - left.width() {
            break;
        }
        right -= g.len();
    }
    format!("{left}…{}", &title[right..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_header_pauses_then_reveals_both_ends() {
        let title = "ab界cdEF";
        assert_eq!(cells(title, 5, Duration::from_secs(9)), "ab…EF");
        assert_eq!(cells(title, 5, DELAY), "ab界c");
        assert!(cells(title, 5, DELAY + Duration::from_secs(1)).ends_with("EF"));
        assert_eq!(cells("e\u{301}23456", 3, Duration::ZERO), "e\u{301}…6");
        assert_eq!(destination("album - Sgt. Pepper"), "Sgt. Pepper");
        assert_eq!(destination("A - B"), "A - B");
    }
}
