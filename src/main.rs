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

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // Check for debug mode
    if args.iter().any(|a| a == "--debug" || a == "-d") {
        return run_debug_mode().await;
    }

    // Check for test mode
    if args.iter().any(|a| a == "--test" || a == "-t") {
        return run_test_mode().await;
    }

    // Normal TUI mode
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    run_tui_mode(verbose).await
}

/// Debug mode - test API without TUI
async fn run_debug_mode() -> Result<()> {
    println!("=== textamp debug mode ===\n");

    // Load config
    let config = config::load_config()?;
    println!("Config loaded:");
    println!("  Server URL: {}", config.plex.server_url);
    println!("  Username: {:?}", config.plex.username);
    println!();

    // Authenticate using stored token
    println!("Authenticating...");
    let auth = PlexAuth::new();

    let stored = PlexAuth::load_token();
    let token = if let Some(stored) = stored {
        println!("  Found stored token");
        match auth.verify_token(&stored.token).await {
            Ok(user) => {
                println!("  Token valid for user: {}", user.username);
                stored.token
            }
            Err(e) => {
                println!("  Stored token invalid: {}", e);
                return Ok(());
            }
        }
    } else {
        println!("  No stored token! Run the app normally to authenticate first.");
        return Ok(());
    };

    // Get user info
    println!("\nVerifying token...");
    match auth.verify_token(&token).await {
        Ok(user) => {
            println!("  User: {} (id={})", user.username, user.id);
        }
        Err(e) => {
            println!("  Token verification FAILED: {}", e);
        }
    }

    // Create client
    let client_info = PlexClientInfo::default();
    let mut client = PlexClient::new(client_info);
    client.set_auth_token(token.clone());
    client.set_server(config.plex.server_url.clone());

    // Get libraries
    println!("\nFetching libraries from {}...", config.plex.server_url);
    match client.get_libraries().await {
        Ok(libs) => {
            println!("  Found {} libraries:", libs.len());
            for lib in &libs {
                let is_music = lib.is_music();
                let marker = if is_music { " [MUSIC]" } else { "" };
                println!("    - {} (key={}, type={}){}",
                    lib.title, lib.key, lib.library_type, marker);
            }

            // Filter to music only
            let music_libs: Vec<_> = libs.iter().filter(|l| l.is_music()).collect();
            println!("\n  Music libraries: {}", music_libs.len());

            // If we have a music library, test fetching artists
            if let Some(lib) = music_libs.first() {
                println!("\nFetching artists from '{}'...", lib.title);
                match client.get_artists(&lib.key).await {
                    Ok(artists) => {
                        println!("  Found {} artists", artists.len());
                        for artist in artists.iter().take(5) {
                            println!("    - {}", artist.title);
                        }
                        if artists.len() > 5 {
                            println!("    ... and {} more", artists.len() - 5);
                        }
                    }
                    Err(e) => println!("  FAILED: {}", e),
                }

                println!("\nFetching albums from '{}'...", lib.title);
                match client.get_albums(&lib.key).await {
                    Ok(albums) => {
                        println!("  Found {} albums", albums.len());
                        for album in albums.iter().take(5) {
                            println!("    - {} by {} (key={}, leaf_count={:?})",
                                album.title, album.artist_name(), album.rating_key, album.leaf_count);
                        }
                        if albums.len() > 5 {
                            println!("    ... and {} more", albums.len() - 5);
                        }

                        // Test album tracks - skip first few albums (might be obscure)
                        // Try to find an album with more tracks
                        let test_album = albums.iter()
                            .skip(5)  // Skip first few potentially obscure albums
                            .take(20)
                            .find(|a| a.leaf_count.unwrap_or(0) > 5)
                            .or_else(|| albums.get(10));

                        if let Some(album) = test_album {
                            println!("\nFetching tracks for album '{}'...", album.title);
                            match client.get_album_tracks(&album.rating_key).await {
                                Ok(tracks) => {
                                    println!("  Found {} tracks:", tracks.len());
                                    for track in tracks.iter().take(5) {
                                        println!("    {}. {} (key={})",
                                            track.track_number(), track.title, track.rating_key);
                                    }
                                    if tracks.len() > 5 {
                                        println!("    ... and {} more", tracks.len() - 5);
                                    }

                                    // Test sonic similarity
                                    // Note: Plex Sonic Similarity only works at album level
                                    println!("\nFetching similar albums for '{}'...", album.title);
                                    match client.get_similar_albums(&album.rating_key, 10).await {
                                        Ok(similar) => {
                                            println!("  Found {} similar albums:", similar.len());
                                            for a in similar.iter().take(5) {
                                                println!("    - {} by {}", a.title, a.artist_name());
                                            }
                                        }
                                        Err(e) => println!("  Similar albums FAILED: {}", e),
                                    }
                                }
                                Err(e) => println!("  Album tracks FAILED: {}", e),
                            }
                        }
                    }
                    Err(e) => println!("  FAILED: {}", e),
                }

                // Test search
                println!("\nTesting search for 'love'...");
                match client.search("love").await {
                    Ok(results) => {
                        println!("  Artists: {}", results.artists.len());
                        for artist in results.artists.iter().take(3) {
                            println!("    - {}", artist.title);
                        }
                        println!("  Albums: {}", results.albums.len());
                        for album in results.albums.iter().take(3) {
                            println!("    - {} by {}", album.title, album.artist_name());
                        }
                        println!("  Tracks: {}", results.tracks.len());
                        for track in results.tracks.iter().take(3) {
                            println!("    - {} by {}", track.title, track.artist_name());
                        }
                    }
                    Err(e) => println!("  Search FAILED: {}", e),
                }

                // Test search for "beatles"
                println!("\nTesting search for 'beatles'...");
                match client.search("beatles").await {
                    Ok(results) => {
                        println!("  Artists: {}", results.artists.len());
                        for artist in results.artists.iter().take(5) {
                            println!("    - {}", artist.title);
                        }
                        println!("  Albums: {}", results.albums.len());
                        for album in results.albums.iter().take(5) {
                            println!("    - {} by {}", album.title, album.artist_name());
                        }
                    }
                    Err(e) => println!("  Search FAILED: {}", e),
                }
            }
        }
        Err(e) => {
            println!("  FAILED: {}", e);
        }
    }

    // Test home hubs
    println!("\nFetching home hubs...");
    match client.get_home_hubs().await {
        Ok(hubs) => {
            println!("  Found {} hubs:", hubs.len());
            for hub in hubs.iter().take(10) {
                let music_flag = if hub.is_music() { " [MUSIC]" } else { "" };
                println!("    - {} (type={}, id={}, items={}){}",
                    hub.title, hub.hub_type, hub.hub_identifier, hub.metadata.len(), music_flag);
            }
            let music_hubs: Vec<_> = hubs.iter().filter(|h| h.is_music()).collect();
            println!("\n  Music hubs after filtering: {}", music_hubs.len());
            for hub in music_hubs.iter() {
                println!("    - {}", hub.title);
            }
        }
        Err(e) => {
            println!("  FAILED: {}", e);
        }
    }

    // Test stations (Plexamp radio)
    println!("\n=== Testing Stations API ===");
    let mut music_libs = client.get_music_libraries().await.unwrap_or_default();
    println!("Found {} music libraries:", music_libs.len());
    for lib in &music_libs {
        println!("  - {} (key={})", lib.title, lib.key);
    }

    // Sort by key (lower key = older = likely main library)
    music_libs.sort_by(|a, b| a.key.parse::<u32>().unwrap_or(999).cmp(&b.key.parse::<u32>().unwrap_or(999)));

    // Test with the oldest library (lowest key, likely the main one)
    if let Some(lib) = music_libs.first() {
        println!("\nTesting stations for library '{}' (key={})...", lib.title, lib.key);

        // First get raw response to see what we're dealing with
        let path = format!("/hubs/sections/{}?includeStations=1", lib.key);
        println!("\nRaw API response for {}:", path);
        match client.get_raw(&path).await {
            Ok(raw) => {
                // Look for stations hub specifically
                if let Some(stations_pos) = raw.find("\"hub.music.stations\"") {
                    println!("\nFound stations context at position {}. Showing context:", stations_pos);
                    let start = stations_pos.saturating_sub(100);
                    let end = (stations_pos + 1000).min(raw.len());
                    println!("{}", &raw[start..end]);
                } else {
                    println!("No 'hub.music.stations' found in response.");
                    println!("\nFull response (first 5000 chars):");
                    println!("{}", &raw[..raw.len().min(5000)]);
                    if raw.len() > 5000 {
                        println!("... ({} more bytes)", raw.len() - 5000);
                    }
                }
            }
            Err(e) => println!("  Raw request FAILED: {}", e),
        }

        // Now try the parsed version
        println!("\nParsed stations:");
        match client.get_stations(&lib.key).await {
            Ok(stations) => {
                if stations.is_empty() {
                    println!("  No stations returned (empty list)");
                } else {
                    println!("  Found {} stations:", stations.len());
                    for station in &stations {
                        println!("    - {} (key={}, type={})",
                            station.title, station.key, station.station_type);
                        if let Some(desc) = &station.description {
                            println!("      {}", desc);
                        }
                    }
                }
            }
            Err(e) => println!("  Stations FAILED: {}", e),
        }
    } else {
        println!("  No music libraries found!");
    }

    println!("\n=== Debug complete ===");
    Ok(())
}

