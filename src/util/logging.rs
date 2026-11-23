//! Logging setup with daily rotation and automatic pruning.

use crate::config::XdgPaths;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, Layer, prelude::*};

const ACTIVITY: &str = "textamp::activity";
const ACTIVITY_PREFIX: &str = "activity.log";
const RETENTION: time::Duration = time::Duration::days(6);

/// User-visible events have a separate filter from technical diagnostics.
pub fn activity_error(message: &str) {
    tracing::info!(target: "textamp::activity", "Error · {}", message.escape_debug());
}

pub fn activity_history_text() -> String {
    match read_activity(
        &XdgPaths::new("textamp").log_dir().join("activity"),
        time::OffsetDateTime::now_utc(),
    ) {
        Ok(text) if text.is_empty() => "No recent activity".into(),
        Ok(text) => text,
        Err(error) => format!("Could not read activity log: {error}"),
    }
}

fn retained_paths(
    dir: &Path,
    now: time::OffsetDateTime,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    let cutoff = now - RETENTION;
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(day) = name.to_str().and_then(|n| n.strip_prefix("activity.log.")) else {
            continue;
        };
        let midnight = format!("{day}T00:00:00Z");
        let Ok(date) =
            time::OffsetDateTime::parse(&midnight, &time::format_description::well_known::Rfc3339)
        else {
            continue;
        };
        if date.date() < cutoff.date() {
            std::fs::remove_file(entry.path())?;
        } else {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_activity(dir: &Path, now: time::OffsetDateTime) -> std::io::Result<String> {
    let cutoff = now - RETENTION;
    let mut entries = Vec::new();
    for path in retained_paths(dir, now)? {
        for line in std::fs::read_to_string(path)?.lines() {
            if let Some((timestamp, _)) = line.split_once(' ')
                && time::OffsetDateTime::parse(
                    timestamp,
                    &time::format_description::well_known::Rfc3339,
                )
                .is_ok_and(|time| time >= cutoff)
            {
                entries.push(line.to_owned());
            }
        }
    }
    entries.reverse();
    Ok(entries.join("\n"))
}

/// Maximum number of log files (normal + verbose combined).
const MAX_LOG_FILES: usize = 10;

/// Setup logging to file with daily rotation.
///
/// When `verbose` is false, logs errors plus audio/network warnings and prunes old log files on startup.
/// When `verbose` is true, logs at info level and skips pruning.
/// The `RUST_LOG` environment variable overrides the log level in either mode.
pub fn setup_logging(verbose: bool) -> Option<(WorkerGuard, WorkerGuard)> {
    let paths = XdgPaths::new("textamp");
    let _ = paths.ensure_dirs();

    let log_dir = paths.log_dir();

    // Prune old logs before setting up new appender (only in normal mode)
    if !verbose {
        prune_logs(&log_dir);
    }

    let prefix = if verbose {
        "textamp-verbose.log"
    } else {
        "textamp.log"
    };
    let file_appender = tracing_appender::rolling::daily(&log_dir, prefix);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let level = if verbose {
        "info"
    } else {
        "error,textamp::audio=warn,textamp::navidrome=warn,textamp::app::handlers::events=warn,textamp::app::handlers::dispatch_playback=warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let activity_dir = log_dir.join("activity");
    std::fs::create_dir_all(&activity_dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&activity_dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    let _ = retained_paths(&activity_dir, time::OffsetDateTime::now_utc());
    let activity = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(ACTIVITY_PREFIX)
        .max_log_files(7)
        .build(&activity_dir)
        .ok()?;
    let (activity, activity_guard) = tracing_appender::non_blocking(activity);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_filter(filter),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(activity)
                .with_ansi(false)
                .with_level(false)
                .with_target(false)
                .with_filter(tracing_subscriber::filter::filter_fn(|meta| {
                    meta.target() == ACTIVITY
                })),
        )
        .try_init()
        .ok()?;
    Some((guard, activity_guard))
}

/// Prune old log files by count.
///
/// Gathers all log files (normal, verbose, and legacy `textamp.log`),
/// then deletes the oldest if total count exceeds MAX_LOG_FILES.
fn prune_logs(log_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };

    let mut all_logs: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        let is_log = name == "textamp.log"
            || name.starts_with("textamp.log.")
            || name.starts_with("textamp-verbose.log.");

        if is_log {
            let modified = path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            all_logs.push((path, modified));
        }
    }

    // Sort oldest first
    all_logs.sort_by_key(|(_, modified)| *modified);

    // Delete oldest files if over the count cap
    while all_logs.len() > MAX_LOG_FILES {
        let (path, _) = all_logs.remove(0);
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_history_keeps_six_days_and_preserves_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let now = time::OffsetDateTime::parse(
            "2026-11-21T12:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        for (name, text) in [
            ("activity.log.2026-11-14", "old"),
            (
                "activity.log.2026-11-15",
                "2026-11-15T11:59:59Z old\n2026-11-15T12:00:00Z first\n",
            ),
            ("activity.log.2026-11-21", "2026-11-21T10:00:00Z recent\n"),
            ("unrelated", "preserve"),
        ] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        assert_eq!(
            read_activity(dir.path(), now).unwrap(),
            "2026-11-21T10:00:00Z recent\n2026-11-15T12:00:00Z first"
        );
        assert!(!dir.path().join("activity.log.2026-11-14").exists());
        assert!(dir.path().join("unrelated").exists());
    }
}
