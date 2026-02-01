//! Logging setup.

use crate::config::XdgPaths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Setup logging to file.
pub fn setup_logging(level: &str) -> Option<WorkerGuard> {
    let paths = XdgPaths::new("textamp");
    let _ = paths.ensure_dirs();

    let log_file = paths.log_file();
    let log_dir = log_file.parent()?;
    let file_name = log_file.file_name()?.to_str()?;

    let file_appender = tracing_appender::rolling::never(log_dir, file_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    Some(guard)
}
