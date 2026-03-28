//! textamp debug mode - test API without TUI.

use anyhow::Result;
use textamp::plex::{PlexAuth, PlexClient, PlexClientInfo};
use textamp::config;

#[tokio::main]
async fn main() -> Result<()> {
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

                        let test_album = albums.iter()
                            .skip(5)
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

    music_libs.sort_by(|a, b| a.key.parse::<u32>().unwrap_or(999).cmp(&b.key.parse::<u32>().unwrap_or(999)));

    if let Some(lib) = music_libs.first() {
        println!("\nTesting stations for library '{}' (key={})...", lib.title, lib.key);

        let path = format!("/hubs/sections/{}?includeStations=1", lib.key);
        println!("\nRaw API response for {}:", path);
        match client.get_raw(&path).await {
            Ok(raw) => {
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
