//! Cross-platform process lock.
//!
//! Used to guarantee that only one textamp instance (TUI or GUI) is running
//! against the shared on-disk caches / auth / config at a time. The lock file
//! lives in the platform state directory (e.g. `~/.local/state/textamp/textamp.lock`).
//!
//! The lock is an OS-level advisory exclusive lock acquired via `fs4`:
//! - Released automatically when the process exits (even on panic or kill).
//! - Works on Linux (fcntl), macOS (flock), and Windows (LockFileEx).
//!
//! Keep the returned `ProcessLock` alive for the lifetime of the process.

use crate::config::XdgPaths;
use fs4::fs_std::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use thiserror::Error;

/// Errors returned when attempting to acquire the process lock.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another textamp instance holds the lock.
    #[error("another textamp instance is already running (lock held at {0})")]
    AlreadyRunning(PathBuf),

    /// OS-level I/O error (permissions, missing directory, etc.).
    #[error("failed to open lock file at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Holds the exclusive lock for the running process.
///
/// Dropping this releases the lock. The OS also releases the lock when the
/// process exits, so a crash or SIGKILL does not leave the app "stuck".
#[must_use = "the lock is released as soon as this value is dropped"]
pub struct ProcessLock {
    _file: File,
    path: PathBuf,
}

impl ProcessLock {
    /// Try to acquire the exclusive lock at `<state_dir>/textamp.lock`.
    ///
    /// Returns `Err(LockError::AlreadyRunning)` if another instance holds it.
    pub fn acquire() -> Result<Self, LockError> {
        let state_dir = XdgPaths::new("textamp").state_dir.clone();
        if let Err(e) = std::fs::create_dir_all(&state_dir) {
            return Err(LockError::Io { path: state_dir, source: e });
        }
        let path = state_dir.join("textamp.lock");

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| LockError::Io { path: path.clone(), source })?;

        match file.try_lock_exclusive() {
            Ok(()) => {
                // Best-effort write the PID for humans inspecting the file.
                // Ignore errors: the lock succeeded and that's what matters.
                let _ = writeln!(file, "{}", std::process::id());
                Ok(Self { _file: file, path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Err(LockError::AlreadyRunning(path))
            }
            Err(source) => Err(LockError::Io { path, source }),
        }
    }

    /// Path to the lock file on disk (for logging / error messages).
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}
