//! Test script for radio stations with authentication.
//!
//! Run with: cargo run --example test_stations_auth --release

use textamp::api::{PlexAuth, PlexClient};

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let server_url = "http://127.0.0.1:32400";
    let username = "featherstonehaugh";
    let password = "4cb-cFb-cRJ-qmA";
    let library_key = "5";

    println!("\n=== Authenticating with Plex ===\n");

    // Authenticate to get token
    let auth = PlexAuth::new();
    let token = match auth.authenticate_password(username, password).await {
        Ok(t) => {
            println!("Authentication successful!");
            t
        }
        Err(e) => {
            eprintln!("Authentication failed: {}", e);
            std::process::exit(1);
        }
    };

    println!("\n=== Testing Radio Stations ===\n");
    println!("Server: {}", server_url);
    println!("Library: {}", library_key);
    println!();

    let client = PlexClient::new_with_url(server_url, Some(&token), "textamp-test-example");

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

    // Test Decade Radio (category drill-in)
    println!("Testing: Decade Radio categories");
    let decade_path = format!("/library/sections/{}/decade", library_key);
    match client.get_station_children(&decade_path).await {
        Ok(decades) => {
            if decades.is_empty() {
                println!("  ❌ FAILED: No decades found");
            } else {
                println!("  ✓ Found {} decades:", decades.len());
                for decade in &decades {
                    println!("    - {} (key: {})", decade.title, decade.key);
                }
                // Test first decade
                if let Some(first_decade) = decades.first() {
                    println!("\n  Testing first decade: {}", first_decade.title);
                    println!("    Key: {}", first_decade.key);
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
    println!();

    // Test Style Radio (category drill-in)
    println!("Testing: Style Radio categories");
    let style_path = format!("/library/sections/{}/style", library_key);
    match client.get_station_children(&style_path).await {
        Ok(styles) => {
            if styles.is_empty() {
                println!("  ❌ FAILED: No styles found");
            } else {
                println!("  ✓ Found {} styles:", styles.len());
                for style in styles.iter().take(5) {
                    println!("    - {} (key: {})", style.title, style.key);
                }
                if styles.len() > 5 {
                    println!("    ... and {} more", styles.len() - 5);
                }
                // Test first style
                if let Some(first_style) = styles.first() {
                    println!("\n  Testing first style: {}", first_style.title);
                    println!("    Key: {}", first_style.key);
                    match client.create_station_queue(&first_style.key).await {
                        Ok(tracks) => {
                            if tracks.is_empty() {
                                println!("    ❌ FAILED: No tracks for style");
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

    // Test Mood Radio (category drill-in)
    println!("Testing: Mood Radio categories");
    let mood_path = format!("/library/sections/{}/mood", library_key);
    match client.get_station_children(&mood_path).await {
        Ok(moods) => {
            if moods.is_empty() {
                println!("  ❌ FAILED: No moods found");
            } else {
                println!("  ✓ Found {} moods:", moods.len());
                for mood in moods.iter().take(5) {
                    println!("    - {} (key: {})", mood.title, mood.key);
                }
                if moods.len() > 5 {
                    println!("    ... and {} more", moods.len() - 5);
                }
                // Test first mood
                if let Some(first_mood) = moods.first() {
                    println!("\n  Testing first mood: {}", first_mood.title);
                    println!("    Key: {}", first_mood.key);
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

    println!("\n=== Tests Complete ===\n");
}
