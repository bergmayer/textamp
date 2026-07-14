//! Test script for radio stations.
//!
//! Set `PLEX_TOKEN`, then run with:
//! `cargo run --example test_stations -- <server_url> <library_key>`

use textamp::plex::PlexClient;
use zeroize::Zeroizing;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: PLEX_TOKEN=... {} <server_url> <library_key>", args[0]);
        std::process::exit(1);
    }

    let server_url = &args[1];
    let token = Zeroizing::new(match std::env::var("PLEX_TOKEN") {
        Ok(token) if !token.is_empty() => token,
        _ => {
            eprintln!("PLEX_TOKEN must be set (tokens are not accepted on the command line)");
            std::process::exit(1);
        }
    });
    let library_key = &args[2];

    println!("\n=== Testing Radio Stations ===\n");
    println!("Server: {}", server_url);
    println!("Library: {}", library_key);
    println!();

    let mut client = match PlexClient::new_with_url(
        server_url,
        Some(token.as_str()),
        "textamp-test-example",
    ) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Could not initialize Plex client: {error}");
            std::process::exit(1);
        }
    };

    // Test each station type
    let station_types = [
        ("library", "Library Radio"),
        ("deepCuts", "Deep Cuts Radio"),
        ("timeTravel", "Time Travel Radio"),
        ("randomAlbum", "Random Album Radio"),
    ];

    for (station_type, name) in station_types {
        let station_key = format!("/library/sections/{}/stations/{}", library_key, station_type);
        println!("Testing: {} ({})", name, station_key);

        match client.create_station_queue(&station_key).await {
            Ok(tracks) => {
                if tracks.is_empty() {
                    println!("  ❌ FAILED: No tracks returned");
                } else {
                    println!("  ✓ SUCCESS: {} tracks", tracks.len());
                    if let Some(first) = tracks.first() {
                        println!("    First track: {} - {}",
                            first.grandparent_title.as_deref().unwrap_or("Unknown"),
                            first.title);
                    }
                }
            }
            Err(e) => {
                println!("  ❌ ERROR: {}", e);
            }
        }
        println!();
    }

    // Test Mood Radio (category drill-in)
    println!("Testing: Mood Radio categories");
    let mood_path = format!("/library/sections/{}/mood", library_key);
    match client.get_station_children(&mood_path).await {
        Ok(moods) => {
            if moods.is_empty() {
                println!("  ❌ FAILED: No moods found");
            } else {
                println!("  ✓ Found {} moods", moods.len());
                // Test first mood
                if let Some(first_mood) = moods.first() {
                    println!("  Testing first mood: {}", first_mood.title);
                    match client.create_station_queue(&first_mood.key).await {
                        Ok(tracks) => {
                            if tracks.is_empty() {
                                println!("    ❌ FAILED: No tracks for mood");
                            } else {
                                println!("    ✓ SUCCESS: {} tracks", tracks.len());
                            }
                        }
                        Err(e) => println!("    ❌ ERROR: {}", e),
                    }
                }
            }
        }
        Err(e) => println!("  ❌ ERROR: {}", e),
    }
    println!();

    // Test Decade Radio (category drill-in)
    println!("Testing: Decade Radio categories");
    let decade_path = format!("/library/sections/{}/decade", library_key);
    match client.get_station_children(&decade_path).await {
        Ok(decades) => {
            if decades.is_empty() {
                println!("  ❌ FAILED: No decades found");
            } else {
                println!("  ✓ Found {} decades", decades.len());
                for decade in &decades {
                    println!("    - {} (key: {})", decade.title, decade.key);
                }
                // Test first decade
                if let Some(first_decade) = decades.first() {
                    println!("  Testing first decade: {}", first_decade.title);
                    match client.create_station_queue(&first_decade.key).await {
                        Ok(tracks) => {
                            if tracks.is_empty() {
                                println!("    ❌ FAILED: No tracks for decade");
                            } else {
                                println!("    ✓ SUCCESS: {} tracks", tracks.len());
                            }
                        }
                        Err(e) => println!("    ❌ ERROR: {}", e),
                    }
                }
            }
        }
        Err(e) => println!("  ❌ ERROR: {}", e),
    }

    println!("\n=== Tests Complete ===\n");
}
