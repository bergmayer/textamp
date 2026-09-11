//! Read-only live compatibility/cache check. --connect explicitly saves local settings.
//! Never starts analysis, plays audio, writes tags, or creates server playlists.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::HashSet, io::BufRead, time::Duration};
use textamp::audiomuse::{Client, Connection, Feature, Snapshot};

#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(900), run())
        .await
        .context("AudioMuse check timed out")?
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut config = textamp::config::load_config()?;
    anyhow::ensure!(
        config.navidrome_sources.len() == 1,
        "Select exactly one Navidrome account for this bounded check"
    );
    let source = config.navidrome_sources[0].clone();
    let folder = source.canonical_folder(
        config
            .default_navidrome
            .as_ref()
            .filter(|s| s.source_id == source.id)
            .and_then(|s| s.folder.clone()),
    );
    let session = textamp::app::sources::navidrome::Session {
        client: textamp::navidrome::Client::new(
            &source,
            "unused-for-cache-identity".into(),
            folder,
        )?,
        source,
        extensions: Default::default(),
    };
    let key = textamp::app::sources::audiomuse::key(&session);
    if args.iter().any(|arg| arg == "--sidebar") {
        let mut state = textamp::app::AppState::new();
        textamp::app::sources::manager::initialize(&mut state, &config);
        state.sources.active = textamp::app::sources::ActiveSource::Navidrome(Box::new(session));
        state.view = textamp::app::state::View::Browse;
        state.hidden_sections = config.ui.hidden_sections.clone();
        state.hidden_collections = config.ui.hidden_collections.clone();
        println!(
            "Sonic enabled: {}",
            textamp::app::sources::sonic::enabled(&state)
        );
        println!(
            "AudioMuse configured: {}",
            textamp::app::sources::audiomuse::connection(&state).is_some()
        );
        for feature in Feature::ALL {
            let row = textamp::app::state::CategoryRow::NavidromeCollection(
                textamp::app::sources::navidrome::commands::CollectionKind::AudioMuse(feature),
            );
            println!(
                "{} in sidebar: {}",
                feature.label(),
                state.category_rows().contains(&row)
            );
        }
        return Ok(());
    }
    let connect = args.first().is_some_and(|s| s == "--connect");
    let supplied = connect || args.first().is_some_and(|s| s == "--check");
    let connection = if supplied {
        anyhow::ensure!(
            args.len() >= 4,
            "--connect URL USERNAME MUSIC_SERVER_NAME [--sync]; password on stdin"
        );
        Connection {
            url: args[1].clone(),
            username: args[2].clone(),
            server: args[3].clone(),
        }
    } else {
        config
            .audiomuse_connections
            .get(&key)
            .context("No saved AudioMuse connection")?
            .clone()
    };
    let password: textamp::util::SecretString = if supplied {
        let mut line = zeroize::Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).into()
    } else {
        textamp::library::credentials::load_source("audiomuse", &key)?
            .context("Missing AudioMuse credentials")?
            .as_str()
            .into()
    };
    let api = Client::connect(&connection, &password).await?;
    api.verify().await?;
    println!("PASS: authentication, configured Navidrome binding and analysis export");
    for feature in [Feature::Describe, Feature::Lyrics] {
        let ready = api.readiness(feature).await?;
        println!(
            "{} enabled={} analyzed={}",
            feature.label(),
            ready.enabled,
            ready.songs
        );
    }
    if args.iter().any(|s| s == "--sonic") {
        let moods = api.moods().await?;
        let mood = moods
            .first()
            .context("No AudioMuse mood centroids available yet")?;
        let ids = api.mood(mood).await?;
        let seed = ids.first().context("No analyzed mood matches yet")?;
        let similar = api.similar(seed).await?;
        let end = similar
            .iter()
            .find(|id| *id != seed)
            .context("No analyzed sonic neighbors yet")?;
        let path = api.path(seed, end, 5).await?;
        anyhow::ensure!(
            path.first() == Some(seed) && path.last() == Some(end),
            "Invalid live sonic path endpoints"
        );
        println!("PASS: {} available moods; {} mood matches; {} sonic neighbors; {}-track path (read-only)", moods.len(), ids.len(), similar.len(), path.len());
    }
    if connect {
        textamp::library::credentials::save_source("audiomuse", &key, password)?;
        config.audiomuse_connections.insert(key, connection.clone());
        textamp::config::save_config(&config)?;
        println!(
            "PASS: saved local connection with private credentials; library selection unchanged"
        );
    }
    if !args.iter().any(|s| s == "--sync") {
        return Ok(());
    }
    // Deserialize only identity fields, not the entire large catalog into a JSON tree.
    #[derive(Deserialize)]
    struct Catalog {
        tracks: Vec<TrackId>,
    }
    #[derive(Deserialize)]
    struct TrackId {
        #[serde(rename = "ratingKey")]
        key: String,
    }
    let store = session.cache_store()?;
    let catalog = store
        .ticket("catalog")
        .read::<Catalog>(Duration::from_secs(86400))?
        .context("Open the Navidrome library to create its catalog first")?;
    let allowed: HashSet<_> = catalog
        .value
        .tracks
        .iter()
        .map(|t| session.id(&t.key))
        .collect::<Result<_>>()?;
    let ticket = store.ticket(&format!(
        "audiomuse:{}",
        serde_json::to_string(&connection)?
    ));
    let previous = ticket
        .read::<Snapshot>(Duration::from_secs(86400))?
        .map(|h| h.value)
        .unwrap_or_default();
    let start = std::time::Instant::now();
    println!(
        "Refreshing analysis: {} cached tracks, {} library tracks",
        previous.tracks.len(),
        allowed.len()
    );
    let snapshot = api
        .refresh_with_progress(&previous, &allowed, |progress| println!("{progress}"))
        .await?;
    println!(
        "PASS: {} analyzed / {} library tracks; {} server analyses; {:.1}s refresh",
        snapshot.tracks.len(),
        allowed.len(),
        snapshot.available,
        start.elapsed().as_secs_f32()
    );
    ticket.write(&snapshot)?;
    println!(
        "Index partial={} unmatched IDs={}",
        snapshot.partial, snapshot.unresolved
    );
    let hit = ticket
        .read::<Snapshot>(Duration::from_secs(86400))?
        .context("Cache reread missing")?;
    anyhow::ensure!(
        hit.value.tracks.len() == snapshot.tracks.len(),
        "Cache count mismatch"
    );
    let grouping = std::time::Instant::now();
    for feature in [
        Feature::Labels,
        Feature::Key,
        Feature::Tempo,
        Feature::Energy,
    ] {
        println!(
            "{} groups: {}",
            feature.label(),
            snapshot.groups(feature).len()
        );
    }
    println!(
        "PASS: persistent cache reread; grouping {:.3}s",
        grouping.elapsed().as_secs_f32()
    );
    Ok(())
}
