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
/// This function counts Unicode grapheme clusters (characters), not bytes,
/// so it works correctly with non-ASCII text.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
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
