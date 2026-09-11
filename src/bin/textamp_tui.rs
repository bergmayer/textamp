//! textamp — keyboard-driven music player (terminal front-end).
//!
//! A process lock in the platform state directory prevents multiple instances
//! from writing the same caches.

use anyhow::Result;
use std::env;
use textamp::app::AppState;
use textamp::audio::AudioPlayer;
use textamp::config::{self, Config};

use textamp::tui::EventLoop;
use textamp::util::{
    install_panic_hook, restore_terminal, setup_logging, setup_terminal, LockError, ProcessLock,
};

fn main() -> Result<()> {
    let verbose = env::args().any(|a| a == "--verbose" || a == "-v");

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
    let result = runtime.block_on(async {
        let result = run_tui_mode().await;
        let drained = textamp::app::tasks::finish_blocking(std::time::Duration::from_secs(3)).await;
        if !drained {
            eprintln!(
                "Background filesystem/CPU work did not finish; recent changes may not be saved."
            );
            tracing::error!("Background work exceeded the shutdown deadline");
        }
        result.and(if drained {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Background work timed out at shutdown"))
        })
    });
    // Blocking/native calls cannot be forcibly cancelled safely.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    result
}

/// Normal TUI mode
async fn run_tui_mode() -> Result<()> {
    // Load configuration
    let config = config::load_config()?;

    tracing::info!("Starting textamp v{}", env!("CARGO_PKG_VERSION"));

    // Complete filesystem/client/audio initialization before entering raw
    // mode. A slow credential volume or a wedged CoreAudio device must not
    // leave the user staring at an unresponsive alternate screen.
    let mut audio = match tokio::task::spawn_blocking(AudioPlayer::new).await {
        Ok(Ok(audio)) => audio,
        Ok(Err(error)) => {
            tracing::warn!(
                "Audio device unavailable: {} — launching without playback",
                error
            );
            AudioPlayer::new_without_audio()
        }
        Err(error) => {
            tracing::warn!(
                "Audio initialization worker failed: {} — launching without playback",
                error
            );
            AudioPlayer::new_without_audio()
        }
    };

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

    // Resolve artwork mode from config, with Apple Terminal defaulting to Braille
    let configured_mode = textamp::app::state::ArtworkMode::from_config(&config.ui.artwork_mode);
    let effective_mode =
        if configured_mode == textamp::app::state::ArtworkMode::Auto && is_apple_terminal {
            tracing::info!("Apple Terminal detected, defaulting to Braille artwork mode");
            textamp::app::state::ArtworkMode::Braille
        } else {
            configured_mode
        };

    let picker_result = if is_apple_terminal {
        tracing::info!("Apple Terminal detected, using halfblocks protocol for fallback");
        Ok(ratatui_image::picker::Picker::halfblocks())
    } else {
        ratatui_image::picker::Picker::from_query_stdio()
    };
    if let Ok(picker) = picker_result {
        // Init renderers BEFORE overriding protocol so native type is stored
        tracing::info!(
            "Native protocol: {:?}, Artwork mode: {:?}",
            picker.protocol_type(),
            effective_mode
        );
        textamp::ui::artwork::init_grid_renderer(picker.clone());
        textamp::ui::screens::now_playing::init_artwork_renderer(picker.clone());
        textamp::ui::init_bio_artwork_renderer(picker.clone());

        // Apply halfblocks if artwork mode requires it
        if effective_mode == textamp::app::state::ArtworkMode::Halfblocks {
            tracing::info!("Halfblocks artwork mode, overriding to halfblocks protocol");
            let hb = ratatui_image::picker::ProtocolType::Halfblocks;
            textamp::ui::artwork::set_grid_protocol_type(hb);
            textamp::ui::screens::now_playing::set_artwork_protocol_type(hb);
            textamp::ui::set_bio_artwork_protocol_type(hb);
        }
        textamp::ui::artwork::set_grid_artwork_mode(effective_mode);
        textamp::ui::screens::now_playing::set_artwork_mode(effective_mode);
        textamp::ui::set_bio_artwork_mode(effective_mode);
    }

    // Run the app and ensure terminal is always restored
    let result = run_app(&mut terminal, config, &mut audio).await;
    audio.stop();

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

/// Display the ANSI art logo on exit (Cubic Player style).
/// Clears the screen and prints the embedded ANSI logo with URLs and farewell message.
fn display_exit_logo() {
    use std::io::{self, Write};

    // Embedded ANSI art logo
    static LOGO_ANSI: &[u8] = include_bytes!("../../textamp.ansi");

    // ANSI color codes (Cubic Player style)
    const BRIGHT_CYAN: &str = "\x1b[38;2;0;187;187m";
    const DIM_CYAN: &str = "\x1b[38;2;0;135;135m";
    const DARK_GRAY: &str = "\x1b[38;2;85;85;85m";
    const DIM_GRAY: &str = "\x1b[38;2;68;68;68m";
    const PURPLE: &str = "\x1b[38;2;200;170;255m";
    const RESET: &str = "\x1b[0m";

    // Clear screen and move cursor to top
    print!("\x1b[2J\x1b[H");
    let _ = io::stdout().flush();

    // Print the ANSI logo directly
    let _ = io::stdout().write_all(LOGO_ANSI);
    let _ = io::stdout().flush();

    // Horizontal separator line with player/version label (Cubic Player style)
    // Line of ─ runs from left edge, interrupted by .- P L A Y E R -.- v1.0.0 -.
    // Total width matches ANSI art (~72 cols)
    let version = env!("CARGO_PKG_VERSION");
    let suffix = format!(" -.- v{version} -.");
    let label_width = 3 + 11 + suffix.len(); // ".- " + "P L A Y E R" + suffix
    let line = "\u{2500}".repeat(72usize.saturating_sub(label_width));
    println!("{DIM_GRAY}{line}.- {PURPLE}P L A Y E R{DIM_GRAY}{suffix}{RESET}");

    // Two-column layout within 72 cols, divider at ~col 33
    println!(
        " {BRIGHT_CYAN}http://bergmayer.net/textamp{RESET}     {DARK_GRAY}Why be bleak{RESET}"
    );
    println!("      {DIM_CYAN}music, locally.    {RESET}      {DIM_GRAY}|{RESET}     when you can be Blake?");
    println!("                                {DIM_GRAY}. {DARK_GRAY}Jhon Balance{RESET}                         {DIM_GRAY}.{RESET}");

    // Bottom corners (two-box Cubic Player style, 72 cols)
    println!("{DIM_GRAY}\u{2514}.                              .\u{2518}.                                   .\u{2518}{RESET}");

    // Farewell message (no color - default terminal text)
    println!("have a nice day...");
    println!();
}

/// Inner app runner - separated so terminal restoration always happens.
/// Returns the event loop result and any pending cache data to save after terminal restore.
async fn run_app(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    config: Config,

    audio: &mut AudioPlayer,
) -> Result<()> {
    // Create application state
    let mut state = AppState::new();
    state.audio_available = audio.has_audio();
    // Wire the audio backend's sample tap so the vectorscope
    // visualizer can drain live (L, R) sample pairs each tick.
    state.vectorscope_tap = audio.sample_tap();

    // Get terminal size
    let size = match terminal.size() {
        Ok(s) => s,
        Err(e) => return Err(e.into()),
    };
    state.terminal_width = size.width;
    state.terminal_height = size.height;

    // Set initial volume from config
    state.playback.volume = config.playback.default_volume;
    audio.set_volume(config.playback.default_volume);

    // Set transcoding preference from config
    state.transcode_kbps = config.playback.transcode_kbps;

    // Run event loop
    let mut event_loop = EventLoop::new(config);
    let result = event_loop.run(terminal, &mut state, audio).await;

    result
}
