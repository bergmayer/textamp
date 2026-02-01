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
}
