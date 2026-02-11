//! Logging setup with daily rotation and automatic pruning.

use crate::config::XdgPaths;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Maximum number of log files (normal + verbose combined).
const MAX_LOG_FILES: usize = 10;

/// Setup logging to file with daily rotation.
///
/// When `verbose` is false, logs only errors and prunes old log files on startup.
/// When `verbose` is true, logs at info level and skips pruning.
/// The `RUST_LOG` environment variable overrides the log level in either mode.
pub fn setup_logging(verbose: bool) -> Option<WorkerGuard> {
    let paths = XdgPaths::new("textamp");
    let _ = paths.ensure_dirs();

    let log_dir = paths.log_dir();

    // Prune old logs before setting up new appender (only in normal mode)
    if !verbose {
        prune_logs(&log_dir);
    }

    let prefix = if verbose { "textamp-verbose.log" } else { "textamp.log" };
    let file_appender = tracing_appender::rolling::daily(&log_dir, prefix);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let level = if verbose { "info" } else { "error" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    Some(guard)
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
            let modified = path.metadata()
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
