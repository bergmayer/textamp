//! Text and formatting utilities.

/// Format milliseconds as M:SS or H:MM:SS for display.
///
/// # Examples
/// ```
/// use textamp::util::format_duration;
/// assert_eq!(format_duration(62000), "1:02");
/// assert_eq!(format_duration(3661000), "1:01:01");
/// ```
pub fn format_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{}:{:02}", mins, secs)
    }
}

/// Format bytes as human-readable string (KB, MB, GB).
///
/// # Examples
/// ```
/// use textamp::util::format_bytes;
/// assert_eq!(format_bytes(1024), "1.0 KB");
/// assert_eq!(format_bytes(1536000), "1.5 MB");
/// ```
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Truncate a string to a maximum character length, appending "..." if truncated.
///
/// This function counts Unicode scalar values (chars), not bytes,
/// so it works correctly with non-ASCII text.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len < 3 {
        // No room for an ellipsis without exceeding max_len.
        s.chars().take(max_len).collect()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    }
}

/// Clamp a string to at most `max_bytes` bytes, rounding the cut down to a
/// UTF-8 char boundary. Unlike `&s[..n]`, this never panics on multibyte
/// input. Intended for log snippets of server responses.
pub fn truncate_to_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Strip codepoints that have no visible glyph but cause renderers
/// to paint a "tofu" notdef shape — control chars, format chars
/// (zero-width joiners / directionality marks / language tags),
/// private-use codepoints, and the misc emoji-format selectors.
///
/// Plex artist / album / track titles occasionally carry these as
/// metadata leftovers (especially the format-category and
/// private-use ones); the system font has no glyph for them, so
/// they show up as mystery shapes — small boxes, stacks of
/// horizontal lines, etc. — that don't appear in Plex's web client
/// because the web stack is also stripping them.
///
/// Allocation-free fast path for clean ASCII / Latin titles via
/// `Cow::Borrowed`; only allocates when the input actually contains
/// a strippable codepoint.
pub fn sanitize_display_text(input: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let mut owned: Option<String> = None;
    for (idx, c) in input.char_indices() {
        if needs_strip(c) {
            let buf = owned.get_or_insert_with(|| {
                let mut b = String::with_capacity(input.len());
                b.push_str(&input[..idx]);
                b
            });
            let _ = buf; // strip-only: we don't push the char
            continue;
        }
        if let Some(buf) = owned.as_mut() {
            buf.push(c);
        }
    }
    match owned {
        Some(s) => Cow::Owned(s),
        None => Cow::Borrowed(input),
    }
}

fn needs_strip(c: char) -> bool {
    let cp = c as u32;
    // Variation selectors + ZWJ + tag chars.
    if matches!(cp, 0xFE00..=0xFE0F | 0x200D | 0xE0020..=0xE007F) {
        return true;
    }
    // C0 / C1 control characters (keep \t / \n / \r — Plex titles
    // sometimes use them and the renderer collapses to spaces).
    if (cp <= 0x1F && !matches!(cp, 0x09 | 0x0A | 0x0D)) || (0x7F..=0x9F).contains(&cp) {
        return true;
    }
    // Format-category (Cf) characters. Hand-rolled list to avoid
    // pulling in `unicode-properties` for one cold-path check.
    if matches!(cp,
        0x00AD                // SOFT HYPHEN
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x0890..=0x0891
        | 0x08E2
        | 0x180E
        | 0x200B..=0x200F     // ZWSP / ZWNJ / ZWJ / LRM / RLM
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x2069
        | 0x206A..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0x110BD
        | 0x110CD
        | 0x13430..=0x13438
        | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A
        | 0xE0001
    ) {
        return true;
    }
    // Private-Use Area.
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFD).contains(&cp)
        || (0x100000..=0x10FFFD).contains(&cp)
    {
        return true;
    }
    false
}

/// Normalize heart-like glyphs in a string to their text-presentation
/// form so terminals render them as monochrome glyphs instead of
/// colorful emoji.
///
/// Strips the U+FE0F (emoji-presentation) variation selector and
/// appends U+FE0E (text-presentation) after each heart glyph. Most
/// terminals honour the selector and switch to a monochrome font path,
/// which keeps the heart from "popping out" against the rest of the
/// muted text in the category column.
pub fn force_text_presentation(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c == '\u{fe0f}' { continue; }
        out.push(c);
        match c {
            '\u{2764}' | '\u{2665}' | '\u{2661}' | '\u{1f90d}' | '\u{1f5a4}' => {
                out.push('\u{fe0e}');
            }
            _ => {}
        }
    }
    out
}

