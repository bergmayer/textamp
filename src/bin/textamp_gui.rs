//! textamp-gui — keyboard-driven Plex Music client (desktop front-end).
//!
//! Sibling to the `textamp` TUI binary. Both share the same core (plex
//! client, cache, services, audio, action dispatch) and differ only in the
//! rendering/input layer. A process lock in the platform state directory
//! prevents both binaries from running at the same time.
//!
//! On Windows, release builds link as the `windows` subsystem so launching
//! from Explorer does not flash a console window. Debug builds stay on the
//! console subsystem so panics print to stderr during development.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::Result;
use textamp::util::{setup_logging, LockError, ProcessLock};

fn main() -> Result<()> {
    // Process lock first — if another instance owns the cache, bail out.
    let _lock = match ProcessLock::acquire() {
        Ok(lock) => lock,
        Err(LockError::AlreadyRunning(path)) => {
            // TODO(step 3): show a native error dialog instead of stderr.
            eprintln!(
                "textamp is already running (lock held at {}).\n\
                 Quit the other instance before starting the GUI.",
                path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to acquire process lock: {e}");
            std::process::exit(1);
        }
    };

    let verbose = std::env::args().any(|a| a == "--verbose" || a == "-v");
    let _guard = setup_logging(verbose);
    tracing::info!("Starting textamp-gui v{}", env!("CARGO_PKG_VERSION"));

    // Some desktop envs (notably WSLg) ship oversized cursor themes that
    // swamp small UI controls. Only set a default when the user has not
    // expressed a preference — custom Xcursor setups stay untouched.
    #[cfg(unix)]
    if std::env::var_os("XCURSOR_SIZE").is_none() {
        std::env::set_var("XCURSOR_SIZE", "24");
    }

    // Try to repair common Linux audio environments (mostly WSLg, where
    // the PULSE_SERVER env var often points at a socket that stopped
    // accepting connections after a suspend/sleep cycle). This is a
    // best-effort fixup: if the socket behind PULSE_SERVER isn't live we
    // try candidate locations in order of reliability, and ultimately
    // unset the variable so PulseAudio clients fall back to autospawn.
    #[cfg(unix)]
    recover_audio_env();

    // The Iced application owns the tokio runtime; everything async inside
    // the app runs on Iced's executor.
    textamp::ui_gui::run()
}

/// Best-effort repair of the PulseAudio client env so rodio/cpal can open
/// a working output stream. If the server pointed at by `PULSE_SERVER`
/// isn't accepting connections we unset the variable, letting libpulse
/// fall back to autospawn (or fail fast, which surfaces our in-app audio
/// error banner).
///
/// We probe by opening the unix socket with a short deadline rather than
/// checking for a local daemon process. On WSLg the PulseAudio daemon
/// runs in a separate distro and is not visible in this distro's process
/// table, but the socket at `/mnt/wslg/PulseServer` is live — a process
/// check would spuriously conclude the bridge is down and clobber a
/// working config. A real connection attempt is definitive: stale socket
/// files return `ECONNREFUSED` immediately, and live servers accept in
/// microseconds. The 300 ms cap handles the pathological case of a
/// listening-but-hung server.
#[cfg(unix)]
fn recover_audio_env() {
    use std::time::Duration;

    match std::env::var("PULSE_SERVER") {
        Ok(server) => {
            if pulse_server_reachable(&server, Duration::from_millis(300)) {
                tracing::info!("Audio: PULSE_SERVER reachable at {server} (unchanged).");
                return;
            }
            tracing::warn!(
                "Audio: PULSE_SERVER ({server}) is not accepting connections; \
                 clearing it so clients can autospawn. On WSL this usually \
                 means WSLg's audio bridge is down — run `wsl --shutdown` \
                 from Windows and reopen WSL to restart it."
            );
            std::env::remove_var("PULSE_SERVER");
        }
        Err(_) => {
            tracing::info!("Audio: PULSE_SERVER not set; relying on default PulseAudio discovery.");
        }
    }
}

/// Probe whether the address in `PULSE_SERVER` is a live unix socket.
///
/// Accepts the `unix:` prefix used by PulseAudio and bare `/path`. For
/// non-unix server strings (e.g. `tcp:host:port`) we don't probe and
/// return `true` so the existing env is left alone — rodio will surface
/// its own error if the server is unreachable.
///
/// The connect runs on a detached helper thread so a hung server cannot
/// block startup past `deadline`.
#[cfg(unix)]
fn pulse_server_reachable(server: &str, deadline: std::time::Duration) -> bool {
    let path = server.strip_prefix("unix:").unwrap_or(server);
    if !path.starts_with('/') {
        // Not a unix socket — assume configured tcp endpoint is intentional.
        return true;
    }

    let path = std::path::PathBuf::from(path);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ok = std::os::unix::net::UnixStream::connect(&path).is_ok();
        let _ = tx.send(ok);
    });
    rx.recv_timeout(deadline).unwrap_or(false)
}
