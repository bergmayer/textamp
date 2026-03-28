//! textamp - A keyboard-driven TUI client for Plex Music.
//!
//! A high-performance terminal user interface for Plex Music,
//! inspired by Plexamp but designed for keyboard-driven workflows.

use anyhow::Result;
use std::env;
use textamp::plex::{PlexAuth, PlexClient, PlexClientInfo};
use textamp::app::{AppState, EventLoop};
use textamp::audio::AudioPlayer;
use textamp::config::{self, Config};
use textamp::util::{restore_terminal, setup_logging, setup_terminal};

// Debug mode: `cargo run --bin debug`
// Test mode: `cargo run --bin test_tui`

#[tokio::main]
async fn main() -> Result<()> {
    let verbose = env::args().any(|a| a == "--verbose" || a == "-v");
    run_tui_mode(verbose).await
}

/// Normal TUI mode
async fn run_tui_mode(verbose: bool) -> Result<()> {
    // Load configuration
    let config = config::load_config()?;

    // Setup logging
    let _guard = setup_logging(verbose);

    tracing::info!("Starting textamp v{}", env!("CARGO_PKG_VERSION"));

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
    let effective_mode = if configured_mode == textamp::app::state::ArtworkMode::Auto && is_apple_terminal {
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
        tracing::info!("Native protocol: {:?}, Artwork mode: {:?}", picker.protocol_type(), effective_mode);
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
    let (result, pending_cache) = run_app(&mut terminal, config).await;

    // Always restore terminal, even on error
    let _ = restore_terminal(&mut terminal);

    // Display exit logo (clear screen, show ANSI art)
    display_exit_logo();

    // Save cache to disk on a background thread (runs while exit logo is displayed)
    let cache_thread = pending_cache.map(|cache_data| {
        std::thread::spawn(move || {
            if let Some(cache) = textamp::plex::LibraryCache::new() {
                if cache.save(&cache_data) {
                    tracing::info!("Cache saved on quit");
                }
            }
        })
    });

    // Now handle any errors
    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    // Wait for cache write to finish before exiting
    if let Some(handle) = cache_thread {
        let _ = handle.join();
    }

    tracing::info!("textamp shutdown complete");
    Ok(())
}

/// Display the ANSI art logo on exit (Cubic Player style).
/// Clears the screen and prints the embedded ANSI logo with URLs and farewell message.
fn display_exit_logo() {
    use std::io::{self, Write};

    // Embedded ANSI art logo
    static LOGO_ANSI: &[u8] = include_bytes!("../textamp.ansi");

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
    println!(" {BRIGHT_CYAN}http://bergmayer.net/textamp{RESET}     {DARK_GRAY}Why be bleak{RESET}");
    println!("      {DIM_CYAN}https://app.plex.tv/{RESET}      {DIM_GRAY}|{RESET}     when you can be Blake?");
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
) -> (Result<()>, Option<textamp::plex::CacheData>) {
    // Create Plex client with stored client_identifier if available
    // IMPORTANT: The client_identifier must match what the auth token was issued for,
    // otherwise Plex will reject requests with 400 errors
    let client_info = if let Some(stored) = PlexAuth::load_token() {
        tracing::info!("Loaded stored client_identifier: {}", stored.client_identifier);
        let mut info = PlexClientInfo::default();
        info.client_identifier = stored.client_identifier;
        info
    } else {
        tracing::info!("No stored auth, using new client_identifier");
        PlexClientInfo::default()
    };
    let mut client = PlexClient::new(client_info);
    tracing::info!("PlexClient created with client_identifier: {}", client.client_identifier());

    // Create audio player (with timeout — WSL2 PulseAudio can hang on fresh reboot)
    // Uses block_in_place instead of spawn_blocking because on macOS the CoreAudio
    // stream type is !Send and cannot be returned across thread boundaries.
    let mut audio = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async { tokio::task::block_in_place(AudioPlayer::new) },
    ).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            tracing::warn!("Audio device unavailable: {} — launching without playback", e);
            AudioPlayer::new_without_audio().unwrap()
        }
        _ => {
            tracing::warn!("Audio initialization timed out — launching without playback");
            AudioPlayer::new_without_audio().unwrap()
        }
    };

    // Create application state
    let mut state = AppState::new();
    state.audio_available = audio.has_audio();

    // Get terminal size
    let size = match terminal.size() {
        Ok(s) => s,
        Err(e) => return (Err(e.into()), None),
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
    let result = event_loop.run(terminal, &mut state, &mut client, &mut audio).await;

    // Extract pending cache save (built during Action::Quit, deferred for fast exit)
    let pending_cache = state.pending_cache_save.take();
    (result, pending_cache)
}