/// Middle-truncate a string, preserving both the beginning and end.
///
/// Replaces the center characters with a single ellipsis (…) so that both
/// the start and end of the string remain visible.
///
/// # Examples
/// ```
/// use textamp::util::truncate_middle;
/// assert_eq!(truncate_middle("ExtremelyLongFileName_2024.png", 20), "ExtremelyL…_2024.png");
/// assert_eq!(truncate_middle("short", 20), "short");
/// ```
pub fn truncate_middle(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();

    if char_count <= max_len {
        return s.to_string();
    }

    // Need at least 3 characters for "x…x"
    if max_len < 3 {
        return s.chars().take(max_len).collect();
    }

    // We need 1 char for the ellipsis, leaving (max_len - 1) chars for content
    // Split roughly evenly, with start getting the extra char if odd
    let content_len = max_len - 1;
    let start_len = (content_len + 1) / 2;
    let end_len = content_len / 2;

    let start: String = s.chars().take(start_len).collect();
    let end: String = s.chars().skip(char_count - end_len).collect();

    format!("{}…{}", start, end)
}

/// Pad or truncate a string to exactly `width` display columns using unicode-width.
///
/// If the string is shorter, pads with spaces. If longer, truncates with "...".
pub fn pad_right(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;

    let display_width = UnicodeWidthStr::width(s);
    if display_width <= width {
        // Pad with spaces to fill remaining width
        let padding = width - display_width;
        format!("{}{}", s, " ".repeat(padding))
    } else {
        // Truncate: walk chars until we reach width - 3, then add "..."
        if width < 3 {
            return ".".repeat(width);
        }
        let target = width - 3;
        let mut current_width = 0;
        let mut end_byte = 0;
        for (i, ch) in s.char_indices() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > target {
                end_byte = i;
                break;
            }
            current_width += ch_width;
            end_byte = i + ch.len_utf8();
        }
        let truncated = &s[..end_byte];
        // Pad truncated part if char widths don't sum exactly to target
        let pad = target - UnicodeWidthStr::width(truncated);
        format!("{}{}...", truncated, " ".repeat(pad))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_unicode() {
        // Japanese characters (each is 3 bytes but 1 char)
        assert_eq!(truncate_str("こんにちは世界", 5), "こん...");
    }

    #[test]
    fn test_truncate_tiny_max_len() {
        // Must never exceed max_len, even when there's no room for "..."
        assert_eq!(truncate_str("hello", 2), "he");
        assert_eq!(truncate_str("hello", 0), "");
        assert_eq!(truncate_str("hello", 3), "...");
    }

    #[test]
    fn test_truncate_to_boundary() {
        assert_eq!(truncate_to_boundary("hello", 10), "hello");
        assert_eq!(truncate_to_boundary("hello", 3), "hel");
        // "é" is 2 bytes; cutting at byte 1 must back up to the boundary
        assert_eq!(truncate_to_boundary("été", 1), "");
        assert_eq!(truncate_to_boundary("été", 2), "é");
        assert_eq!(truncate_to_boundary("été", 3), "ét");
        // 3-byte chars
        assert_eq!(truncate_to_boundary("こんにちは", 7), "こん");
        assert_eq!(truncate_to_boundary("", 5), "");
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(1000), "0:01");
        assert_eq!(format_duration(59000), "0:59");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60000), "1:00");
        assert_eq!(format_duration(62000), "1:02");
        assert_eq!(format_duration(3599000), "59:59");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(3600000), "1:00:00");
        assert_eq!(format_duration(3661000), "1:01:01");
        assert_eq!(format_duration(7322000), "2:02:02");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_truncate_middle_short_string() {
        assert_eq!(truncate_middle("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_middle_exact_length() {
        assert_eq!(truncate_middle("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_middle_long_string() {
        // "ExtremelyLongFileName_2024.png" = 30 chars
        // At 20: start=10, end=9 → "ExtremelyL…_2024.png"
        assert_eq!(
            truncate_middle("ExtremelyLongFileName_2024.png", 20),
            "ExtremelyL…_2024.png"
        );
    }

    #[test]
    fn test_truncate_middle_preserves_ends() {
        // "abcdefghij" = 10 chars, truncate to 5
        // content_len = 4, start_len = 2, end_len = 2
        // "ab…ij"
        assert_eq!(truncate_middle("abcdefghij", 5), "ab…ij");
    }

    #[test]
    fn test_truncate_middle_unicode() {
        // Japanese: "こんにちは世界" = 7 chars
        // At 5: content_len = 4, start=2, end=2
        // "こん…世界"
        assert_eq!(truncate_middle("こんにちは世界", 5), "こん…世界");
    }

    #[test]
    fn test_truncate_middle_minimum() {
        assert_eq!(truncate_middle("abcdef", 3), "a…f");
        assert_eq!(truncate_middle("abcdef", 2), "ab");
        assert_eq!(truncate_middle("abcdef", 1), "a");
    }
}