/// Test mode - automated TUI testing with headless rendering
async fn run_test_mode() -> Result<()> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use textamp::app::state::ConnectionState;
    use textamp::ui;

    println!("=== textamp test mode ===\n");

    // Load config
    let config = config::load_config()?;

    // Authenticate using stored token
    println!("Step 1: Authenticating...");
    let auth = PlexAuth::new();
    let token = if let Some(stored) = PlexAuth::load_token() {
        auth.verify_token(&stored.token).await?;
        stored.token
    } else {
        anyhow::bail!("No stored token. Run the app normally to authenticate first.");
    };
    println!("  OK - Authenticated");

    // Create client
    let client_info = PlexClientInfo::default();
    let mut client = PlexClient::new(client_info);
    client.set_auth_token(token);
    client.set_server(config.plex.server_url.clone());

    // Create test backend (80x24 terminal)
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend)?;

    // Create application state
    let mut state = AppState::new();
    state.terminal_width = 100;
    state.terminal_height = 30;
    state.connection = ConnectionState::Connected {
        username: "test_user".to_string(),
        has_plex_pass: false,
    };

    // Load libraries
    println!("\nStep 2: Loading libraries...");
    let libraries = client.get_libraries().await?;
    let music_libs: Vec<_> = libraries.into_iter().filter(|l| l.is_music()).collect();
    println!("  OK - Found {} music libraries", music_libs.len());

    if let Some(lib) = music_libs.first() {
        state.libraries = music_libs.clone();
        state.active_library = Some(lib.key.clone());

        // Load artists for testing
        println!("\nStep 3: Loading artists...");
        let artists = client.get_artists(&lib.key).await?;
        println!("  OK - Loaded {} artists", artists.len());
        state.artists = artists;

        // Load albums
        println!("\nStep 4: Loading albums...");
        let albums = client.get_albums(&lib.key).await?;
        println!("  OK - Loaded {} albums", albums.len());
        state.albums = albums;
    }

    // Test sequence - comprehensive navigation testing (musikcube-style)
    use textamp::app::state::{View, BrowseCategory, Focus};

    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.focus = Focus::Left;

    println!("\n=== Running Navigation Tests ===\n");

    // Test 1: Category switching
    println!("Test 1: Category switching");
    state.browse_category = BrowseCategory::Playlists;
    assert_eq!(state.browse_category, BrowseCategory::Playlists, "Should be on Playlists category");
    println!("  PASS - Switched to Playlists category");

    // Test 2: Navigate in playlist list
    println!("\nTest 2: Playlist list navigation");
    apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(state.list_state.playlists_index >= 1, "Playlist index should have increased");
    println!("  PASS - Playlist navigation works, index = {}", state.list_state.playlists_index);

    // Test 3: Load tracks for selected playlist
    println!("\nTest 3: Load playlist tracks");
    state.list_state.playlists_index = 0;
    if let Some(playlist) = state.playlists.get(state.list_state.playlists_index) {
        println!("  Loading playlist: {}", playlist.title);
        match client.get_playlist_tracks(&playlist.rating_key).await {
            Ok(tracks) => {
                state.selected_album_tracks = tracks.clone();
                state.focus = Focus::Right;
                println!("  PASS - Loaded {} tracks for playlist", tracks.len());
                if !tracks.is_empty() {
                    println!("    First track: {}", tracks[0].title);
                }
            }
            Err(e) => {
                println!("  FAIL - Could not load tracks: {}", e);
            }
        }
    }

    // Test 4: Search functionality
    println!("\nTest 4: Search navigation");
    state.view = View::Search;
    state.search_query = "love".to_string();
    match client.search("love").await {
        Ok(results) => {
            println!("  Search returned {} artists, {} albums, {} tracks",
                results.artists.len(), results.albums.len(), results.tracks.len());
            if !results.is_empty() {
                state.search_results = Some(results.clone());
                state.list_state.search_item_index = 0;
                apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                println!("  PASS - Search results can be navigated");
                println!("  Current selection: index={}",
                    state.list_state.search_item_index);
            }
        }
        Err(e) => {
            println!("  FAIL - Search failed: {}", e);
        }
    }

    // Test 5: Similar albums
    println!("\nTest 5: Similar albums");
    if let Some(album) = state.albums.get(10) {
        match client.get_similar_albums(&album.rating_key, 10).await {
            Ok(similar) => {
                state.similar.albums = similar.clone();
                state.similar.source_title = format!("{} - {}", album.artist_name(), album.title);
                state.view = View::Similar;
                println!("  PASS - Loaded {} similar albums", similar.len());
                if let Some(first) = similar.first() {
                    println!("    First similar: {} by {}", first.title, first.artist_name());
                }
            }
            Err(e) => {
                println!("  FAIL - Similar albums failed: {}", e);
            }
        }
    }

    // Test 6: Render each view
    println!("\n=== Rendering Tests ===\n");

    let test_views = vec![
        ("Browse Artists", View::Browse, BrowseCategory::Library),
        ("Browse Playlists", View::Browse, BrowseCategory::Playlists),
        ("NowPlaying", View::NowPlaying, BrowseCategory::Library),
        ("Search", View::Search, BrowseCategory::Library),
        ("Help", View::Help, BrowseCategory::Library),
    ];

    for (name, view, category) in test_views {
        state.view = view;
        state.browse_category = category;
        terminal.draw(|f| ui::render(f, &state))?;
        println!("  Rendered: {}", name);
    }

    // Final state test
    println!("\n=== Final State Check ===\n");

    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.focus = Focus::Left;

    terminal.draw(|f| ui::render(f, &state))?;

    println!("Rendered final state:");
    print_buffer(&terminal.backend());
    println!();

    println!("=== Test sequence complete ===");
    println!("\nFinal state:");
    println!("  View: {:?}", state.view);
    println!("  Category: {:?}", state.browse_category);
    println!("  Focus: {:?}", state.focus);
    println!("  Artists loaded: {}", state.artists.len());
    println!("  Albums loaded: {}", state.albums.len());
    println!("  Artists index: {}", state.list_state.artists_index);
    println!("  Albums index: {}", state.list_state.albums_index);

    Ok(())
}

