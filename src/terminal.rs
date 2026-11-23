//! textamp — keyboard-driven music player (terminal front-end).
//!
//! A process lock in the platform state directory prevents multiple instances
//! from writing the same caches.

use crate::config;
use anyhow::Result;

use crate::tui::EventLoop;
use crate::util::{
    LockError, ProcessLock, install_panic_hook, restore_terminal, setup_logging, setup_terminal,
};

/// Run the existing TUI in the caller's terminal without initializing native windows.
/// # Errors
/// Propagates locking, configuration, terminal, runtime and shutdown failures.
pub fn run(arguments: &crate::app::launch::Arguments) -> Result<()> {
    use std::io::IsTerminal;
    arguments.validate_terminal(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )?;
    let verbose = arguments.verbose;

    // Acquire the cross-platform process lock before doing anything else.
    // If another textamp (TUI or GUI) is running, bail out with a clear
    // message rather than racing it for cache files.
    let _lock = match ProcessLock::acquire() {
        Ok(lock) => lock,
        Err(LockError::AlreadyRunning(path)) => {
            eprintln!(
                "textamp is already running (lock held at {}).\n\
                 Quit the other instance before starting a new one.",
                path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to acquire process lock: {e}");
            std::process::exit(1);
        }
    };

    let _logging = setup_logging(verbose);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_tui_mode(arguments.no_audio));
    // Blocking/native calls cannot be forcibly cancelled safely.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    result
}

/// Normal TUI mode
async fn run_tui_mode(no_audio: bool) -> Result<()> {
    let signals = crate::tui::ShutdownSignal::new()?;
    // Load configuration
    let config = config::load_config()?;

    tracing::info!("Starting textamp v{}", env!("CARGO_PKG_VERSION"));

    // Complete filesystem/client/audio initialization before entering raw
    // mode. A slow credential volume or a wedged CoreAudio device must not
    // leave the user staring at an unresponsive alternate screen.
    let session = crate::app::runtime::Session::open(
        config,
        if no_audio {
            crate::app::runtime::AudioMode::Silent
        } else {
            crate::app::runtime::AudioMode::Native
        },
    )
    .await?;
    // Restore the terminal if anything panics during or after raw-mode setup,
    // so the panic message is visible and the shell stays usable.
    install_panic_hook();

    // Setup terminal
    let mut terminal = setup_terminal()?;

    // Detect terminal graphics capabilities (must happen before event reader starts)
    // Apple Terminal can't render Sixel/Kitty protocols and from_query_stdio() echoes artifacts.
    let is_apple_terminal = std::env::var("TERM_PROGRAM")
        .map(|v| v == "Apple_Terminal")
        .unwrap_or(false)
        || std::env::var("TERM_SESSION_ID")
            .map(|v| v.contains("com.apple.Terminal"))
            .unwrap_or(false);

    let effective_mode = session.state.artwork.mode;

    let picker_result = if is_apple_terminal {
        tracing::info!("Apple Terminal detected, using halfblocks protocol for fallback");
        Ok(ratatui_image::picker::Picker::halfblocks())
    } else {
        ratatui_image::picker::Picker::from_query_stdio()
    };
    let picker = picker_result.unwrap_or_else(|error| {
        tracing::warn!(%error, "Image protocol detection failed; using half blocks");
        ratatui_image::picker::Picker::halfblocks()
    });
    {
        // Init renderers BEFORE overriding protocol so native type is stored
        tracing::info!(
            "Native protocol: {:?}, Artwork mode: {:?}",
            picker.protocol_type(),
            effective_mode
        );
        crate::ui::artwork::init_grid_renderer(picker.clone());
        crate::ui::screens::now_playing::init_artwork_renderer(picker.clone());
        crate::ui::init_bio_artwork_renderer(picker.clone());

        // Apply halfblocks if artwork mode requires it
        if effective_mode == crate::app::state::ArtworkMode::Halfblocks {
            tracing::info!("Halfblocks artwork mode, overriding to halfblocks protocol");
            let hb = ratatui_image::picker::ProtocolType::Halfblocks;
            crate::ui::artwork::set_grid_protocol_type(hb);
            crate::ui::screens::now_playing::set_artwork_protocol_type(hb);
            crate::ui::set_bio_artwork_protocol_type(hb);
        }
        crate::ui::artwork::set_grid_artwork_mode(effective_mode);
        crate::ui::screens::now_playing::set_artwork_mode(effective_mode);
        crate::ui::set_bio_artwork_mode(effective_mode);
    }

    // Run the app and ensure terminal is always restored
    let result = EventLoop::new(session, signals).run(&mut terminal).await;

    // Always restore terminal, even on error
    if let Err(error) = restore_terminal(&mut terminal) {
        eprintln!("Failed to fully restore terminal state: {error}");
    }

    // Display exit logo (clear screen, show ANSI art)
    display_exit_logo();

    tracing::info!("textamp shutdown complete");
    // Preserve an event-loop failure as the process exit status.
    result
}

/// Display the application logo after restoring the invoking terminal.
fn display_exit_logo() {
    use std::io::{self, Write};
    let mut output = io::stdout().lock();
    let _ = output.write_all(b"\x1b[2J\x1b[H");
    let _ = output.write_all(include_bytes!("../assets/artwork/textamp.ansi"));
    let _ = writeln!(output, "\x1b[0mTextamp {}", env!("CARGO_PKG_VERSION"));
    let _ = output.flush();
}
