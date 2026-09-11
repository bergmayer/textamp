//! Read-only, bounded production-server check. --add explicitly saves an account;
//! otherwise uses saved credentials. Never starts scans or modifies server metadata.
use anyhow::{Context, Result};
use serde_json::Value;
use std::{collections::BTreeSet, io::BufRead, time::Duration};
use textamp::navidrome::{Client, Source};

#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(480), run())
        .await
        .context("Navidrome check timed out")?
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = textamp::config::load_config()?;
    let add = args.first().is_some_and(|s| s == "--add");
    let mut source = if add {
        anyhow::ensure!(args.len() >= 3, "Usage: --add URL USERNAME [--audio]");
        config
            .navidrome_sources
            .iter()
            .find(|s| s.url == args[1] && s.username == args[2])
            .cloned()
            .unwrap_or_else(|| Source {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{} @ {}", args[2], args[1]),
                url: args[1].clone(),
                username: args[2].clone(),
                libraries: vec![],
            })
    } else {
        let selection = config
            .default_navidrome
            .as_ref()
            .context("No default Navidrome account")?;
        config
            .navidrome_sources
            .iter()
            .find(|s| s.id == selection.source_id)
            .cloned()
            .context("Saved account missing")?
    };
    let password = if add {
        eprintln!("Password on stdin (disable terminal echo before running):");
        let mut line = zeroize::Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        textamp::util::SecretString::from(line.trim_end_matches(['\r', '\n']))
    } else {
        let secret = textamp::library::credentials::load_source("navidrome", &source.id)?
            .context("Saved password missing")?;
        textamp::util::SecretString::from(secret.as_str())
    };
    let client = Client::new(&source, password.clone(), None)?;
    let ping = client.call("ping", &[]).await?;
    println!(
        "PASS: authentication; server version {}",
        ping["serverVersion"]
    );
    source.libraries = client.folders().await?;
    println!("PASS: {} music libraries", source.libraries.len());
    if add {
        textamp::library::credentials::save_source("navidrome", &source.id, password)?;
        config.navidrome_sources.retain(|s| s.id != source.id);
        config.navidrome_sources.push(source.clone());
        config.default_navidrome = Some(textamp::navidrome::Selection {
            source_id: source.id.clone(),
            folder: None,
        });
        config.default_folder_source = None;
        textamp::config::save_config(&config)?;
        println!("PASS: account saved with private credentials; other sources preserved");
    }
    let extensions = client.call("getOpenSubsonicExtensions", &[]).await?;
    println!("Extensions: {}", extensions["openSubsonicExtensions"]);
    let response = client
        .call(
            "getAlbumList2",
            &[("type", "alphabeticalByName".into()), ("size", "50".into())],
        )
        .await?;
    let albums: Vec<textamp::navidrome::Album> =
        textamp::navidrome::array(&response["albumList2"], "album")?;
    println!("PASS: {} sampled albums", albums.len());
    let raw_albums = response["albumList2"]["album"]
        .as_array()
        .context("No album list")?;
    let multi = raw_albums
        .iter()
        .filter(|a| a["genres"].as_array().is_some_and(|g| g.len() > 1))
        .count();
    println!("Albums with multiple structured genres in sample: {multi}");
    for album in raw_albums
        .iter()
        .filter(|a| {
            a["genres"]
                .as_array()
                .is_some_and(|g| !g.is_empty() && !g.iter().any(|tag| tag["name"] == a["genre"]))
        })
        .take(3)
    {
        println!(
            "Legacy/structured disagreement: legacy={} structured={}",
            album["genre"], album["genres"]
        );
    }
    for album in raw_albums
        .iter()
        .filter(|a| a["genres"].as_array().is_some_and(|g| g.len() > 1))
        .take(3)
    {
        println!(
            "Genre shape: legacy={} structured={}",
            album["genre"], album["genres"]
        );
    }
    let genres_response = client.call("getGenres", &[]).await?;
    let genres: Vec<serde_json::Value> =
        textamp::navidrome::array(&genres_response["genres"], "genre")?;
    println!("PASS: {} server genres", genres.len());
    let folded: BTreeSet<_> = genres
        .iter()
        .filter_map(|g| g["value"].as_str())
        .map(str::to_lowercase)
        .collect();
    println!(
        "Genre names that differ only in case: {}",
        genres.len().saturating_sub(folded.len())
    );
    println!(
        "Genre sample: {}",
        serde_json::to_string(&genres.iter().take(16).collect::<Vec<_>>())?
    );
    let Some(album) = albums.first() else {
        anyhow::bail!("No music available")
    };
    let album = client.album(&album.id).await?;
    println!("PASS: album detail ({} tracks)", album.song.len());
    if let Some(id) = &album.artist_id {
        println!(
            "PASS: artist detail ({} albums)",
            client.artist(id).await?.album.len()
        );
    }
    println!("PASS: {} playlists", client.playlists().await?.len());
    let song = album
        .song
        .iter()
        .find(|s| s.duration.is_some_and(|d| d > 15))
        .or_else(|| album.song.first())
        .context("Empty album")?;
    let query = song.title.split_whitespace().next().unwrap_or(&song.title);
    let results = client
        .call(
            "search3",
            &[
                ("query", query.into()),
                ("songCount", "10".into()),
                ("albumCount", "0".into()),
                ("artistCount", "0".into()),
            ],
        )
        .await?;
    let songs: Vec<textamp::navidrome::Song> =
        textamp::navidrome::array(&results["searchResult3"], "song")?;
    println!("PASS: search ({} tracks)", songs.len());
    if args.iter().any(|s| s == "--sonic" || s == "--sonic-id") {
        let sonic_id = args
            .windows(2)
            .find(|pair| pair[0] == "--sonic-id")
            .map(|pair| pair[1].as_str())
            .unwrap_or(&song.id);
        client
            .song(sonic_id)
            .await
            .context("Sonic seed is not a Navidrome song")?;
        match client
            .call(
                "getSonicSimilarTracks",
                &[("id", sonic_id.into()), ("count", "3".into())],
            )
            .await
        {
            Ok(response) => {
                let matches: Vec<serde_json::Value> =
                    textamp::navidrome::array(&response, "sonicMatch")?;
                println!("Sonic probe: {} matches through Navidrome", matches.len());
                if let Some(end) = matches
                    .iter()
                    .filter_map(|m| m["entry"]["id"].as_str())
                    .find(|id| *id != sonic_id)
                {
                    let path = client
                        .call(
                            "findSonicPath",
                            &[
                                ("startSongId", sonic_id.into()),
                                ("endSongId", end.into()),
                                ("count", "5".into()),
                            ],
                        )
                        .await?;
                    let path: Vec<Value> = textamp::navidrome::array(&path, "sonicMatch")?;
                    anyhow::ensure!(
                        path.len() <= 5
                            && path.first().is_some_and(|p| p["entry"]["id"] == sonic_id)
                            && path.last().is_some_and(|p| p["entry"]["id"] == end),
                        "Invalid sonic path"
                    );
                    println!("PASS: sonic path ({} tracks)", path.len());
                }
            }
            Err(error) => {
                println!("Sonic probe unavailable: {error:#}");
                // Inspect only fixed diagnostic categories. Never print an arbitrary
                // server error body, which can echo credentials or private URLs.
                let raw = client
                    .bytes(
                        client.url(
                            "getSonicSimilarTracks",
                            &[("id", sonic_id.into()), ("count", "3".into())],
                        )?,
                        1024 * 1024,
                    )
                    .await?;
                let raw: Value = serde_json::from_slice(&raw)?;
                let message = raw["subsonic-response"]["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase();
                let categories: Vec<_> = [
                    "not found",
                    "not analyzed",
                    "index",
                    "unauthorized",
                    "forbidden",
                    "connection",
                    "timeout",
                    "plugin",
                    "disabled",
                    "empty",
                    "internal",
                    "failed",
                ]
                .into_iter()
                .filter(|word| message.contains(word))
                .collect();
                println!("Server diagnostic categories: {categories:?}");
            }
        }
    }
    let sample_genres: BTreeSet<_> = albums
        .iter()
        .filter_map(|a| a.genre.as_deref())
        .take(3)
        .collect();
    for genre in sample_genres {
        let result = client
            .call(
                "getAlbumList2",
                &[
                    ("type", "byGenre".into()),
                    ("genre", genre.into()),
                    ("size", "5".into()),
                ],
            )
            .await?;
        let matches: Vec<textamp::navidrome::Album> =
            textamp::navidrome::array(&result["albumList2"], "album")?;
        println!(
            "PASS: genre {} ({} sampled albums)",
            textamp::util::sanitize_display_text(genre),
            matches.len()
        );
    }
    if let Some(cover) = &album.cover_art {
        let bytes = client
            .bytes(
                client.url(
                    "getCoverArt",
                    &[("id", cover.clone()), ("size", "200".into())],
                )?,
                8 * 1024 * 1024,
            )
            .await?;
        let image = image::load_from_memory(&bytes)?;
        println!(
            "PASS: artwork decoded ({} × {})",
            image.width(),
            image.height()
        );
    }
    if args.iter().any(|s| s == "--audio") {
        let mut player = textamp::audio::AudioPlayer::new()?;
        player.set_volume(0.0);
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        player.play_url_with_headers(
            client
                .url(
                    "stream",
                    &[("id", song.id.clone()), ("format", "raw".into())],
                )?
                .as_str(),
            Default::default(),
            None,
            tx,
            client.http(),
        )?;
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        tokio::time::timeout(Duration::from_secs(30), async {
            while player.position().is_none_or(|p| p < Duration::from_secs(1)) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("Streaming did not advance")?;
        player.pause();
        player.seek(Duration::from_secs(5))?;
        tokio::time::timeout(Duration::from_secs(15), async {
            while player.position().is_none_or(|p| p < Duration::from_secs(5))
                || !player.is_paused()
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("Paused seek did not complete")?;
        player.stop();
        drain.abort();
        println!(
            "PASS: muted native streaming, pause, seek and stop; no play-history reports sent"
        );
    }
    if args.iter().any(|s| s == "--catalog") {
        check_catalog(source, client, config, &genres).await?;
    }
    Ok(())
}

async fn check_catalog(
    source: Source,
    client: Client,
    mut config: textamp::config::Config,
    server_genres: &[serde_json::Value],
) -> Result<()> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use textamp::app::{
        action::*,
        dispatch::{dispatch_action, handle_core_event},
        sources::{navidrome::Session, ActiveSource},
        state::{BrowseCategory, BrowseItem, RefreshCategory, View},
        AppState,
    };
    let mut state = AppState::new();
    state.active_library = Some(format!("navidrome:{}:all", source.id));
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        source,
        client,
        extensions: Default::default(),
    }));
    state.view = View::Browse;
    state.artwork.default_visible = false;

    let mut audio = textamp::audio::AudioPlayer::new_without_audio();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    println!("Loading full catalog through the application (bounded metadata requests only)…");
    let started = std::time::Instant::now();
    dispatch_action(
        SystemAction::RefreshCategory(RefreshCategory::Artists).into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await?;
    while state.library_loading {
        let event = tokio::time::timeout(Duration::from_secs(310), rx.recv())
            .await?
            .context("Catalog event channel closed")?;
        for action in handle_core_event(event, &mut state, &tx) {
            dispatch_action(action, &mut state, &mut audio, &mut config, &tx).await?;
        }
    }
    anyhow::ensure!(
        state.notifications.last_error.is_none(),
        "Catalog failed: {:?}",
        state.notifications.last_error
    );
    println!(
        "PASS: full application catalog: {} artists, {} albums, {} tracks, {} genres in {:.1}s",
        state.library.artists.len(),
        state.library.albums.len(),
        state.library.all_tracks.len(),
        state.library.album_genres.len(),
        started.elapsed().as_secs_f64()
    );
    let present: BTreeSet<_> = state
        .library
        .album_genres
        .iter()
        .map(|g| g.title.as_str())
        .collect();
    let missing: Vec<_> = server_genres
        .iter()
        .filter_map(|g| g["value"].as_str())
        .filter(|g| !present.contains(g))
        .collect();
    println!(
        "Server genres not represented by album tags: {}",
        missing.len()
    );
    let server_names: BTreeSet<_> = server_genres
        .iter()
        .filter_map(|g| g["value"].as_str())
        .collect();
    let extra: Vec<_> = present
        .iter()
        .copied()
        .filter(|g| !server_names.contains(g))
        .collect();
    println!(
        "Album tag names absent from the server genre index: {} (sample {:?})",
        extra.len(),
        extra.iter().take(12).collect::<Vec<_>>()
    );
    println!(
        "Case-insensitive album genre count: {}",
        present
            .iter()
            .map(|g| g.to_lowercase())
            .collect::<BTreeSet<_>>()
            .len()
    );
    println!(
        "Missing genre sample: {:?}",
        missing.iter().take(10).collect::<Vec<_>>()
    );
    let multi_album = state
        .library
        .albums
        .iter()
        .find(|a| a.genre.len() > 1)
        .context("No multi-genre album")?;
    let album_key = multi_album.rating_key.clone();
    let secondary_genre = multi_album.genre[1].tag.clone();
    for action in textamp::app::handlers::key_input::handle_key(
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
        &mut state,
        &config,
    ) {
        dispatch_action(action, &mut state, &mut audio, &mut config, &tx).await?;
    }
    anyhow::ensure!(
        state.browse_category == BrowseCategory::AlbumGenres,
        "Ctrl+G did not open genres"
    );
    let column = state
        .tag_nav
        .columns
        .first_mut()
        .context("No genre column")?;
    column.selected_index = column
        .items
        .iter()
        .position(|i| matches!(i,BrowseItem::Genre{title,..} if title==&secondary_genre))
        .context("Secondary genre missing from UI")?;
    dispatch_action(
        BrowseAction::LoadTagAlbums {
            replace_child: false,
        }
        .into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await?;
    anyhow::ensure!(
        state
            .tag_nav
            .columns
            .last()
            .is_some_and(|c| c.items.iter().any(|i| i.key() == album_key)),
        "Album missing from secondary genre"
    );
    dispatch_action(
        MillerAction::LoadGenreTracksForMiller {
            album_key,
            replace_child: false,
        }
        .into(),
        &mut state,
        &mut audio,
        &mut config,
        &tx,
    )
    .await?;
    anyhow::ensure!(
        state
            .tag_nav
            .columns
            .last()
            .is_some_and(|c| !c.tracks.is_empty()),
        "No genre album tracks"
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40))?;
    terminal.draw(|frame| {
        textamp::ui::render(frame, &state);
    })?;
    println!(
        "PASS: real Ctrl+G, secondary genre → album → {} ordered tracks, and headless rendering",
        state.tag_nav.columns.last().unwrap().tracks.len()
    );
    Ok(())
}