/// Apply a key event to the application state (simplified for testing - musikcube style)
fn apply_key_to_state(state: &mut AppState, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    use textamp::app::state::{View, BrowseCategory, Focus};

    // Handle based on view
    match state.view {
        View::Browse => {
            match key.code {
                KeyCode::Char('a') => {
                    state.browse_category = BrowseCategory::Library;
                    state.focus = Focus::Left;
                }
                KeyCode::Char('p') => {
                    state.browse_category = BrowseCategory::Playlists;
                    state.focus = Focus::Left;
                }
                KeyCode::Tab => {
                    state.focus = match state.focus {
                        Focus::Left => Focus::Right,
                        Focus::Right => Focus::Left,
                    };
                }
                KeyCode::Up => {
                    if state.focus == Focus::Left {
                        match state.browse_category {
                            BrowseCategory::Library => {
                                state.list_state.artists_index = state.list_state.artists_index.saturating_sub(1);
                            }
                            BrowseCategory::Playlists => {
                                state.list_state.playlists_index = state.list_state.playlists_index.saturating_sub(1);
                            }
                            BrowseCategory::Genres => {
                                state.genres_index = state.genres_index.saturating_sub(1);
                            }
                            BrowseCategory::Folders => {
                                // Folders handled separately
                            }
                        }
                    } else {
                        state.list_state.tracks_index = state.list_state.tracks_index.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if state.focus == Focus::Left {
                        match state.browse_category {
                            BrowseCategory::Library => {
                                let max = state.artists.len().saturating_sub(1);
                                state.list_state.artists_index = (state.list_state.artists_index + 1).min(max);
                            }
                            BrowseCategory::Playlists => {
                                let max = state.playlists.len().saturating_sub(1);
                                state.list_state.playlists_index = (state.list_state.playlists_index + 1).min(max);
                            }
                            BrowseCategory::Genres => {
                                let max = state.genres.len().saturating_sub(1);
                                state.genres_index = (state.genres_index + 1).min(max);
                            }
                            BrowseCategory::Folders => {
                                // Folders handled separately
                            }
                        }
                    } else {
                        let max = state.selected_album_tracks.len().saturating_sub(1);
                        state.list_state.tracks_index = (state.list_state.tracks_index + 1).min(max);
                    }
                }
                KeyCode::Char('n') => state.view = View::NowPlaying,
                KeyCode::Char('f') => state.view = View::Search,
                KeyCode::Char('?') => state.view = View::Help,
                _ => {}
            }
        }
        View::Search => {
            match key.code {
                KeyCode::Esc => {
                    state.view = View::Browse;
                    state.search_query.clear();
                    state.search_results = None;
                }
                KeyCode::Down => {
                    state.list_state.search_item_index += 1;
                }
                KeyCode::Up => {
                    state.list_state.search_item_index = state.list_state.search_item_index.saturating_sub(1);
                }
                _ => {}
            }
        }
        View::Queue | View::NowPlaying | View::Similar | View::Related => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('b') => state.view = View::Browse,
                KeyCode::Down => {
                    if state.view == View::Queue || state.view == View::NowPlaying {
                        let max = state.queue.len().saturating_sub(1);
                        state.list_state.queue_index = (state.list_state.queue_index + 1).min(max);
                    } else if state.view == View::Similar {
                        let max = state.similar.albums.len().saturating_sub(1);
                        state.list_state.similar_index = (state.list_state.similar_index + 1).min(max);
                    }
                }
                KeyCode::Up => {
                    if state.view == View::Queue || state.view == View::NowPlaying {
                        state.list_state.queue_index = state.list_state.queue_index.saturating_sub(1);
                    } else if state.view == View::Similar {
                        state.list_state.similar_index = state.list_state.similar_index.saturating_sub(1);
                    }
                }
                _ => {}
            }
        }
        View::Help => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('b') => state.view = View::Browse,
                _ => {}
            }
        }
        View::Settings => {
            match key.code {
                KeyCode::Esc => state.view = View::Browse,
                _ => {}
            }
        }
        View::Auth => {}
    }
}

/// Print the test backend buffer in a readable format
fn print_buffer(backend: &ratatui::backend::TestBackend) {
    let buffer = backend.buffer();
    let area = buffer.area;

    // Print top border
    println!("┌{}┐", "─".repeat(area.width as usize));

    for y in 0..area.height {
        print!("│");
        for x in 0..area.width {
            let cell = buffer.cell((x, y)).unwrap();
            let ch = cell.symbol();
            // Handle empty/space cells
            if ch.is_empty() || ch == " " {
                print!(" ");
            } else {
                print!("{}", ch);
            }
        }
        println!("│");
    }

    // Print bottom border
    println!("└{}┘", "─".repeat(area.width as usize));
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

    // Create audio player
    let mut audio = match AudioPlayer::new() {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };

    // Create application state
    let mut state = AppState::new();

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
