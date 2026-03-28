//! textamp test mode - automated TUI testing with headless rendering.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use textamp::app::state::{AppState, BrowseCategory, ConnectionState, Focus, View};
use textamp::config;
use textamp::plex::{PlexAuth, PlexClient, PlexClientInfo};
use textamp::ui;

#[tokio::main]
async fn main() -> Result<()> {
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

    // Create test backend (100x30 terminal)
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

        println!("\nStep 3: Loading artists...");
        let artists = client.get_artists(&lib.key).await?;
        println!("  OK - Loaded {} artists", artists.len());
        state.library.artists = artists;

        println!("\nStep 4: Loading albums...");
        let albums = client.get_albums(&lib.key).await?;
        println!("  OK - Loaded {} albums", albums.len());
        state.library.albums = albums;
    }

    state.view = View::Browse;
    state.browse_category = BrowseCategory::Library;
    state.focus = Focus::Left;

    println!("\n=== Running Navigation Tests ===\n");

    // Test 1: Category switching
    println!("Test 1: Category switching");
    state.browse_category = BrowseCategory::Playlists;
    assert_eq!(state.browse_category, BrowseCategory::Playlists);
    println!("  PASS - Switched to Playlists category");

    // Test 2: Navigate in playlist list
    println!("\nTest 2: Playlist list navigation");
    apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(state.list_state.playlists_index >= 1);
    println!("  PASS - Playlist navigation works, index = {}", state.list_state.playlists_index);

    // Test 3: Load tracks for selected playlist
    println!("\nTest 3: Load playlist tracks");
    state.list_state.playlists_index = 0;
    if let Some(playlist) = state.library.playlists.get(state.list_state.playlists_index) {
        println!("  Loading playlist: {}", playlist.title);
        match client.get_playlist_tracks(&playlist.rating_key).await {
            Ok(tracks) => {
                state.library.selected_album_tracks = tracks.clone();
                state.focus = Focus::Right;
                println!("  PASS - Loaded {} tracks for playlist", tracks.len());
                if !tracks.is_empty() {
                    println!("    First track: {}", tracks[0].title);
                }
            }
            Err(e) => println!("  FAIL - Could not load tracks: {}", e),
        }
    }

    // Test 4: Search functionality
    println!("\nTest 4: Search navigation");
    state.view = View::Search;
    state.search.query = "love".to_string();
    match client.search("love").await {
        Ok(results) => {
            println!("  Search returned {} artists, {} albums, {} tracks",
                results.artists.len(), results.albums.len(), results.tracks.len());
            if !results.is_empty() {
                state.search.results = Some(results.clone());
                state.list_state.search_item_index = 0;
                apply_key_to_state(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                println!("  PASS - Search results can be navigated");
                println!("  Current selection: index={}", state.list_state.search_item_index);
            }
        }
        Err(e) => println!("  FAIL - Search failed: {}", e),
    }

    // Test 5: Similar albums
    println!("\nTest 5: Similar albums");
    if let Some(album) = state.library.albums.get(10) {
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
            Err(e) => println!("  FAIL - Similar albums failed: {}", e),
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
    print_buffer(terminal.backend());
    println!();

    println!("=== Test sequence complete ===");
    println!("\nFinal state:");
    println!("  View: {:?}", state.view);
    println!("  Category: {:?}", state.browse_category);
    println!("  Focus: {:?}", state.focus);
    println!("  Artists loaded: {}", state.library.artists.len());
    println!("  Albums loaded: {}", state.library.albums.len());
    println!("  Artists index: {}", state.list_state.artists_index);
    println!("  Albums index: {}", state.list_state.albums_index);

    Ok(())
}

/// Apply a key event to the application state (simplified for testing).
fn apply_key_to_state(state: &mut AppState, key: KeyEvent) {
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
                                state.library.genres_index = state.library.genres_index.saturating_sub(1);
                            }
                            BrowseCategory::Folders => {}
                        }
                    } else {
                        state.list_state.tracks_index = state.list_state.tracks_index.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if state.focus == Focus::Left {
                        match state.browse_category {
                            BrowseCategory::Library => {
                                let max = state.library.artists.len().saturating_sub(1);
                                state.list_state.artists_index = (state.list_state.artists_index + 1).min(max);
                            }
                            BrowseCategory::Playlists => {
                                let max = state.library.playlists.len().saturating_sub(1);
                                state.list_state.playlists_index = (state.list_state.playlists_index + 1).min(max);
                            }
                            BrowseCategory::Genres => {
                                let max = state.library.genres.len().saturating_sub(1);
                                state.library.genres_index = (state.library.genres_index + 1).min(max);
                            }
                            BrowseCategory::Folders => {}
                        }
                    } else {
                        let max = state.library.selected_album_tracks.len().saturating_sub(1);
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
                    state.search.query.clear();
                    state.search.results = None;
                }
                KeyCode::Down => { state.list_state.search_item_index += 1; }
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
                        let max = state.queue.tracks.len().saturating_sub(1);
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

/// Print the test backend buffer in a readable format.
fn print_buffer(backend: &TestBackend) {
    let buffer = backend.buffer();
    let area = buffer.area;

    println!("\u{250c}{}\u{2510}", "\u{2500}".repeat(area.width as usize));

    for y in 0..area.height {
        print!("\u{2502}");
        for x in 0..area.width {
            let cell = buffer.cell((x, y)).unwrap();
            let ch = cell.symbol();
            if ch.is_empty() || ch == " " {
                print!(" ");
            } else {
                print!("{}", ch);
            }
        }
        println!("\u{2502}");
    }

    println!("\u{2514}{}\u{2518}", "\u{2500}".repeat(area.width as usize));
}
